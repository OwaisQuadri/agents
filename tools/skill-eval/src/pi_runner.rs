use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::model::{
    ArtifactDefinition, ArtifactKind, CandidateArtifact, CaseDefinition, CaseDrive,
    HarnessIdentity, ModelIdentity, RunId, SkillEvalError, Timestamp, TrialKey, TrialUsage,
};
use crate::ports::CandidateRunner;

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const ANTHROPIC_AUTH_REVISION: &str = "c6605e2db9ad3e783c3fe8b23d269848e0981d26";
const MAX_PI_EVENT_BYTES: usize = 64 * 1024 * 1024;
const EFFECTIVE_IDENTITY_EVENT: &str = "skill-eval-effective-identity";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessRequest {
    pub(crate) program: String,
    pub(crate) arguments: Vec<String>,
    pub(crate) working_directory: PathBuf,
    pub(crate) timeout: Option<Duration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessOutput {
    pub(crate) exit_code: Option<i32>,
    pub(crate) standard_output: Vec<u8>,
    pub(crate) standard_error: Vec<u8>,
    pub(crate) is_timed_out: bool,
}

pub(crate) trait Process {
    fn run(&mut self, request: &ProcessRequest) -> io::Result<ProcessOutput>;
}

pub(crate) struct SystemProcess;

impl Process for SystemProcess {
    fn run(&mut self, request: &ProcessRequest) -> io::Result<ProcessOutput> {
        let mut command = Command::new(&request.program);
        command
            .args(&request.arguments)
            .current_dir(&request.working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = command.spawn()?;
        let process_group = child.id();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Pi stdout was not captured"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("Pi stderr was not captured"))?;
        let stdout_reader = thread::spawn(move || read_all(stdout));
        let stderr_reader = thread::spawn(move || read_all(stderr));
        let started_at = Instant::now();
        let mut descendants = BTreeSet::new();
        let wait_result = (|| -> io::Result<(Option<i32>, bool)> {
            loop {
                track_descendants(process_group, &mut descendants);
                if let Some(status) = child.try_wait()? {
                    break Ok((status.code(), false));
                }
                if request
                    .timeout
                    .is_some_and(|timeout| started_at.elapsed() >= timeout)
                {
                    child.kill()?;
                    let status = child.wait()?;
                    break Ok((status.code(), true));
                }
                thread::sleep(PROCESS_POLL_INTERVAL);
            }
        })();
        stop_process_group(process_group, &descendants);
        let (exit_code, is_timed_out) = wait_result?;

        Ok(ProcessOutput {
            exit_code,
            standard_output: join_reader(stdout_reader)?,
            standard_error: join_reader(stderr_reader)?,
            is_timed_out,
        })
    }
}

fn track_descendants(parent: u32, descendants: &mut BTreeSet<u32>) {
    let Ok(output) = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid="])
        .output()
    else {
        return;
    };
    let processes = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        })
        .collect::<Vec<(u32, u32)>>();
    loop {
        let before = descendants.len();
        for (process, process_parent) in &processes {
            if *process_parent == parent || descendants.contains(process_parent) {
                descendants.insert(*process);
            }
        }
        if descendants.len() == before {
            break;
        }
    }
}

fn stop_process_group(process_group: u32, descendants: &BTreeSet<u32>) {
    let group = format!("-{process_group}");
    for signal in ["-TERM", "-KILL"] {
        for process in descendants.iter().rev() {
            let _ = Command::new("/bin/kill")
                .args([signal, &process.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = Command::new("/bin/kill")
            .args([signal, group.as_str()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        thread::sleep(Duration::from_millis(10));
    }
}

fn read_all(reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_PI_EVENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PI_EVENT_BYTES {
        return Err(io::Error::other("Pi event output exceeded the size limit"));
    }
    Ok(bytes)
}

fn join_reader(reader: thread::JoinHandle<io::Result<Vec<u8>>>) -> io::Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| io::Error::other("Pi output reader panicked"))?
}

pub(crate) struct PiCandidateRunner<P = SystemProcess> {
    output_root: PathBuf,
    process: P,
}

impl PiCandidateRunner<SystemProcess> {
    pub(crate) fn new(output_root: PathBuf) -> Self {
        Self {
            output_root,
            process: SystemProcess,
        }
    }
}

impl<P> PiCandidateRunner<P> {
    pub(crate) fn with_process(output_root: PathBuf, process: P) -> Self {
        Self {
            output_root,
            process,
        }
    }
}

impl<P: Process> CandidateRunner for PiCandidateRunner<P> {
    fn execute(
        &mut self,
        run_id: &RunId,
        key: &TrialKey,
        artifact: &ArtifactDefinition,
        case: &CaseDefinition,
        model: &ModelIdentity,
        harness: &HarnessIdentity,
        candidate_timeout_seconds: Option<u32>,
    ) -> Result<CandidateArtifact, SkillEvalError> {
        validate_component(&run_id.0, "run")?;
        validate_trial(key, artifact, case, model, harness)?;
        if candidate_timeout_seconds == Some(0) {
            return Err(invalid("candidate timeout must be greater than zero"));
        }
        let is_frontier_execution = run_id.0.starts_with("frontier-");
        if is_frontier_execution {
            validate_frontier_provider(model)?;
            if candidate_timeout_seconds.is_some() {
                return Err(invalid(
                    "frontier Pi candidates must not have a wall-clock timeout",
                ));
            }
        }
        let output_root = fs::canonicalize(&self.output_root)
            .map_err(|error| io_error(&self.output_root, error))?;
        let trial_directory = trial_directory(&output_root, run_id, key)?;
        create_clean_directory(&trial_directory)?;
        let working_directory = prepare_working_directory(&trial_directory, case)?;
        let transcript_path = trial_directory.join("transcript.jsonl");
        let response_path = trial_directory.join("response.txt");
        let all_tools_extension = trial_directory.join("all-tools.ts");
        write_bytes(
            &all_tools_extension,
            all_tools_extension_source().as_bytes(),
        )?;
        fs::set_permissions(&all_tools_extension, fs::Permissions::from_mode(0o600))
            .map_err(|error| io_error(&all_tools_extension, error))?;
        let arguments = pi_arguments(artifact, case, model, &all_tools_extension)?;
        let request = ProcessRequest {
            program: "pi".to_owned(),
            arguments,
            working_directory: working_directory.clone(),
            timeout: candidate_timeout_seconds
                .map(|seconds| Duration::from_secs(u64::from(seconds))),
        };
        let started_at = Instant::now();
        let output = self
            .process
            .run(&request)
            .map_err(|error| SkillEvalError::Process {
                program: request.program.clone(),
                exit_code: None,
                standard_error: error.to_string(),
            })?;
        let elapsed_milliseconds = milliseconds(started_at.elapsed());
        if output.is_timed_out {
            let timeout_seconds =
                candidate_timeout_seconds.ok_or_else(|| SkillEvalError::Process {
                    program: request.program.clone(),
                    exit_code: output.exit_code,
                    standard_error: "process reported a timeout for an unbounded request"
                        .to_owned(),
                })?;
            write_timeout_transcript(&transcript_path, &output.standard_output, timeout_seconds)?;
            return Err(SkillEvalError::Process {
                program: request.program,
                exit_code: output.exit_code,
                standard_error: format!(
                    "candidate exceeded its configured {timeout_seconds}-second deadline"
                ),
            });
        }
        write_bytes(&transcript_path, &output.standard_output)?;
        let parsed = match parse_events(
            &output.standard_output,
            model,
            elapsed_milliseconds,
            output.is_timed_out,
        ) {
            Ok(parsed) => parsed,
            Err(error) => {
                let raw_output = format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&output.standard_output),
                    String::from_utf8_lossy(&output.standard_error)
                );
                if is_quota_text(&raw_output) {
                    return Err(SkillEvalError::Quota {
                        model: model.clone(),
                        reset_at: quota_reset_at(&raw_output),
                    });
                }
                let standard_error = String::from_utf8_lossy(&output.standard_error);
                if output.exit_code != Some(0) || !standard_error.trim().is_empty() {
                    return Err(SkillEvalError::Process {
                        program: request.program,
                        exit_code: output.exit_code,
                        standard_error: standard_error.trim().to_owned(),
                    });
                }
                return Err(error);
            }
        };

        if is_frontier_execution && !parsed.is_thinking_observed {
            return Err(invalid(
                "frontier Pi response did not report its effective thinking level",
            ));
        }
        if is_frontier_execution && parsed.model != *model {
            return Err(invalid(format!(
                "frontier Pi effective identity {}/{}/{} differs from requested {}/{}/{}",
                parsed.model.provider,
                parsed.model.model,
                parsed.model.thinking,
                model.provider,
                model.model,
                model.thinking,
            )));
        }
        if let Some((quota_model, reset_at)) = quota_pause(&parsed, &output) {
            return Err(SkillEvalError::Quota {
                model: quota_model,
                reset_at,
            });
        }
        if output.exit_code != Some(0) || parsed.error_message.is_some() {
            return Err(SkillEvalError::Process {
                program: request.program,
                exit_code: output.exit_code,
                standard_error: process_error_text(&output, parsed.error_message.as_deref()),
            });
        }

        write_bytes(&response_path, parsed.response.as_bytes())?;
        let artifact_path = match case.execution.drive {
            CaseDrive::Fixture { .. } => working_directory,
            CaseDrive::Response | CaseDrive::ExistingHarness { .. } => response_path,
        };
        Ok(CandidateArtifact {
            key: key.clone(),
            model: parsed.model,
            harness: harness.clone(),
            artifact_path,
            transcript_path,
            usage: parsed.usage,
        })
    }
}

fn validate_trial(
    key: &TrialKey,
    artifact: &ArtifactDefinition,
    case: &CaseDefinition,
    model: &ModelIdentity,
    harness: &HarnessIdentity,
) -> Result<(), SkillEvalError> {
    validate_component(&key.artifact.0, "artifact")?;
    validate_component(&key.case.0, "case")?;
    if key.artifact != artifact.name || key.case != case.id {
        return Err(invalid("trial key does not match its artifact and case"));
    }
    if harness.artifact_revision != artifact.revision {
        return Err(invalid(
            "harness artifact revision does not match the loaded artifact revision",
        ));
    }
    if case.execution.timeout_seconds == 0 {
        return Err(invalid("Pi trial timeout must be greater than zero"));
    }
    if model.provider.is_empty()
        || model.model.is_empty()
        || model.provider.contains('/')
        || model.provider.contains('\0')
        || model.model.contains('\0')
    {
        return Err(invalid("candidate model identity is malformed"));
    }
    validate_safety(&case.input)?;
    validate_tools(case)?;
    validate_artifact_paths(artifact)?;
    Ok(())
}

fn validate_frontier_provider(model: &ModelIdentity) -> Result<(), SkillEvalError> {
    if !matches!(model.provider.as_str(), "anthropic" | "openai-codex") {
        return Err(invalid(format!(
            "frontier Pi execution requires an exact first-party provider, got {:?}",
            model.provider
        )));
    }
    Ok(())
}

fn validate_component(value: &str, name: &str) -> Result<(), SkillEvalError> {
    let is_valid = !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if !is_valid {
        return Err(invalid(format!(
            "trial {name} identifier {value:?} is not one safe path component"
        )));
    }
    Ok(())
}

fn validate_safety(input: &str) -> Result<(), SkillEvalError> {
    let normalized = input.to_ascii_lowercase();
    let is_vulnerability_exploitation = normalized.contains("exploit")
        && (normalized.contains("vulnerability") || normalized.contains("vulnerable"));
    let is_authorization_bypass =
        normalized.contains("authorization") && normalized.contains("bypass");
    let is_credential_access = normalized.contains("credential")
        && ["access", "steal", "extract", "exfiltrate"]
            .iter()
            .any(|action| normalized.contains(action));
    let is_live_system_attack = normalized.contains("attack")
        && (normalized.contains("live system")
            || normalized.contains("live-system")
            || normalized.contains("production system"));
    if is_vulnerability_exploitation
        || is_authorization_bypass
        || is_credential_access
        || is_live_system_attack
    {
        return Err(invalid(
            "trial requests vulnerability exploitation, authorization bypass, credential access, or a live-system attack",
        ));
    }
    Ok(())
}

fn validate_tools(case: &CaseDefinition) -> Result<(), SkillEvalError> {
    let mut tools = BTreeSet::new();
    for tool in &case.execution.allowed_tools {
        let normalized = tool.trim().to_ascii_lowercase();
        let is_valid = !normalized.is_empty()
            && normalized == *tool
            && normalized.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            });
        if !is_valid || !tools.insert(normalized) {
            return Err(invalid(format!(
                "tool allowlist entry {tool:?} is malformed, repeated, or can widen the declaration"
            )));
        }
    }
    let has_write_tool = tools.iter().any(|tool| {
        matches!(
            tool.as_str(),
            "write" | "edit" | "bash" | "shell" | "apply_patch"
        )
    });
    if has_write_tool && !matches!(case.execution.drive, CaseDrive::Fixture { .. }) {
        return Err(invalid(
            "write-capable tools require a disposable fixture drive",
        ));
    }
    Ok(())
}

