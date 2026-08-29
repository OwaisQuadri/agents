use std::fmt::Write;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::model::{
    CandidateArtifact, CaseDefinition, CaseDrive, CheckResult, CheckStatus, CommandDefinition,
    SkillEvalError,
};
use crate::ports::Verifier;

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const HASH_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const HASH_PRIME: u64 = 0x0000_0100_0000_01b3;

pub(crate) struct FileVerifier {
    engine: VerificationEngine<StandardProcess>,
}

impl FileVerifier {
    pub(crate) fn new(run_root: &Path) -> Result<Self, SkillEvalError> {
        Ok(Self {
            engine: VerificationEngine::new(run_root, StandardProcess)?,
        })
    }
}

impl Verifier for FileVerifier {
    fn verify(
        &mut self,
        case: &CaseDefinition,
        candidate: &CandidateArtifact,
    ) -> Result<Vec<CheckResult>, SkillEvalError> {
        self.engine.verify(case, candidate)
    }
}

struct VerificationEngine<P> {
    run_root: PathBuf,
    process: P,
}

impl<P: Process> VerificationEngine<P> {
    fn new(run_root: &Path, process: P) -> Result<Self, SkillEvalError> {
        let run_root = fs::canonicalize(run_root).map_err(|error| SkillEvalError::Io {
            path: run_root.to_path_buf(),
            message: error.to_string(),
        })?;
        if !run_root.is_dir() {
            return Err(verification(format!(
                "verification root {} is not a directory",
                run_root.display()
            )));
        }
        Ok(Self { run_root, process })
    }

    fn verify(
        &mut self,
        case: &CaseDefinition,
        candidate: &CandidateArtifact,
    ) -> Result<Vec<CheckResult>, SkillEvalError> {
        match &case.execution.drive {
            CaseDrive::Response => {
                self.validate_candidate_paths(candidate)?;
                Ok(vec![CheckResult {
                    name: "deterministic verification".to_string(),
                    status: CheckStatus::NotApplicable,
                    detail: None,
                }])
            }
            CaseDrive::Fixture {
                verify_commands, ..
            } if verify_commands.is_empty() => {
                self.validate_candidate_paths(candidate)?;
                Ok(Vec::new())
            }
            CaseDrive::Fixture {
                verify_commands, ..
            } => self.verify_requested(case, candidate, verify_commands),
            CaseDrive::ExistingHarness { command } => {
                self.verify_requested(case, candidate, std::slice::from_ref(command))
            }
        }
    }

    fn verify_requested(
        &mut self,
        case: &CaseDefinition,
        candidate: &CandidateArtifact,
        requested: &[CommandDefinition],
    ) -> Result<Vec<CheckResult>, SkillEvalError> {
        ensure_declared(case, requested)?;
        self.validate_candidate_paths_lexically(candidate)?;
        let execution_root = self.execution_root(case, candidate)?;
        for command in requested {
            self.validate_command_arguments(command, &execution_root)?;
        }
        if case.execution.timeout_seconds == 0 {
            return Err(verification(format!(
                "case {:?} has a zero verification timeout",
                case.id.0
            )));
        }

        let fixture_root = matches!(case.execution.drive, CaseDrive::Fixture { .. })
            .then_some(execution_root.as_path());
        let baseline = self.candidate_snapshot(candidate, fixture_root)?;
        let mut checks = Vec::with_capacity(requested.len());
        for command in requested {
            let working_directory = self.working_directory(case, command, &execution_root)?;
            let request = ProcessRequest {
                program: &command.program,
                arguments: &command.arguments,
                working_directory: &working_directory,
                timeout: Duration::from_secs(u64::from(case.execution.timeout_seconds)),
            };
            let outcome = self.process.run(request);
            self.ensure_candidate_unchanged(candidate, fixture_root, &baseline)?;
            checks.push(check_from_outcome(command, outcome)?);
        }
        Ok(checks)
    }

    fn validate_candidate_paths(
        &self,
        candidate: &CandidateArtifact,
    ) -> Result<(), SkillEvalError> {
        self.validate_candidate_paths_lexically(candidate)?;
        self.candidate_snapshot(candidate, None).map(|_| ())
    }

    fn validate_candidate_paths_lexically(
        &self,
        candidate: &CandidateArtifact,
    ) -> Result<(), SkillEvalError> {
        for (name, path) in [
            ("candidate artifact", &candidate.artifact_path),
            ("candidate transcript", &candidate.transcript_path),
        ] {
            self.validate_path_lexically(name, path, &self.run_root, false)?;
        }
        Ok(())
    }

    fn execution_root(
        &self,
        case: &CaseDefinition,
        candidate: &CandidateArtifact,
    ) -> Result<PathBuf, SkillEvalError> {
        let path = match case.execution.drive {
            CaseDrive::Fixture { .. } if candidate.artifact_path.is_dir() => {
                candidate.artifact_path.clone()
            }
            CaseDrive::Fixture { .. }
                if candidate.artifact_path.file_name()
                    == Some(Path::new("response.txt").as_os_str()) =>
            {
                candidate
                    .artifact_path
                    .parent()
                    .ok_or_else(|| verification("candidate artifact has no trial directory"))?
                    .join("fixture")
            }
            CaseDrive::Fixture { .. } => candidate.artifact_path.clone(),
            CaseDrive::Response | CaseDrive::ExistingHarness { .. } => candidate
                .artifact_path
                .parent()
                .ok_or_else(|| verification("candidate artifact has no trial directory"))?
                .to_path_buf(),
        };
        let canonical = self.canonical_path("disposable fixture", &path)?;
        if !canonical.is_dir() {
            return Err(verification(format!(
                "disposable fixture {} is not a directory",
                path.display()
            )));
        }
        Ok(canonical)
    }

    fn validate_command_arguments(
        &self,
        command: &CommandDefinition,
        execution_root: &Path,
    ) -> Result<(), SkillEvalError> {
        for argument in &command.arguments {
            let path = Path::new(argument);
            if path.is_absolute() {
                self.validate_path_lexically(
                    "absolute command argument",
                    path,
                    execution_root,
                    true,
                )?;
            } else if path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            }) {
                return Err(verification(format!(
                    "command argument {argument:?} escapes the disposable fixture"
                )));
            }
        }
        Ok(())
    }

    fn validate_path_lexically(
        &self,
        name: &str,
        path: &Path,
        root: &Path,
        is_root_allowed: bool,
    ) -> Result<(), SkillEvalError> {
        let is_normalized_absolute = path.is_absolute()
            && path
                .components()
                .all(|component| !matches!(component, Component::CurDir | Component::ParentDir));
        if !is_normalized_absolute || !path.starts_with(root) || (!is_root_allowed && path == root)
        {
            return Err(verification(format!(
                "{name} {} escapes verification root {}",
                path.display(),
                root.display()
            )));
        }
        Ok(())
    }

    fn canonical_path(&self, name: &str, path: &Path) -> Result<PathBuf, SkillEvalError> {
        let canonical = fs::canonicalize(path).map_err(|error| SkillEvalError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        if !canonical.starts_with(&self.run_root) {
            return Err(verification(format!(
                "{name} {} resolves outside verification root {}",
                path.display(),
                self.run_root.display()
            )));
        }
        Ok(canonical)
    }

    fn working_directory(
        &self,
        case: &CaseDefinition,
        command: &CommandDefinition,
        execution_root: &Path,
    ) -> Result<PathBuf, SkillEvalError> {
        let Some(declared) = command.working_directory.as_deref() else {
            return Ok(execution_root.to_path_buf());
        };
        let mapped = match &case.execution.drive {
            CaseDrive::Fixture { source, .. } => {
                let relative = if declared == source {
                    Path::new("")
                } else if let Ok(relative) = declared.strip_prefix(source) {
                    relative
                } else if source.parent() == Some(declared) {
                    Path::new("")
                } else {
                    return Err(verification(format!(
                        "declared working directory {} is outside fixture source {}",
                        declared.display(),
                        source.display()
                    )));
                };
                execution_root.join(relative)
            }
            CaseDrive::Response | CaseDrive::ExistingHarness { .. } => {
                self.validate_path_lexically(
                    "verification working directory",
                    declared,
                    execution_root,
                    true,
                )?;
                declared.to_path_buf()
            }
        };
        self.validate_path_lexically(
            "mapped verification working directory",
            &mapped,
            execution_root,
            true,
        )?;
        let canonical = fs::canonicalize(&mapped).map_err(|error| SkillEvalError::Io {
            path: mapped.clone(),
            message: error.to_string(),
        })?;
        if !canonical.starts_with(execution_root) || !canonical.is_dir() {
            return Err(verification(format!(
                "verification working directory {} is outside the disposable fixture or is not a directory",
                mapped.display()
            )));
        }
        Ok(canonical)
    }

    fn candidate_snapshot(
        &self,
        candidate: &CandidateArtifact,
        execution_root: Option<&Path>,
    ) -> Result<Vec<SnapshotEntry>, SkillEvalError> {
        let mut entries = Vec::new();
        for (label, path) in [
            ("artifact", &candidate.artifact_path),
            ("transcript", &candidate.transcript_path),
        ] {
            let canonical = self.canonical_path(label, path)?;
            collect_snapshot(&canonical, Path::new(label), &self.run_root, &mut entries)?;
        }
        if let Some(execution_root) = execution_root {
            collect_snapshot(
                execution_root,
                Path::new("fixture"),
                &self.run_root,
                &mut entries,
            )?;
        }
        Ok(entries)
    }

    fn ensure_candidate_unchanged(
        &self,
        candidate: &CandidateArtifact,
        fixture_root: Option<&Path>,
        baseline: &[SnapshotEntry],
    ) -> Result<(), SkillEvalError> {
        let current = self
            .candidate_snapshot(candidate, fixture_root)
            .map_err(|error| {
                verification(format!(
                    "candidate changed or became unreadable during verification: {error:?}"
                ))
            })?;
        if current != baseline {
            return Err(verification(
                "candidate changed during deterministic verification",
            ));
        }
        Ok(())
    }
}