fn validate_artifact_paths(artifact: &ArtifactDefinition) -> Result<(), SkillEvalError> {
    if !artifact.root.is_dir() {
        return Err(invalid(format!(
            "artifact root {} is not a directory",
            artifact.root.display()
        )));
    }
    if artifact.kind == ArtifactKind::Workflow {
        let workflow = one_file_with_suffix(&artifact.root, ".workflow.js")?;
        let source = fs::read_to_string(&workflow).map_err(|error| io_error(&workflow, error))?;
        if source.contains("setActiveTools") {
            return Err(invalid(
                "workflow can widen its tool declaration before execution",
            ));
        }
    }
    Ok(())
}

fn trial_directory(root: &Path, run_id: &RunId, key: &TrialKey) -> Result<PathBuf, SkillEvalError> {
    if root.as_os_str().is_empty() {
        return Err(invalid("Pi runner output root is empty"));
    }
    Ok(root
        .join(&run_id.0)
        .join(&key.artifact.0)
        .join(format!("{:?}", key.tier).to_ascii_lowercase())
        .join(key.route_index.to_string())
        .join(&key.case.0)
        .join(key.attempt.to_string()))
}

fn create_clean_directory(path: &Path) -> Result<(), SkillEvalError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(invalid(format!(
                    "trial output {} must not be a symbolic link",
                    path.display()
                )));
            }
            fs::remove_dir_all(path).map_err(|error| io_error(path, error))?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(path, error)),
    }
    fs::create_dir_all(path).map_err(|error| io_error(path, error))
}

fn prepare_working_directory(
    trial_directory: &Path,
    case: &CaseDefinition,
) -> Result<PathBuf, SkillEvalError> {
    match &case.execution.drive {
        CaseDrive::Fixture { source, .. } => {
            let destination = trial_directory.join("fixture");
            copy_fixture(source, &destination)?;
            Ok(destination)
        }
        CaseDrive::Response | CaseDrive::ExistingHarness { .. } => Ok(trial_directory.to_owned()),
    }
}

fn copy_fixture(source: &Path, destination: &Path) -> Result<(), SkillEvalError> {
    let metadata = fs::symlink_metadata(source).map_err(|error| io_error(source, error))?;
    if metadata.file_type().is_symlink() {
        return Err(invalid(format!(
            "fixture source {} must not be a symbolic link",
            source.display()
        )));
    }
    fs::create_dir_all(destination).map_err(|error| io_error(destination, error))?;
    if metadata.is_file() {
        let name = source
            .file_name()
            .ok_or_else(|| invalid("fixture file has no file name"))?;
        let target = destination.join(name);
        fs::copy(source, &target).map_err(|error| io_error(&target, error))?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(invalid(format!(
            "fixture source {} is not a file or directory",
            source.display()
        )));
    }
    copy_directory(source, destination)
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), SkillEvalError> {
    for entry in fs::read_dir(source).map_err(|error| io_error(source, error))? {
        let entry = entry.map_err(|error| io_error(source, error))?;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(invalid(format!(
                "fixture entry {} must not be a symbolic link",
                path.display()
            )));
        }
        if metadata.is_dir() {
            fs::create_dir(&target).map_err(|error| io_error(&target, error))?;
            copy_directory(&path, &target)?;
        } else if metadata.is_file() {
            fs::copy(&path, &target).map_err(|error| io_error(&target, error))?;
        } else {
            return Err(invalid(format!(
                "fixture entry {} is not a file or directory",
                path.display()
            )));
        }
    }
    Ok(())
}