fn ensure_declared(
    case: &CaseDefinition,
    requested: &[CommandDefinition],
) -> Result<(), SkillEvalError> {
    let declared = match &case.execution.drive {
        CaseDrive::Response => Vec::new(),
        CaseDrive::Fixture {
            verify_commands, ..
        } => verify_commands.iter().collect(),
        CaseDrive::ExistingHarness { command } => vec![command],
    };
    let mut is_used = vec![false; declared.len()];
    for command in requested {
        let Some(index) = declared
            .iter()
            .enumerate()
            .position(|(index, declared)| !is_used[index] && *declared == command)
        else {
            return Err(verification(format!(
                "verification command {} was not declared by case {:?}",
                command_name(command),
                case.id.0
            )));
        };
        is_used[index] = true;
    }
    Ok(())
}

fn check_from_outcome(
    command: &CommandDefinition,
    outcome: Result<ProcessOutput, ProcessFailure>,
) -> Result<CheckResult, SkillEvalError> {
    let output = outcome.map_err(|failure| match failure {
        ProcessFailure::Launch(message) => verification(format!(
            "failed to launch declared command {}: {message}",
            command_name(command)
        )),
        ProcessFailure::Monitor(message) => verification(format!(
            "failed while waiting for declared command {}: {message}",
            command_name(command)
        )),
        ProcessFailure::Read(message) => verification(format!(
            "declared command {} produced unreadable output: {message}",
            command_name(command)
        )),
    })?;
    match output {
        ProcessOutput::TimedOut => Err(verification(format!(
            "declared command {} timed out",
            command_name(command)
        ))),
        ProcessOutput::Completed {
            exit_code,
            standard_output,
            standard_error,
        } => {
            let standard_output = String::from_utf8(standard_output).map_err(|error| {
                verification(format!(
                    "declared command {} produced unreadable standard output: {error}",
                    command_name(command)
                ))
            })?;
            let standard_error = String::from_utf8(standard_error).map_err(|error| {
                verification(format!(
                    "declared command {} produced unreadable standard error: {error}",
                    command_name(command)
                ))
            })?;
            let is_passed = exit_code == Some(0);
            Ok(CheckResult {
                name: command_name(command),
                status: if is_passed {
                    CheckStatus::Passed
                } else {
                    CheckStatus::Failed
                },
                detail: output_detail(exit_code, &standard_output, &standard_error),
            })
        }
    }
}

fn output_detail(
    exit_code: Option<i32>,
    standard_output: &str,
    standard_error: &str,
) -> Option<String> {
    let mut detail = String::new();
    if exit_code != Some(0) {
        match exit_code {
            Some(code) => {
                let _ = write!(detail, "exit code: {code}");
            }
            None => detail.push_str("process ended without an exit code"),
        }
    }
    append_output(&mut detail, "standard output", standard_output);
    append_output(&mut detail, "standard error", standard_error);
    (!detail.is_empty()).then_some(detail)
}

fn append_output(detail: &mut String, label: &str, output: &str) {
    if output.is_empty() {
        return;
    }
    if !detail.is_empty() {
        detail.push('\n');
    }
    let _ = write!(detail, "{label}: {output}");
}

fn command_name(command: &CommandDefinition) -> String {
    if command.arguments.is_empty() {
        command.program.clone()
    } else {
        format!("{} {:?}", command.program, command.arguments)
    }
}

#[derive(Debug, Eq, PartialEq)]
struct SnapshotEntry {
    path: PathBuf,
    file: Option<FileSnapshot>,
}

#[derive(Debug, Eq, PartialEq)]
struct FileSnapshot {
    length: u64,
    digest: u64,
}

fn collect_snapshot(
    path: &Path,
    relative: &Path,
    run_root: &Path,
    entries: &mut Vec<SnapshotEntry>,
) -> Result<(), SkillEvalError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if metadata.file_type().is_symlink() {
        return Err(verification(format!(
            "candidate path {} must not be a symbolic link",
            path.display()
        )));
    }
    let canonical = fs::canonicalize(path).map_err(|error| SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if !canonical.starts_with(run_root) || canonical == run_root {
        return Err(verification(format!(
            "candidate path {} resolves outside verification root {}",
            path.display(),
            run_root.display()
        )));
    }
    if metadata.is_file() {
        entries.push(SnapshotEntry {
            path: relative.to_path_buf(),
            file: Some(hash_file(&canonical)?),
        });
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(verification(format!(
            "candidate path {} is not a regular file or directory",
            path.display()
        )));
    }
    entries.push(SnapshotEntry {
        path: relative.to_path_buf(),
        file: None,
    });
    let mut children = fs::read_dir(&canonical)
        .map_err(|error| SkillEvalError::Io {
            path: canonical.clone(),
            message: error.to_string(),
        })?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| SkillEvalError::Io {
                    path: canonical.clone(),
                    message: error.to_string(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    children.sort();
    for child in children {
        let name = child
            .file_name()
            .ok_or_else(|| verification("candidate entry has no file name"))?;
        collect_snapshot(&child, &relative.join(name), run_root, entries)?;
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<FileSnapshot, SkillEvalError> {
    let mut file = File::open(path).map_err(|error| SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let length = file
        .metadata()
        .map_err(|error| SkillEvalError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?
        .len();
    let mut digest = HASH_OFFSET;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer).map_err(|error| SkillEvalError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            digest ^= u64::from(*byte);
            digest = digest.wrapping_mul(HASH_PRIME);
        }
    }
    Ok(FileSnapshot { length, digest })
}

fn verification(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::Verification(message.into())
}

struct ProcessRequest<'a> {
    program: &'a str,
    arguments: &'a [String],
    working_directory: &'a Path,
    timeout: Duration,
}

trait Process {
    fn run(&mut self, request: ProcessRequest<'_>) -> Result<ProcessOutput, ProcessFailure>;
}

enum ProcessOutput {
    Completed {
        exit_code: Option<i32>,
        standard_output: Vec<u8>,
        standard_error: Vec<u8>,
    },
    TimedOut,
}

enum ProcessFailure {
    Launch(String),
    Monitor(String),
    Read(String),
}

struct StandardProcess;

impl Process for StandardProcess {
    fn run(&mut self, request: ProcessRequest<'_>) -> Result<ProcessOutput, ProcessFailure> {
        let program = verification_program(request.program);
        let mut child = Command::new(program)
            .args(request.arguments)
            .current_dir(request.working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| ProcessFailure::Launch(error.to_string()))?;
        let standard_output = child.stdout.take().ok_or_else(|| {
            ProcessFailure::Read("standard output pipe is unavailable".to_string())
        })?;
        let standard_error = child.stderr.take().ok_or_else(|| {
            ProcessFailure::Read("standard error pipe is unavailable".to_string())
        })?;
        let standard_output = thread::spawn(move || read_all(standard_output));
        let standard_error = thread::spawn(move || read_all(standard_error));
        let started = Instant::now();

        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < request.timeout => thread::sleep(POLL_INTERVAL),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(ProcessOutput::TimedOut);
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ProcessFailure::Monitor(error.to_string()));
                }
            }
        };
        Ok(ProcessOutput::Completed {
            exit_code: status.code(),
            standard_output: join_output(standard_output, "standard output")?,
            standard_error: join_output(standard_error, "standard error")?,
        })
    }
}