fn pi_arguments(
    artifact: &ArtifactDefinition,
    case: &CaseDefinition,
    model: &ModelIdentity,
    all_tools_extension: &Path,
) -> Result<Vec<String>, SkillEvalError> {
    let mut arguments = vec![
        "--mode".to_owned(),
        "json".to_owned(),
        "--no-session".to_owned(),
        "--no-skills".to_owned(),
    ];
    match artifact.kind {
        ArtifactKind::Skill => {
            arguments.push("--skill".to_owned());
            arguments.push(existing_file(&artifact.root.join("SKILL.md"))?);
        }
        ArtifactKind::Agent => {
            arguments.push("--append-system-prompt".to_owned());
            arguments.push(path_text(&one_file_with_suffix(&artifact.root, ".md")?));
        }
        ArtifactKind::Workflow => {
            arguments.push("--skill".to_owned());
            arguments.push(existing_file(&artifact.root.join("SKILL.md"))?);
            arguments.push("--extension".to_owned());
            arguments.push(path_text(&one_file_with_suffix(
                &artifact.root,
                ".workflow.js",
            )?));
        }
    }
    arguments.push("--extension".to_owned());
    arguments.push(path_text(all_tools_extension));
    arguments.push("--model".to_owned());
    arguments.push(format!("{}/{}", model.provider, model.model));
    arguments.push("--thinking".to_owned());
    arguments.push(model.thinking.clone());
    arguments.extend([
        "--no-prompt-templates".to_owned(),
        "--no-themes".to_owned(),
        "--no-context-files".to_owned(),
        "--no-approve".to_owned(),
    ]);
    arguments.push(pi_positional_prompt(&case.input));
    Ok(arguments)
}

fn all_tools_extension_source() -> &'static str {
    r#"import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI): void {
  const enableAll = () => pi.setActiveTools(pi.getAllTools().map((tool) => tool.name));
  pi.on("session_start", enableAll);
  pi.on("before_agent_start", () => {
    enableAll();
    pi.sendMessage({
      customType: "skill-eval-effective-identity",
      content: "",
      display: false,
      details: { thinking: pi.getThinkingLevel() },
    }, { triggerTurn: false });
    pi.appendEntry("skill-eval-tool-inventory", {
      tools: pi.getActiveTools().slice().sort(),
    });
  });
  pi.on("turn_start", enableAll);
  pi.on("context", enableAll);
}
"#
}

fn one_file_with_suffix(root: &Path, suffix: &str) -> Result<PathBuf, SkillEvalError> {
    let mut matches = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| io_error(root, error))? {
        let entry = entry.map_err(|error| io_error(root, error))?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|error| io_error(&path, error))?
            .is_file()
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(suffix))
        {
            matches.push(path);
        }
    }
    if matches.len() != 1 {
        return Err(invalid(format!(
            "artifact root {} must contain exactly one {suffix} file",
            root.display()
        )));
    }
    Ok(matches.remove(0))
}

fn existing_file(path: &Path) -> Result<String, SkillEvalError> {
    if !path.is_file() {
        return Err(invalid(format!(
            "explicit artifact file {} does not exist",
            path.display()
        )));
    }
    Ok(path_text(path))
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn pi_positional_prompt(prompt: &str) -> String {
    format!(
        "Treat the following text as the complete user request, not as a command-line file or option:\n\n{prompt}"
    )
}

pub(crate) fn append_pi_auth_extension(
    arguments: &mut Vec<String>,
    model: &ModelIdentity,
) -> Result<(), SkillEvalError> {
    if model.provider != "anthropic" {
        return Ok(());
    }
    let home = env::var_os("HOME").ok_or_else(|| {
        invalid("HOME is required to load the Anthropic Pi authentication extension")
    })?;
    append_pi_auth_extension_from_home(arguments, model, Path::new(&home), ANTHROPIC_AUTH_REVISION)
}

fn append_pi_auth_extension_from_home(
    arguments: &mut Vec<String>,
    model: &ModelIdentity,
    home: &Path,
    expected_revision: &str,
) -> Result<(), SkillEvalError> {
    if model.provider != "anthropic" {
        return Ok(());
    }
    let configured = home.join(".pi/agent/extensions/pi-anthropic-auth/src/index.ts");
    let extension = fs::canonicalize(&configured).map_err(|error| io_error(&configured, error))?;
    if !extension.is_file() {
        return Err(invalid(format!(
            "Anthropic Pi authentication extension {} is not a file",
            configured.display()
        )));
    }
    let package_root = extension
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| invalid("Anthropic Pi authentication extension has no package root"))?;
    let flags = git_output(
        package_root,
        &[
            "ls-files",
            "-v",
            "--",
            "src",
            "package.json",
            "pnpm-lock.yaml",
        ],
    )?;
    if flags.lines().any(|line| !line.starts_with("H ")) {
        return Err(invalid(
            "Anthropic Pi authentication extension uses hidden index flags",
        ));
    }
    let revision = git_output(package_root, &["rev-parse", "HEAD"])?;
    if revision.trim() != expected_revision {
        return Err(invalid(format!(
            "Anthropic Pi authentication extension revision is {}, expected {expected_revision}",
            revision.trim()
        )));
    }
    let status = git_output(
        package_root,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            "src",
            "package.json",
            "pnpm-lock.yaml",
        ],
    )?;
    if !status.trim().is_empty() {
        return Err(invalid(
            "Anthropic Pi authentication extension source differs from its pinned revision",
        ));
    }
    arguments.push("--extension".to_owned());
    arguments.push(path_text(&extension));
    Ok(())
}

fn git_output(root: &Path, arguments: &[&str]) -> Result<String, SkillEvalError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .map_err(|error| SkillEvalError::Process {
            program: "git".to_owned(),
            exit_code: None,
            standard_error: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(SkillEvalError::Process {
            program: "git".to_owned(),
            exit_code: output.status.code(),
            standard_error: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    String::from_utf8(output.stdout).map_err(|error| {
        invalid(format!(
            "Anthropic Pi authentication extension git output is not UTF-8: {error}"
        ))
    })
}

struct ParsedTrial {
    response: String,
    model: ModelIdentity,
    is_thinking_observed: bool,
    usage: TrialUsage,
    error_message: Option<String>,
}

fn parse_events(
    bytes: &[u8],
    requested_model: &ModelIdentity,
    elapsed_milliseconds: u64,
    is_timed_out: bool,
) -> Result<ParsedTrial, SkillEvalError> {
    let text = std::str::from_utf8(bytes).map_err(|error| SkillEvalError::InvalidEvent {
        line: 0,
        message: format!("Pi event stream is not UTF-8: {error}"),
    })?;
    let mut final_message = None;
    let mut effective_thinking = None;
    let mut usage = TrialUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        turns: 0,
        tool_calls: 0,
        elapsed_milliseconds,
        cost_millionths_of_dollar: 0,
    };
    let lines = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        let event: Value = match serde_json::from_str(line) {
            Ok(event) => event,
            Err(_) if is_timed_out && index + 1 == lines.len() => break,
            Err(error) => {
                return Err(SkillEvalError::InvalidEvent {
                    line: (index + 1) as u64,
                    message: error.to_string(),
                });
            }
        };
        match event.get("type").and_then(Value::as_str) {
            Some("turn_end") => usage.turns = checked_increment(usage.turns, index)?,
            Some("message_end")
                if event.pointer("/message/role").and_then(Value::as_str) == Some("custom")
                    && event.pointer("/message/customType").and_then(Value::as_str)
                        == Some(EFFECTIVE_IDENTITY_EVENT) =>
            {
                let thinking = event
                    .pointer("/message/details/thinking")
                    .and_then(Value::as_str)
                    .filter(|thinking| !thinking.is_empty())
                    .ok_or_else(|| {
                        invalid_event(index, "effective identity event has no thinking level")
                    })?;
                if effective_thinking.replace(thinking.to_owned()).is_some() {
                    return Err(invalid_event(
                        index,
                        "effective identity event is duplicated",
                    ));
                }
            }
            Some("tool_execution_start") => {
                usage.tool_calls = checked_increment(usage.tool_calls, index)?;
            }
            Some("message_end")
                if event.pointer("/message/role").and_then(Value::as_str) == Some("assistant") =>
            {
                let message = event
                    .get("message")
                    .ok_or_else(|| invalid_event(index, "message_end has no message"))?;
                add_usage(&mut usage, message, index)?;
                final_message = Some(message.clone());
            }
            _ => {}
        }
    }
    let Some(message) = final_message else {
        if is_timed_out {
            return Ok(ParsedTrial {
                response: String::new(),
                model: requested_model.clone(),
                is_thinking_observed: false,
                usage,
                error_message: None,
            });
        }
        return Err(SkillEvalError::InvalidEvent {
            line: 0,
            message: "Pi event stream has no authoritative assistant message_end".to_owned(),
        });
    };
    let is_thinking_observed = effective_thinking.is_some();
    let model = completed_model(&message, requested_model, effective_thinking.as_deref())?;
    let response = final_text(&message, 0)?;
    let error_message = if message.get("stopReason").and_then(Value::as_str) == Some("error") {
        Some(
            message
                .get("errorMessage")
                .and_then(Value::as_str)
                .unwrap_or("Pi assistant stopped with an unspecified error")
                .to_owned(),
        )
    } else {
        None
    };
    Ok(ParsedTrial {
        response,
        model,
        is_thinking_observed,
        usage,
        error_message,
    })
}

fn checked_increment(value: u32, line_index: usize) -> Result<u32, SkillEvalError> {
    value
        .checked_add(1)
        .ok_or_else(|| invalid_event(line_index, "event count overflowed"))
}

fn add_usage(
    total: &mut TrialUsage,
    message: &Value,
    line_index: usize,
) -> Result<(), SkillEvalError> {
    let usage = message
        .get("usage")
        .ok_or_else(|| invalid_event(line_index, "assistant message_end has no usage"))?;
    total.input_tokens = add_token(total.input_tokens, usage, "input", line_index)?;
    total.output_tokens = add_token(total.output_tokens, usage, "output", line_index)?;
    total.cache_read_tokens = add_token(total.cache_read_tokens, usage, "cacheRead", line_index)?;
    total.cache_write_tokens =
        add_token(total.cache_write_tokens, usage, "cacheWrite", line_index)?;
    let cost = usage
        .pointer("/cost/total")
        .and_then(Value::as_f64)
        .ok_or_else(|| invalid_event(line_index, "assistant usage has no numeric total cost"))?;
    if !cost.is_finite() || cost < 0.0 || cost > u64::MAX as f64 / 1_000_000.0 {
        return Err(invalid_event(line_index, "assistant usage cost is invalid"));
    }
    let millionths = (cost * 1_000_000.0).round() as u64;
    total.cost_millionths_of_dollar = total
        .cost_millionths_of_dollar
        .checked_add(millionths)
        .ok_or_else(|| invalid_event(line_index, "assistant usage cost overflowed"))?;
    Ok(())
}

fn add_token(
    total: u64,
    usage: &Value,
    field: &str,
    line_index: usize,
) -> Result<u64, SkillEvalError> {
    let value = usage
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_event(line_index, &format!("assistant usage has no {field}")))?;
    total
        .checked_add(value)
        .ok_or_else(|| invalid_event(line_index, "assistant token usage overflowed"))
}