fn verification_program(program: &str) -> &str {
    if program == "/usr/bin/test"
        && !Path::new(program).exists()
        && Path::new("/bin/test").is_file()
    {
        "/bin/test"
    } else {
        program
    }
}

fn read_all(mut reader: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn join_output(
    handle: thread::JoinHandle<Result<Vec<u8>, String>>,
    name: &str,
) -> Result<Vec<u8>, ProcessFailure> {
    handle
        .join()
        .map_err(|_| ProcessFailure::Read(format!("{name} reader stopped unexpectedly")))?
        .map_err(ProcessFailure::Read)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::env;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::model::{
        ArtifactName, CaseId, ExecutionDefinition, HarnessIdentity, ModelIdentity, Tier, TrialKey,
        TrialUsage,
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        artifact: PathBuf,
        candidate_file: PathBuf,
        transcript: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let identifier = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root = env::temp_dir().join(format!(
                "skill-eval-verifier-{}-{identifier}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            let root = fs::canonicalize(root).unwrap();
            let artifact = root.join("fixture");
            fs::create_dir(&artifact).unwrap();
            let candidate_file = artifact.join("candidate.txt");
            let transcript = root.join("transcript.jsonl");
            fs::write(&candidate_file, "candidate").unwrap();
            fs::write(&transcript, "transcript").unwrap();
            Self {
                root,
                artifact,
                candidate_file,
                transcript,
            }
        }

        fn candidate(&self) -> CandidateArtifact {
            CandidateArtifact {
                key: TrialKey {
                    artifact: ArtifactName("fixture".to_string()),
                    tier: Tier::T2,
                    route_index: 0,
                    case: CaseId("case".to_string()),
                    attempt: 1,
                },
                model: ModelIdentity {
                    tier: Tier::T2,
                    provider: "fixture".to_string(),
                    model: "model".to_string(),
                    thinking: "low".to_string(),
                },
                harness: HarnessIdentity {
                    runner_version: "1".to_string(),
                    pi_version: "1".to_string(),
                    artifact_revision: "revision".to_string(),
                    tool_policy_digest: "digest".to_string(),
                },
                artifact_path: self.artifact.clone(),
                transcript_path: self.transcript.clone(),
                usage: TrialUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    turns: 0,
                    tool_calls: 0,
                    elapsed_milliseconds: 0,
                    cost_millionths_of_dollar: 0,
                },
            }
        }

        fn candidate_with_response_artifact(&self) -> CandidateArtifact {
            let response = self.root.join("response.txt");
            fs::write(&response, "candidate response").unwrap();
            let mut candidate = self.candidate();
            candidate.artifact_path = response;
            candidate
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    enum FakeAction {
        Complete {
            exit_code: Option<i32>,
            standard_output: Vec<u8>,
            standard_error: Vec<u8>,
        },
        Timeout,
        Fail(ProcessFailure),
        Mutate(PathBuf),
    }

    #[derive(Default)]
    struct FakeProcess {
        actions: VecDeque<FakeAction>,
        calls: Vec<(String, Vec<String>, PathBuf, Duration)>,
    }

    impl FakeProcess {
        fn with(actions: impl IntoIterator<Item = FakeAction>) -> Self {
            Self {
                actions: actions.into_iter().collect(),
                calls: Vec::new(),
            }
        }
    }

    impl Process for FakeProcess {
        fn run(&mut self, request: ProcessRequest<'_>) -> Result<ProcessOutput, ProcessFailure> {
            self.calls.push((
                request.program.to_string(),
                request.arguments.to_vec(),
                request.working_directory.to_path_buf(),
                request.timeout,
            ));
            match self.actions.pop_front().unwrap() {
                FakeAction::Complete {
                    exit_code,
                    standard_output,
                    standard_error,
                } => Ok(ProcessOutput::Completed {
                    exit_code,
                    standard_output,
                    standard_error,
                }),
                FakeAction::Timeout => Ok(ProcessOutput::TimedOut),
                FakeAction::Fail(failure) => Err(failure),
                FakeAction::Mutate(path) => {
                    fs::write(path, "mutated").unwrap();
                    Ok(ProcessOutput::Completed {
                        exit_code: Some(0),
                        standard_output: Vec::new(),
                        standard_error: Vec::new(),
                    })
                }
            }
        }
    }

    fn command(program: &str, root: &Path) -> CommandDefinition {
        CommandDefinition {
            program: program.to_string(),
            arguments: vec!["--fixture".to_string()],
            working_directory: Some(root.to_path_buf()),
        }
    }

    fn fixture_case(commands: Vec<CommandDefinition>) -> CaseDefinition {
        let source = commands
            .first()
            .and_then(|command| command.working_directory.clone())
            .unwrap_or_else(|| PathBuf::from("fixture"));
        CaseDefinition {
            id: CaseId("case".to_string()),
            input: "input".to_string(),
            expect: "expect".to_string(),
            source: "source".to_string(),
            is_holdout: false,
            support_files: Vec::new(),
            execution: ExecutionDefinition {
                drive: CaseDrive::Fixture {
                    source,
                    verify_commands: commands,
                },
                allowed_tools: Vec::new(),
                timeout_seconds: 2,
            },
        }
    }

    fn response_case() -> CaseDefinition {
        let mut case = fixture_case(Vec::new());
        case.execution.drive = CaseDrive::Response;
        case
    }

    fn message(error: SkillEvalError) -> String {
        match error {
            SkillEvalError::Verification(message) => message,
            other => panic!("expected verification error, got {other:?}"),
        }
    }

    #[test]
    fn system_process_runs_the_portable_test_program() {
        let declared = Path::new("/usr/bin/test");
        let fallback = Path::new("/bin/test");
        if !declared.exists() && !fallback.is_file() {
            return;
        }
        assert_eq!(
            verification_program("/usr/bin/test"),
            if declared.exists() {
                "/usr/bin/test"
            } else {
                "/bin/test"
            }
        );
        let fixture = Fixture::new();
        let arguments = vec!["!".to_owned(), "-e".to_owned(), "missing".to_owned()];
        let mut process = StandardProcess;

        let outcome = match process.run(ProcessRequest {
            program: "/usr/bin/test",
            arguments: &arguments,
            working_directory: &fixture.root,
            timeout: Duration::from_secs(2),
        }) {
            Ok(outcome) => outcome,
            Err(_) => panic!("portable test program failed to launch"),
        };

        assert!(matches!(
            outcome,
            ProcessOutput::Completed {
                exit_code: Some(0),
                ..
            }
        ));
    }

    #[test]
    fn verifier_pass_and_fail_preserve_all_results() {
        let fixture = Fixture::new();
        let commands = vec![
            command("pass", &fixture.root),
            command("fail", &fixture.root),
        ];
        let case = fixture_case(commands);
        let process = FakeProcess::with([
            FakeAction::Complete {
                exit_code: Some(0),
                standard_output: b"passed".to_vec(),
                standard_error: Vec::new(),
            },
            FakeAction::Complete {
                exit_code: Some(7),
                standard_output: Vec::new(),
                standard_error: b"failed".to_vec(),
            },
        ]);
        let mut verifier = VerificationEngine::new(&fixture.root, process).unwrap();

        let checks = verifier.verify(&case, &fixture.candidate()).unwrap();

        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].status, CheckStatus::Passed);
        assert_eq!(checks[0].detail.as_deref(), Some("standard output: passed"));
        assert_eq!(checks[1].status, CheckStatus::Failed);
        assert_eq!(
            checks[1].detail.as_deref(),
            Some("exit code: 7\nstandard error: failed")
        );
        assert_eq!(verifier.process.calls.len(), 2);
    }

    #[test]
    fn verifier_maps_a_response_artifact_to_its_sibling_fixture() {
        let fixture = Fixture::new();
        let declared = command("pass", &fixture.root);
        let case = fixture_case(vec![declared]);
        let process = FakeProcess::with([FakeAction::Complete {
            exit_code: Some(0),
            standard_output: Vec::new(),
            standard_error: Vec::new(),
        }]);
        let mut verifier = VerificationEngine::new(&fixture.root, process).unwrap();

        let checks = verifier
            .verify(&case, &fixture.candidate_with_response_artifact())
            .unwrap();

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, CheckStatus::Passed);
        assert_eq!(verifier.process.calls[0].2, fixture.artifact);
    }

    #[test]
    fn verifier_tracks_the_sibling_fixture_during_response_artifact_checks() {
        let fixture = Fixture::new();
        let declared = command("mutate", &fixture.root);
        let case = fixture_case(vec![declared]);
        let process = FakeProcess::with([FakeAction::Mutate(fixture.candidate_file.clone())]);
        let mut verifier = VerificationEngine::new(&fixture.root, process).unwrap();

        let error = verifier
            .verify(&case, &fixture.candidate_with_response_artifact())
            .unwrap_err();

        assert!(message(error).contains("candidate changed"));
    }

    #[test]
    fn verifier_existing_harness_uses_program_and_argument_vector() {
        let fixture = Fixture::new();
        let declared = CommandDefinition {
            program: "verify harness".to_string(),
            arguments: vec!["literal $VALUE; exit 9".to_string()],
            working_directory: None,
        };
        let mut case = fixture_case(Vec::new());
        case.execution.drive = CaseDrive::ExistingHarness { command: declared };
        let process = FakeProcess::with([FakeAction::Complete {
            exit_code: Some(0),
            standard_output: Vec::new(),
            standard_error: Vec::new(),
        }]);
        let mut verifier = VerificationEngine::new(&fixture.root, process).unwrap();

        let checks = verifier.verify(&case, &fixture.candidate()).unwrap();

        assert_eq!(checks[0].status, CheckStatus::Passed);
        assert_eq!(
            verifier.process.calls[0],
            (
                "verify harness".to_string(),
                vec!["literal $VALUE; exit 9".to_string()],
                fixture.root.clone(),
                Duration::from_secs(2),
            )
        );
    }

    #[test]
    fn verifier_timeout_is_rejected() {
        let fixture = Fixture::new();
        let declared = command("slow", &fixture.root);
        let case = fixture_case(vec![declared]);
        let process = FakeProcess::with([FakeAction::Timeout]);
        let mut verifier = VerificationEngine::new(&fixture.root, process).unwrap();

        let error = verifier.verify(&case, &fixture.candidate()).unwrap_err();

        assert!(message(error).contains("timed out"));
        assert_eq!(verifier.process.calls.len(), 1);
    }

    #[test]
    fn verifier_escape_is_rejected_before_spawn() {
        let fixture = Fixture::new();
        let outside = fixture.root.parent().unwrap().join("synthetic-outside");
        let declared = CommandDefinition {
            program: "verify".to_string(),
            arguments: vec![outside.display().to_string()],
            working_directory: Some(fixture.root.clone()),
        };
        let case = fixture_case(vec![declared]);
        let mut verifier = VerificationEngine::new(&fixture.root, FakeProcess::default()).unwrap();

        let error = verifier.verify(&case, &fixture.candidate()).unwrap_err();

        assert!(message(error).contains("escapes verification root"));
        assert!(verifier.process.calls.is_empty());
    }

    #[test]
    fn verifier_candidate_escape_is_rejected_before_spawn() {
        let fixture = Fixture::new();
        let case = fixture_case(vec![command("verify", &fixture.root)]);
        let mut candidate = fixture.candidate();
        candidate.transcript_path = fixture.root.parent().unwrap().join("synthetic-transcript");
        let mut verifier = VerificationEngine::new(&fixture.root, FakeProcess::default()).unwrap();

        let error = verifier.verify(&case, &candidate).unwrap_err();

        assert!(message(error).contains("candidate transcript"));
        assert!(verifier.process.calls.is_empty());
    }

    #[test]
    fn verifier_response_and_empty_fixture_are_explicit_no_ops() {
        let fixture = Fixture::new();
        let mut verifier = VerificationEngine::new(&fixture.root, FakeProcess::default()).unwrap();

        let response = verifier
            .verify(&response_case(), &fixture.candidate())
            .unwrap();
        let fixture_checks = verifier
            .verify(&fixture_case(Vec::new()), &fixture.candidate())
            .unwrap();

        assert_eq!(response.len(), 1);
        assert_eq!(response[0].status, CheckStatus::NotApplicable);
        assert!(fixture_checks.is_empty());
        assert!(verifier.process.calls.is_empty());
    }

    #[test]
    fn verifier_undeclared_command_never_spawns() {
        let fixture = Fixture::new();
        let declared = command("declared", &fixture.root);
        let undeclared = command("undeclared", &fixture.root);
        let case = fixture_case(vec![declared]);
        let mut verifier = VerificationEngine::new(&fixture.root, FakeProcess::default()).unwrap();

        let error = verifier
            .verify_requested(&case, &fixture.candidate(), &[undeclared])
            .unwrap_err();

        assert!(message(error).contains("was not declared"));
        assert!(verifier.process.calls.is_empty());
    }

    #[test]
    fn verifier_unreadable_output_is_rejected() {
        let fixture = Fixture::new();
        let declared = command("verify", &fixture.root);
        let case = fixture_case(vec![declared]);
        let process = FakeProcess::with([FakeAction::Complete {
            exit_code: Some(0),
            standard_output: vec![0xff],
            standard_error: Vec::new(),
        }]);
        let mut verifier = VerificationEngine::new(&fixture.root, process).unwrap();

        let error = verifier.verify(&case, &fixture.candidate()).unwrap_err();

        assert!(message(error).contains("unreadable standard output"));
    }

    #[test]
    fn verifier_candidate_mutation_is_rejected() {
        let fixture = Fixture::new();
        let declared = command("mutate", &fixture.root);
        let case = fixture_case(vec![declared]);
        let process = FakeProcess::with([FakeAction::Mutate(fixture.candidate_file.clone())]);
        let mut verifier = VerificationEngine::new(&fixture.root, process).unwrap();

        let error = verifier.verify(&case, &fixture.candidate()).unwrap_err();

        assert!(message(error).contains("candidate changed"));
    }

    #[test]
    fn verifier_process_failures_are_rejected() {
        let fixture = Fixture::new();
        for (failure, expected) in [
            (
                ProcessFailure::Launch("launch".to_string()),
                "failed to launch",
            ),
            (
                ProcessFailure::Monitor("wait".to_string()),
                "failed while waiting",
            ),
            (
                ProcessFailure::Read("read".to_string()),
                "unreadable output",
            ),
        ] {
            let declared = command("verify", &fixture.root);
            let case = fixture_case(vec![declared]);
            let process = FakeProcess::with([FakeAction::Fail(failure)]);
            let mut verifier = VerificationEngine::new(&fixture.root, process).unwrap();

            let error = verifier.verify(&case, &fixture.candidate()).unwrap_err();

            assert!(message(error).contains(expected));
        }
    }
}