fn completed_model(
    message: &Value,
    requested: &ModelIdentity,
    effective_thinking: Option<&str>,
) -> Result<ModelIdentity, SkillEvalError> {
    let provider = message
        .get("provider")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_event(0, "final assistant message has no provider"))?;
    let model = message
        .get("responseModel")
        .and_then(Value::as_str)
        .or_else(|| message.get("model").and_then(Value::as_str))
        .ok_or_else(|| invalid_event(0, "final assistant message has no model"))?;
    Ok(ModelIdentity {
        tier: requested.tier,
        provider: provider.to_owned(),
        model: model.to_owned(),
        thinking: effective_thinking.unwrap_or(&requested.thinking).to_owned(),
    })
}

fn final_text(message: &Value, line_index: usize) -> Result<String, SkillEvalError> {
    let content = message
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_event(line_index, "final assistant message has no content array"))?;
    let mut response = String::new();
    for part in content {
        if part.get("type").and_then(Value::as_str) == Some("text") {
            let text = part
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid_event(line_index, "text content has no text"))?;
            response.push_str(text);
        }
    }
    Ok(response)
}

fn quota_pause(
    parsed: &ParsedTrial,
    output: &ProcessOutput,
) -> Option<(ModelIdentity, Option<Timestamp>)> {
    let stderr = String::from_utf8_lossy(&output.standard_error);
    let text = parsed
        .error_message
        .as_deref()
        .map(|message| format!("{message}\n{stderr}"))
        .unwrap_or_else(|| stderr.into_owned());
    if is_quota_text(&text) {
        Some((parsed.model.clone(), quota_reset_at(&text)))
    } else {
        None
    }
}

fn is_quota_text(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    [
        "usage limit",
        "usage_limit_reached",
        "usage_not_included",
        "insufficient_quota",
        "quota exceeded",
        "account's rate limit",
        "monthly usage limit",
        "out of budget",
        "available balance",
    ]
    .iter()
    .any(|phrase| normalized.contains(phrase))
}

fn quota_reset_at(text: &str) -> Option<Timestamp> {
    for marker in ["reset_at=", "reset_at: ", "resets_at=", "resets_at: "] {
        let Some((_, remainder)) = text.split_once(marker) else {
            continue;
        };
        let value = remainder
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(|character| matches!(character, '"' | ',' | ';'));
        if !value.is_empty() {
            return Some(Timestamp(value.to_owned()));
        }
    }
    None
}

fn process_error_text(output: &ProcessOutput, assistant_error: Option<&str>) -> String {
    let stderr = String::from_utf8_lossy(&output.standard_error);
    match (assistant_error, stderr.trim()) {
        (Some(message), "") => message.to_owned(),
        (Some(message), standard_error) => format!("{message}\n{standard_error}"),
        (None, standard_error) => standard_error.to_owned(),
    }
}

fn milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), SkillEvalError> {
    fs::write(path, bytes).map_err(|error| io_error(path, error))
}

fn write_timeout_transcript(
    path: &Path,
    bytes: &[u8],
    timeout_seconds: u32,
) -> Result<(), SkillEvalError> {
    let text = std::str::from_utf8(bytes).map_err(|error| SkillEvalError::InvalidEvent {
        line: 0,
        message: format!("timed-out Pi event stream is not UTF-8: {error}"),
    })?;
    let mut transcript = String::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if serde_json::from_str::<Value>(line).is_err() {
            break;
        }
        transcript.push_str(line);
        transcript.push('\n');
    }
    transcript.push_str(&format!(
        "{{\"type\":\"skill_eval_timeout\",\"timeout_seconds\":{timeout_seconds}}}\n"
    ));
    write_bytes(path, transcript.as_bytes())
}

fn invalid(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(message.into())
}

fn invalid_event(line_index: usize, message: &str) -> SkillEvalError {
    SkillEvalError::InvalidEvent {
        line: (line_index + 1) as u64,
        message: message.to_owned(),
    }
}

fn io_error(path: &Path, error: io::Error) -> SkillEvalError {
    SkillEvalError::Io {
        path: path.to_owned(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::model::{
        ArtifactName, CaseId, ExecutionDefinition, QualificationPolicy, QualificationPurpose,
        RunConfiguration, RunEvent, RunMode, Tier, TierDestination,
    };
    use crate::ports::RunStore;
    use crate::store::FileRunStore;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[derive(Default)]
    struct FakeProcess {
        outputs: VecDeque<ProcessOutput>,
        requests: Vec<ProcessRequest>,
    }

    impl FakeProcess {
        fn returning(stdout: &str) -> Self {
            Self {
                outputs: VecDeque::from([ProcessOutput {
                    exit_code: Some(0),
                    standard_output: stdout.as_bytes().to_vec(),
                    standard_error: Vec::new(),
                    is_timed_out: false,
                }]),
                requests: Vec::new(),
            }
        }
    }

    impl Process for FakeProcess {
        fn run(&mut self, request: &ProcessRequest) -> io::Result<ProcessOutput> {
            self.requests.push(request.clone());
            self.outputs
                .pop_front()
                .ok_or_else(|| io::Error::other("no fake process output"))
        }
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let number = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "skill-eval-pi-runner-{}-{name}-{number}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(path.join("runs")).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn run_id() -> RunId {
        RunId("run-1".to_owned())
    }

    fn frontier_run_id() -> RunId {
        RunId("frontier-run-1".to_owned())
    }

    fn key() -> TrialKey {
        TrialKey {
            artifact: ArtifactName("fixture-skill".to_owned()),
            tier: Tier::T2,
            route_index: 0,
            case: CaseId("ordinary".to_owned()),
            attempt: 1,
        }
    }

    fn model() -> ModelIdentity {
        ModelIdentity {
            tier: Tier::T2,
            provider: "openai".to_owned(),
            model: "candidate".to_owned(),
            thinking: "low".to_owned(),
        }
    }

    fn harness() -> HarnessIdentity {
        HarnessIdentity {
            runner_version: "1".to_owned(),
            pi_version: "2".to_owned(),
            artifact_revision: "abc".to_owned(),
            tool_policy_digest: "def".to_owned(),
        }
    }

    fn skill(root: &Path) -> ArtifactDefinition {
        fs::write(
            root.join("SKILL.md"),
            "---\nname: fixture-skill\ndescription: fixture\n---\n",
        )
        .unwrap();
        ArtifactDefinition {
            name: ArtifactName("fixture-skill".to_owned()),
            kind: ArtifactKind::Skill,
            root: root.to_owned(),
            revision: "abc".to_owned(),
            required_destinations: vec![TierDestination::SkillMinimum],
            current_tiers: Vec::new(),
            cases: Vec::new(),
        }
    }

    fn response_case(tools: &[&str]) -> CaseDefinition {
        CaseDefinition {
            id: CaseId("ordinary".to_owned()),
            input: "Answer the fixture prompt.".to_owned(),
            expect: "final answer".to_owned(),
            source: "fixture".to_owned(),
            is_holdout: false,
            support_files: Vec::new(),
            execution: ExecutionDefinition {
                drive: CaseDrive::Response,
                allowed_tools: tools.iter().map(|tool| (*tool).to_owned()).collect(),
                timeout_seconds: 30,
            },
        }
    }

    fn frontier_model(provider: &str) -> ModelIdentity {
        ModelIdentity {
            tier: Tier::T2,
            provider: provider.to_owned(),
            model: "frontier-model".to_owned(),
            thinking: "high".to_owned(),
        }
    }

    fn exact_output(fixture: &str, model: &ModelIdentity) -> String {
        format!(
            "{{\"type\":\"message_end\",\"message\":{{\"role\":\"custom\",\"customType\":\"{EFFECTIVE_IDENTITY_EVENT}\",\"content\":\"\",\"display\":false,\"details\":{{\"thinking\":\"{}\"}}}}}}\n{}",
            model.thinking,
            fixture
                .replace("routed-provider", &model.provider)
                .replace("actual-model", &model.model)
        )
    }

    #[test]
    fn frontier_first_party_identity_is_exact_and_unbounded() {
        for provider in ["anthropic", "openai-codex"] {
            let directory = TestDirectory::new(provider);
            let artifact_root = directory.path().join("artifact");
            fs::create_dir(&artifact_root).unwrap();
            let artifact = skill(&artifact_root);
            let model = frontier_model(provider);
            let output = exact_output(include_str!("../tests/fixtures/pi/success.jsonl"), &model);
            let mut runner = PiCandidateRunner::with_process(
                directory.path().join("runs"),
                FakeProcess::returning(&output),
            );

            let candidate = runner
                .execute(
                    &frontier_run_id(),
                    &key(),
                    &artifact,
                    &response_case(&[]),
                    &model,
                    &harness(),
                    None,
                )
                .unwrap();

            assert_eq!(candidate.model, model);
            assert_eq!(runner.process.requests.len(), 1);
            let request = &runner.process.requests[0];
            assert_eq!(request.timeout, None);
            let provider_model = format!("{provider}/frontier-model");
            assert!(
                request
                    .arguments
                    .windows(2)
                    .any(|pair| { pair[0] == "--model" && pair[1] == provider_model })
            );
            assert!(
                request
                    .arguments
                    .windows(2)
                    .any(|pair| pair == ["--thinking", "high"])
            );
        }
    }

    #[test]
    fn frontier_candidate_timeout_fails_before_launch() {
        let directory = TestDirectory::new("frontier-timeout");
        let artifact_root = directory.path().join("artifact");
        fs::create_dir(&artifact_root).unwrap();
        let artifact = skill(&artifact_root);
        let mut runner = PiCandidateRunner::with_process(
            directory.path().join("runs"),
            FakeProcess::returning(include_str!("../tests/fixtures/pi/success.jsonl")),
        );

        let error = runner
            .execute(
                &frontier_run_id(),
                &key(),
                &artifact,
                &response_case(&[]),
                &frontier_model("anthropic"),
                &harness(),
                Some(30),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            SkillEvalError::InvalidConfiguration(message)
                if message.contains("must not have a wall-clock timeout")
        ));
        assert!(runner.process.requests.is_empty());
    }

    #[test]
    fn frontier_non_first_party_provider_fails_before_launch() {
        for provider in ["openrouter", "openai", "extension"] {
            let directory = TestDirectory::new(provider);
            let artifact_root = directory.path().join("artifact");
            fs::create_dir(&artifact_root).unwrap();
            let artifact = skill(&artifact_root);
            let mut runner = PiCandidateRunner::with_process(
                directory.path().join("runs"),
                FakeProcess::returning(include_str!("../tests/fixtures/pi/success.jsonl")),
            );

            let error = runner
                .execute(
                    &frontier_run_id(),
                    &key(),
                    &artifact,
                    &response_case(&[]),
                    &frontier_model(provider),
                    &harness(),
                    None,
                )
                .unwrap_err();

            assert!(matches!(error, SkillEvalError::InvalidConfiguration(_)));
            assert!(runner.process.requests.is_empty());
        }
    }

    #[test]
    fn frontier_effective_identity_drift_returns_no_candidate() {
        let directory = TestDirectory::new("frontier-identity-drift");
        let artifact_root = directory.path().join("artifact");
        fs::create_dir(&artifact_root).unwrap();
        let artifact = skill(&artifact_root);
        let requested = frontier_model("openai-codex");
        let effective = ModelIdentity {
            provider: "anthropic".to_owned(),
            model: "other-model".to_owned(),
            ..requested.clone()
        };
        let output = exact_output(
            include_str!("../tests/fixtures/pi/success.jsonl"),
            &effective,
        );
        let mut runner = PiCandidateRunner::with_process(
            directory.path().join("runs"),
            FakeProcess::returning(&output),
        );

        let error = runner
            .execute(
                &frontier_run_id(),
                &key(),
                &artifact,
                &response_case(&[]),
                &requested,
                &harness(),
                None,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            SkillEvalError::InvalidConfiguration(message)
                if message.contains("effective identity")
        ));
        assert_eq!(runner.process.requests.len(), 1);
        let runs_root = fs::canonicalize(directory.path().join("runs")).unwrap();
        let response = trial_directory(&runs_root, &frontier_run_id(), &key())
            .unwrap()
            .join("response.txt");
        assert!(!response.exists());
    }

    #[test]
    fn frontier_missing_effective_thinking_returns_no_candidate() {
        let directory = TestDirectory::new("frontier-thinking-absent");
        let artifact_root = directory.path().join("artifact");
        fs::create_dir(&artifact_root).unwrap();
        let artifact = skill(&artifact_root);
        let requested = frontier_model("openai-codex");
        let output = include_str!("../tests/fixtures/pi/success.jsonl")
            .replace("routed-provider", &requested.provider)
            .replace("actual-model", &requested.model);
        let mut runner = PiCandidateRunner::with_process(
            directory.path().join("runs"),
            FakeProcess::returning(&output),
        );

        let error = runner
            .execute(
                &frontier_run_id(),
                &key(),
                &artifact,
                &response_case(&[]),
                &requested,
                &harness(),
                None,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            SkillEvalError::InvalidConfiguration(message)
                if message.contains("did not report its effective thinking level")
        ));
    }

    #[test]
    fn frontier_effective_thinking_drift_returns_no_candidate() {
        let directory = TestDirectory::new("frontier-thinking-drift");
        let artifact_root = directory.path().join("artifact");
        fs::create_dir(&artifact_root).unwrap();
        let artifact = skill(&artifact_root);
        let requested = frontier_model("openai-codex");
        let effective = ModelIdentity {
            thinking: "low".to_owned(),
            ..requested.clone()
        };
        let output = exact_output(
            include_str!("../tests/fixtures/pi/success.jsonl"),
            &effective,
        );
        let mut runner = PiCandidateRunner::with_process(
            directory.path().join("runs"),
            FakeProcess::returning(&output),
        );

        let error = runner
            .execute(
                &frontier_run_id(),
                &key(),
                &artifact,
                &response_case(&[]),
                &requested,
                &harness(),
                None,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            SkillEvalError::InvalidConfiguration(message)
                if message.contains("effective identity")
        ));
    }

    #[test]
    fn candidate_has_no_wall_clock_limit() {
        let directory = TestDirectory::new("candidate-timeout-process");
        let mut process = SystemProcess;
        let unbounded = process
            .run(&ProcessRequest {
                program: "/bin/sh".to_owned(),
                arguments: vec!["-c".to_owned(), "sleep 0.03; printf complete".to_owned()],
                working_directory: directory.path().to_owned(),
                timeout: None,
            })
            .unwrap();
        assert_eq!(unbounded.exit_code, Some(0));
        assert!(!unbounded.is_timed_out);
        assert_eq!(unbounded.standard_output, b"complete");

        let bounded = process
            .run(&ProcessRequest {
                program: "/bin/sh".to_owned(),
                arguments: vec!["-c".to_owned(), "sleep 1".to_owned()],
                working_directory: directory.path().to_owned(),
                timeout: Some(Duration::from_millis(30)),
            })
            .unwrap();
        assert!(bounded.is_timed_out);
    }

    #[test]
    fn unbounded_candidate_rejects_impossible_timeout_report() {
        let directory = TestDirectory::new("impossible-unbounded-timeout");
        let artifact_root = directory.path().join("artifact");
        fs::create_dir(&artifact_root).unwrap();
        let artifact = skill(&artifact_root);
        let process = FakeProcess {
            outputs: VecDeque::from([ProcessOutput {
                exit_code: None,
                standard_output: Vec::new(),
                standard_error: Vec::new(),
                is_timed_out: true,
            }]),
            requests: Vec::new(),
        };
        let mut runner = PiCandidateRunner::with_process(directory.path().join("runs"), process);
        let mut frontier_model = model();
        frontier_model.provider = "openai-codex".to_owned();

        let error = runner
            .execute(
                &run_id(),
                &key(),
                &artifact,
                &response_case(&[]),
                &frontier_model,
                &harness(),
                None,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            SkillEvalError::Process { standard_error, .. }
                if standard_error.contains("unbounded request")
        ));
        assert_eq!(runner.process.requests[0].timeout, None);
    }

    #[cfg(unix)]
    #[test]
    fn tc_51_canonicalizes_runner_paths_for_store_run_containment() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("tc-51-canonical-root");
        let artifact_root = directory.path().join("artifact");
        let runs_root = directory.path().join("runs");
        let runs_alias = directory.path().join("runs-alias");
        fs::create_dir(&artifact_root).unwrap();
        symlink(&runs_root, &runs_alias).unwrap();
        let canonical_runs_root = fs::canonicalize(&runs_root).unwrap();
        let artifact = skill(&artifact_root);
        let case = response_case(&[]);
        let first_run_id = RunId("safe-run-1".to_owned());
        let second_run_id = RunId("safe-run-2".to_owned());
        let timestamp = Timestamp("now".to_owned());
        let policy = QualificationPolicy {
            purpose: QualificationPurpose::Artifact,
            candidate_tiers: vec![Tier::T2],
            reference_tier: Tier::T4,
            judge_tier: Tier::T5,
            repeats_per_case: 1,
            minimum_score: 7,
            noninferiority_margin: 0.1,
            confidence_level: 0.95,
        };
        let mut store = FileRunStore::new(&runs_root).unwrap();
        for run_id in [&first_run_id, &second_run_id] {
            store
                .append(
                    run_id,
                    &RunEvent::RunStarted {
                        at: timestamp.clone(),
                        configuration: RunConfiguration {
                            run_id: run_id.clone(),
                            mode: RunMode::Execute,
                            artifacts: Vec::new(),
                            change: None,
                            policy: policy.clone(),
                            qualification_routes: Default::default(),
                            created_at: timestamp.clone(),
                        },
                    },
                )
                .unwrap();
        }
        let output = include_str!("../tests/fixtures/pi/success.jsonl");
        let mut runner =
            PiCandidateRunner::with_process(runs_alias, FakeProcess::returning(output));

        let candidate = runner
            .execute(
                &first_run_id,
                &key(),
                &artifact,
                &case,
                &model(),
                &harness(),
                Some(case.execution.timeout_seconds),
            )
            .unwrap();

        assert!(
            candidate
                .artifact_path
                .starts_with(canonical_runs_root.join(&first_run_id.0))
        );
        assert!(
            candidate
                .transcript_path
                .starts_with(canonical_runs_root.join(&first_run_id.0))
        );
        store
            .append(
                &first_run_id,
                &RunEvent::TrialStarted {
                    at: timestamp.clone(),
                    key: candidate.key.clone(),
                    models: vec![candidate.model.clone()],
                    harness: candidate.harness.clone(),
                },
            )
            .unwrap();
        store
            .append(
                &first_run_id,
                &RunEvent::CandidateExecuted {
                    at: timestamp.clone(),
                    candidate: candidate.clone(),
                },
            )
            .unwrap();

        let second_log = canonical_runs_root
            .join(&second_run_id.0)
            .join("events.jsonl");
        let before_cross_run = fs::read(&second_log).unwrap();
        assert!(
            store
                .append(
                    &second_run_id,
                    &RunEvent::CandidateExecuted {
                        at: timestamp,
                        candidate,
                    },
                )
                .is_err()
        );
        assert_eq!(fs::read(second_log).unwrap(), before_cross_run);
    }

    #[test]
    fn candidate_run_paths_are_distinct_contained_and_store_scoped() {
        let directory = TestDirectory::new("candidate-run-paths");
        let artifact_root = directory.path().join("artifact");
        let runs_root = directory.path().join("runs");
        fs::create_dir(&artifact_root).unwrap();
        let runs_root = fs::canonicalize(runs_root).unwrap();
        let artifact = skill(&artifact_root);
        let case = response_case(&[]);
        let first_run_id = RunId("safe-run-1".to_owned());
        let second_run_id = RunId("safe-run-2".to_owned());
        let output = include_str!("../tests/fixtures/pi/success.jsonl");
        let timestamp = Timestamp("now".to_owned());
        let policy = QualificationPolicy {
            purpose: QualificationPurpose::Artifact,
            candidate_tiers: vec![Tier::T2],
            reference_tier: Tier::T4,
            judge_tier: Tier::T5,
            repeats_per_case: 1,
            minimum_score: 7,
            noninferiority_margin: 0.1,
            confidence_level: 0.95,
        };
        let mut store = FileRunStore::new(&runs_root).unwrap();
        for run_id in [&first_run_id, &second_run_id] {
            store
                .append(
                    run_id,
                    &RunEvent::RunStarted {
                        at: timestamp.clone(),
                        configuration: RunConfiguration {
                            run_id: run_id.clone(),
                            mode: RunMode::Execute,
                            artifacts: Vec::new(),
                            change: None,
                            policy: policy.clone(),
                            qualification_routes: Default::default(),
                            created_at: timestamp.clone(),
                        },
                    },
                )
                .unwrap();
        }
        let mut first_runner =
            PiCandidateRunner::with_process(runs_root.clone(), FakeProcess::returning(output));
        let mut second_runner =
            PiCandidateRunner::with_process(runs_root.clone(), FakeProcess::returning(output));

        let first = first_runner
            .execute(
                &first_run_id,
                &key(),
                &artifact,
                &case,
                &model(),
                &harness(),
                Some(case.execution.timeout_seconds),
            )
            .unwrap();
        let second = second_runner
            .execute(
                &second_run_id,
                &key(),
                &artifact,
                &case,
                &model(),
                &harness(),
                Some(case.execution.timeout_seconds),
            )
            .unwrap();

        assert_ne!(first.artifact_path, second.artifact_path);
        assert_ne!(first.transcript_path, second.transcript_path);
        assert!(
            first
                .artifact_path
                .starts_with(runs_root.join(&first_run_id.0))
        );
        assert!(
            first
                .transcript_path
                .starts_with(runs_root.join(&first_run_id.0))
        );
        assert!(
            second
                .artifact_path
                .starts_with(runs_root.join(&second_run_id.0))
        );
        assert!(
            second
                .transcript_path
                .starts_with(runs_root.join(&second_run_id.0))
        );

        for (run_id, candidate) in [
            (&first_run_id, first.clone()),
            (&second_run_id, second.clone()),
        ] {
            store
                .append(
                    run_id,
                    &RunEvent::TrialStarted {
                        at: timestamp.clone(),
                        key: candidate.key.clone(),
                        models: vec![candidate.model.clone()],
                        harness: candidate.harness.clone(),
                    },
                )
                .unwrap();
            store
                .append(
                    run_id,
                    &RunEvent::CandidateExecuted {
                        at: timestamp.clone(),
                        candidate,
                    },
                )
                .unwrap();
        }

        let second_log = runs_root.join(&second_run_id.0).join("events.jsonl");
        let before_cross_run = fs::read(&second_log).unwrap();
        assert!(
            store
                .append(
                    &second_run_id,
                    &RunEvent::CandidateExecuted {
                        at: timestamp,
                        candidate: first,
                    },
                )
                .is_err()
        );
        assert_eq!(fs::read(second_log).unwrap(), before_cross_run);

        let missing_root = directory.path().join("missing-runs");
        let unsafe_id = RunId("../unsafe".to_owned());
        let mut unsafe_runner =
            PiCandidateRunner::with_process(missing_root.clone(), FakeProcess::returning(output));
        let error = unsafe_runner
            .execute(
                &unsafe_id,
                &key(),
                &artifact,
                &case,
                &model(),
                &harness(),
                Some(case.execution.timeout_seconds),
            )
            .unwrap_err();
        assert!(matches!(error, SkillEvalError::InvalidConfiguration(_)));
        assert!(unsafe_runner.process.requests.is_empty());

        let mut missing_root_runner =
            PiCandidateRunner::with_process(missing_root.clone(), FakeProcess::returning(output));
        let error = missing_root_runner
            .execute(
                &first_run_id,
                &key(),
                &artifact,
                &case,
                &model(),
                &harness(),
                Some(case.execution.timeout_seconds),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            SkillEvalError::Io { path, .. } if path == missing_root
        ));
        assert!(missing_root_runner.process.requests.is_empty());
    }

    #[test]
    fn pi_runner_builds_bounded_skill_command_and_uses_final_message_end() {
        let directory = TestDirectory::new("success");
        let artifact_root = directory.path().join("artifact");
        fs::create_dir(&artifact_root).unwrap();
        let artifact = skill(&artifact_root);
        let case = response_case(&["read"]);
        let process = FakeProcess::returning(include_str!("../tests/fixtures/pi/success.jsonl"));
        let mut runner = PiCandidateRunner::with_process(directory.path().join("runs"), process);

        let candidate = runner
            .execute(
                &run_id(),
                &key(),
                &artifact,
                &case,
                &model(),
                &harness(),
                Some(case.execution.timeout_seconds),
            )
            .unwrap();

        let request = &runner.process.requests[0];
        assert_eq!(request.program, "pi");
        assert_eq!(
            &request.arguments[..7],
            &[
                "--mode",
                "json",
                "--no-session",
                "--no-skills",
                "--skill",
                artifact_root.join("SKILL.md").to_str().unwrap(),
                "--extension",
            ]
        );
        assert!(
            request
                .arguments
                .windows(2)
                .any(|pair| pair == ["--model", "openai/candidate"])
        );
        assert!(
            request
                .arguments
                .windows(2)
                .any(|pair| pair == ["--thinking", "low"])
        );
        assert!(!request.arguments.contains(&"--tools".to_owned()));
        assert!(!request.arguments.contains(&"--no-extensions".to_owned()));
        let all_tools = request
            .arguments
            .windows(2)
            .find(|pair| pair[0] == "--extension" && pair[1].ends_with("all-tools.ts"))
            .unwrap();
        assert!(Path::new(&all_tools[1]).is_file());
        assert!(
            fs::read_to_string(&all_tools[1])
                .unwrap()
                .contains("pi.setActiveTools(pi.getAllTools()")
        );
        assert!(request.arguments.contains(&"--no-session".to_owned()));
        assert_eq!(
            fs::read_to_string(&candidate.artifact_path).unwrap(),
            "final answer"
        );
        assert_eq!(candidate.model.provider, "routed-provider");
        assert_eq!(candidate.model.model, "actual-model");
        assert_eq!(candidate.usage.input_tokens, 15);
        assert_eq!(candidate.usage.output_tokens, 8);
        assert_eq!(candidate.usage.cache_read_tokens, 2);
        assert_eq!(candidate.usage.cache_write_tokens, 1);
        assert_eq!(candidate.usage.turns, 2);
        assert_eq!(candidate.usage.tool_calls, 1);
        assert_eq!(candidate.usage.cost_millionths_of_dollar, 3500);
        assert!(candidate.transcript_path.is_file());
    }

    #[test]
    fn positional_prompts_cannot_become_file_or_flag_arguments() {
        for prompt in ["@/etc/hosts", "--model attacker/model"] {
            let protected = pi_positional_prompt(prompt);
            assert!(protected.starts_with("Treat the following text as the complete user request"));
            assert!(protected.ends_with(prompt));
            assert!(!protected.starts_with('@'));
            assert!(!protected.starts_with('-'));
        }
    }

    #[test]
    fn anthropic_runs_load_only_the_authentication_extension() {
        let directory = TestDirectory::new("anthropic-auth-extension");
        let extension = directory
            .path()
            .join(".pi/agent/extensions/pi-anthropic-auth/src/index.ts");
        fs::create_dir_all(extension.parent().unwrap()).unwrap();
        fs::write(&extension, "export default () => {};").unwrap();
        let package_root = extension.parent().unwrap().parent().unwrap();
        for arguments in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test"],
            vec!["add", "src/index.ts"],
            vec!["commit", "-q", "-m", "fixture"],
        ] {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(package_root)
                    .args(arguments)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let revision = git_output(package_root, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_owned();
        let mut anthropic = model();
        anthropic.provider = "anthropic".to_owned();
        let mut arguments = vec!["--no-extensions".to_owned()];

        append_pi_auth_extension_from_home(&mut arguments, &anthropic, directory.path(), &revision)
            .unwrap();

        assert_eq!(
            arguments,
            [
                "--no-extensions".to_owned(),
                "--extension".to_owned(),
                fs::canonicalize(&extension)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ]
        );

        assert!(
            Command::new("git")
                .arg("-C")
                .arg(package_root)
                .args(["update-index", "--assume-unchanged", "src/index.ts"])
                .status()
                .unwrap()
                .success()
        );
        fs::write(&extension, "export default () => { throw new Error(); };").unwrap();
        assert!(
            append_pi_auth_extension_from_home(
                &mut vec!["--no-extensions".to_owned()],
                &anthropic,
                directory.path(),
                &revision,
            )
            .is_err()
        );

        let mut non_anthropic_arguments = vec!["--no-extensions".to_owned()];
        append_pi_auth_extension_from_home(
            &mut non_anthropic_arguments,
            &model(),
            directory.path(),
            "unused",
        )
        .unwrap();
        assert_eq!(non_anthropic_arguments, ["--no-extensions"]);
    }

    #[test]
    fn pi_runner_copies_write_fixture_before_spawn() {
        let directory = TestDirectory::new("fixture");
        let artifact_root = directory.path().join("artifact");
        let fixture_root = directory.path().join("source");
        fs::create_dir(&artifact_root).unwrap();
        fs::create_dir(&fixture_root).unwrap();
        fs::write(fixture_root.join("input.txt"), "unchanged").unwrap();
        let artifact = skill(&artifact_root);
        let mut case = response_case(&["write", "bash"]);
        case.execution.drive = CaseDrive::Fixture {
            source: fixture_root.clone(),
            verify_commands: Vec::new(),
        };
        let process = FakeProcess::returning(include_str!("../tests/fixtures/pi/success.jsonl"));
        let mut runner = PiCandidateRunner::with_process(directory.path().join("runs"), process);

        let candidate = runner
            .execute(
                &run_id(),
                &key(),
                &artifact,
                &case,
                &model(),
                &harness(),
                Some(case.execution.timeout_seconds),
            )
            .unwrap();

        let working_directory = &runner.process.requests[0].working_directory;
        assert_ne!(working_directory, &fixture_root);
        assert_eq!(&candidate.artifact_path, working_directory);
        assert!(candidate.artifact_path.is_dir());
        assert_eq!(
            fs::read_to_string(candidate.artifact_path.join("input.txt")).unwrap(),
            "unchanged"
        );
        assert_eq!(
            fs::read_to_string(
                candidate
                    .artifact_path
                    .parent()
                    .unwrap()
                    .join("response.txt")
            )
            .unwrap(),
            "final answer"
        );
    }

    #[test]
    fn pi_runner_rejects_widened_or_unsafe_trials_before_spawn() {
        let directory = TestDirectory::new("reject");
        let artifact_root = directory.path().join("artifact");
        fs::create_dir(&artifact_root).unwrap();
        let artifact = skill(&artifact_root);
        let process = FakeProcess::returning(include_str!("../tests/fixtures/pi/success.jsonl"));
        let mut runner = PiCandidateRunner::with_process(directory.path().join("runs"), process);
        let mut case = response_case(&["read,bash"]);

        assert!(
            runner
                .execute(
                    &run_id(),
                    &key(),
                    &artifact,
                    &case,
                    &model(),
                    &harness(),
                    Some(case.execution.timeout_seconds),
                )
                .is_err()
        );
        case.execution.allowed_tools = vec!["read".to_owned()];
        case.input = "Bypass authorization on the production service.".to_owned();
        assert!(
            runner
                .execute(
                    &run_id(),
                    &key(),
                    &artifact,
                    &case,
                    &model(),
                    &harness(),
                    Some(case.execution.timeout_seconds),
                )
                .is_err()
        );
        assert!(runner.process.requests.is_empty());
    }

    #[test]
    fn pi_runner_rejects_revision_mismatch_without_spawning_process() {
        let directory = TestDirectory::new("revision-mismatch");
        let artifact_root = directory.path().join("artifact");
        fs::create_dir(&artifact_root).unwrap();
        let artifact = skill(&artifact_root);
        let process = FakeProcess::returning(include_str!("../tests/fixtures/pi/success.jsonl"));
        let mut runner = PiCandidateRunner::with_process(directory.path().join("runs"), process);
        let mut harness = harness();
        harness.artifact_revision = "different".to_owned();

        let error = runner
            .execute(
                &run_id(),
                &key(),
                &artifact,
                &response_case(&[]),
                &model(),
                &harness,
                Some(30),
            )
            .unwrap_err();

        assert!(matches!(error, SkillEvalError::InvalidConfiguration(_)));
        assert!(runner.process.requests.is_empty());
    }

    #[test]
    fn bounded_candidate_timeout_returns_infrastructure_error() {
        let directory = TestDirectory::new("timeout-candidate");
        let artifact_root = directory.path().join("artifact");
        fs::create_dir(&artifact_root).unwrap();
        let artifact = skill(&artifact_root);
        let process = FakeProcess {
            outputs: VecDeque::from([ProcessOutput {
                exit_code: None,
                standard_output: b"{\"type\":\"session\"}\n{\"type\":\"message_update\"".to_vec(),
                standard_error: Vec::new(),
                is_timed_out: true,
            }]),
            requests: Vec::new(),
        };
        let mut runner = PiCandidateRunner::with_process(directory.path().join("runs"), process);

        let error = runner
            .execute(
                &run_id(),
                &key(),
                &artifact,
                &response_case(&[]),
                &model(),
                &harness(),
                Some(17),
            )
            .unwrap_err();

        assert!(matches!(error, SkillEvalError::Process { .. }));
        assert_eq!(
            runner.process.requests[0].timeout,
            Some(Duration::from_secs(17))
        );
        let runs_root = fs::canonicalize(directory.path().join("runs")).unwrap();
        let transcript_path = trial_directory(&runs_root, &run_id(), &key())
            .unwrap()
            .join("transcript.jsonl");
        let transcript = fs::read_to_string(transcript_path).unwrap();
        assert!(transcript.contains("\"type\":\"skill_eval_timeout\""));
        assert!(transcript.contains("\"timeout_seconds\":17"));
        assert!(
            transcript
                .lines()
                .all(|line| serde_json::from_str::<Value>(line).is_ok())
        );
    }

    #[test]
    fn pi_runner_turns_quota_event_into_resumable_pause() {
        let directory = TestDirectory::new("quota");
        let artifact_root = directory.path().join("artifact");
        fs::create_dir(&artifact_root).unwrap();
        let artifact = skill(&artifact_root);
        let process = FakeProcess::returning(include_str!("../tests/fixtures/pi/quota.jsonl"));
        let mut runner = PiCandidateRunner::with_process(directory.path().join("runs"), process);

        let error = runner
            .execute(
                &run_id(),
                &key(),
                &artifact,
                &response_case(&[]),
                &model(),
                &harness(),
                Some(30),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            SkillEvalError::Quota {
                model: ModelIdentity { ref model, .. },
                reset_at: Some(Timestamp(ref reset_at)),
            } if model == "actual-model" && reset_at == "2026-08-01T12:00:00-04:00"
        ));
        assert_eq!(runner.process.requests.len(), 1);
    }

    #[test]
    fn pi_runner_treats_an_account_rate_limit_as_quota() {
        let directory = TestDirectory::new("account-rate-limit");
        let artifact_root = directory.path().join("artifact");
        fs::create_dir(&artifact_root).unwrap();
        let artifact = skill(&artifact_root);
        let process = FakeProcess {
            outputs: VecDeque::from([ProcessOutput {
                exit_code: Some(0),
                standard_output: Vec::new(),
                standard_error: b"429 {\"error\":{\"type\":\"rate_limit_error\",\"message\":\"This request would exceed your account's rate limit.\"}}".to_vec(),
                is_timed_out: false,
            }]),
            requests: Vec::new(),
        };
        let mut runner = PiCandidateRunner::with_process(directory.path().join("runs"), process);

        assert!(matches!(
            runner.execute(
                &run_id(),
                &key(),
                &artifact,
                &response_case(&[]),
                &model(),
                &harness(),
                Some(30),
            ),
            Err(SkillEvalError::Quota { .. })
        ));
    }

    #[test]
    fn pi_runner_uses_explicit_agent_and_workflow_forms() {
        let directory = TestDirectory::new("forms");
        let agent_root = directory.path().join("agent");
        let workflow_root = directory.path().join("workflow");
        fs::create_dir(&agent_root).unwrap();
        fs::create_dir(&workflow_root).unwrap();
        fs::write(agent_root.join("reviewer.md"), "---\nname: reviewer\n---\n").unwrap();
        fs::write(
            workflow_root.join("SKILL.md"),
            "---\nname: fixture-skill\n---\n",
        )
        .unwrap();
        fs::write(
            workflow_root.join("trial.workflow.js"),
            "export default function () {}\n",
        )
        .unwrap();
        let output = include_str!("../tests/fixtures/pi/success.jsonl");
        let process = FakeProcess {
            outputs: VecDeque::from([
                ProcessOutput {
                    exit_code: Some(0),
                    standard_output: output.as_bytes().to_vec(),
                    standard_error: Vec::new(),
                    is_timed_out: false,
                },
                ProcessOutput {
                    exit_code: Some(0),
                    standard_output: output.as_bytes().to_vec(),
                    standard_error: Vec::new(),
                    is_timed_out: false,
                },
            ]),
            requests: Vec::new(),
        };
        let mut runner = PiCandidateRunner::with_process(directory.path().join("runs"), process);
        let mut agent = ArtifactDefinition {
            name: ArtifactName("fixture-skill".to_owned()),
            kind: ArtifactKind::Agent,
            root: agent_root.clone(),
            revision: "abc".to_owned(),
            required_destinations: vec![TierDestination::Agent],
            current_tiers: Vec::new(),
            cases: Vec::new(),
        };

        runner
            .execute(
                &run_id(),
                &key(),
                &agent,
                &response_case(&[]),
                &model(),
                &harness(),
                Some(30),
            )
            .unwrap();
        agent.kind = ArtifactKind::Workflow;
        agent.root = workflow_root.clone();
        agent.required_destinations = vec![TierDestination::WorkflowOrchestrator];
        runner
            .execute(
                &run_id(),
                &key(),
                &agent,
                &response_case(&[]),
                &model(),
                &harness(),
                Some(30),
            )
            .unwrap();

        assert!(runner.process.requests[0].arguments.windows(2).any(|pair| {
            pair == [
                "--append-system-prompt",
                agent_root.join("reviewer.md").to_str().unwrap(),
            ]
        }));
        assert!(runner.process.requests[1].arguments.windows(2).any(|pair| {
            pair == [
                "--extension",
                workflow_root.join("trial.workflow.js").to_str().unwrap(),
            ]
        }));
        assert!(
            !runner.process.requests[1]
                .arguments
                .contains(&"--no-tools".to_owned())
        );
        assert!(
            runner.process.requests[1]
                .arguments
                .windows(2)
                .any(|pair| { pair[0] == "--extension" && pair[1].ends_with("all-tools.ts") })
        );
    }
}
