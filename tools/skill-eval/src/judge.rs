use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::model::{
    JudgeInput, JudgeResult, ModelIdentity, PromptJudgeRequest, PromptJudgeResult, SkillEvalError,
    Timestamp, TrialUsage, TrialVerdict,
};
use crate::pi_runner::{append_pi_auth_extension, pi_positional_prompt};
use crate::ports::Judge;

const GRADE_TIMEOUT: Duration = Duration::from_secs(300);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const JUDGE_PACKET_DIRECTORY: &str = "judge-evidence";
const JUDGE_TRANSCRIPT_NAME: &str = "judge-transcript.jsonl";
const LOCKED_READ_EXTENSION_NAME: &str = "locked-read.ts";
const MAX_JUDGE_FILE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_JUDGE_PACKET_BYTES: u64 = 20 * 1024 * 1024;
const MAX_JUDGE_DIRECTORY_ENTRIES: usize = 10_000;
const MAX_PI_EVENT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProcessRequest {
    program: String,
    arguments: Vec<String>,
    working_directory: PathBuf,
    timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProcessOutput {
    exit_code: Option<i32>,
    standard_output: Vec<u8>,
    standard_error: Vec<u8>,
    is_timed_out: bool,
}

struct JudgePacket {
    root: PathBuf,
    read_extension: PathBuf,
    transcript: PathBuf,
}

trait Process {
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
        let wait_result = (|| -> io::Result<(Option<i32>, bool)> {
            loop {
                if let Some(status) = child.try_wait()? {
                    break Ok((status.code(), false));
                }
                if started_at.elapsed() >= request.timeout {
                    child.kill()?;
                    let status = child.wait()?;
                    break Ok((status.code(), true));
                }
                thread::sleep(PROCESS_POLL_INTERVAL);
            }
        })();
        stop_process_group(process_group);
        let (exit_code, is_timed_out) = wait_result?;

        Ok(ProcessOutput {
            exit_code,
            standard_output: join_reader(stdout_reader)?,
            standard_error: join_reader(stderr_reader)?,
            is_timed_out,
        })
    }
}

fn stop_process_group(process_group: u32) {
    let group = format!("-{process_group}");
    for signal in ["-TERM", "-KILL"] {
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

pub(crate) struct PiJudge<P = SystemProcess> {
    process: P,
}

impl PiJudge<SystemProcess> {
    pub(crate) fn new() -> Self {
        Self {
            process: SystemProcess,
        }
    }
}

impl<P> PiJudge<P> {
    fn with_process(process: P) -> Self {
        Self { process }
    }

    pub(crate) fn recover_frontier_grade(
        &self,
        model: &ModelIdentity,
        input: &JudgeInput,
    ) -> Result<Option<JudgeResult>, SkillEvalError> {
        validate_identities(model, Some(&input.candidate.model))?;
        ensure_external(model, Some(&input.candidate.model))?;
        let artifact = canonical_file_or_directory(&input.candidate.artifact_path)?;
        let trial_root = artifact
            .parent()
            .ok_or_else(|| invalid_configuration("candidate artifact has no trial directory"))?;
        let evidence_root = trial_root.join(JUDGE_PACKET_DIRECTORY);
        let entries = match fs::read_dir(&evidence_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(&evidence_root, error)),
        };
        let mut attempts = entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| io_error(&evidence_root, error))?;
        attempts.sort();
        for attempt in attempts.into_iter().rev() {
            let attempt_metadata =
                fs::symlink_metadata(&attempt).map_err(|error| io_error(&attempt, error))?;
            if attempt_metadata.file_type().is_symlink() {
                return Err(invalid_configuration(
                    "frontier judge recovery attempt is a symbolic link",
                ));
            }
            if !attempt_metadata.is_dir() {
                continue;
            }
            let transcript_path = attempt.join(JUDGE_TRANSCRIPT_NAME);
            let transcript_metadata = match fs::symlink_metadata(&transcript_path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error(&transcript_path, error)),
            };
            if transcript_metadata.file_type().is_symlink() {
                return Err(invalid_configuration(
                    "frontier judge recovery transcript is a symbolic link",
                ));
            }
            if !transcript_metadata.is_file() {
                continue;
            }
            let transcript =
                fs::read(&transcript_path).map_err(|error| io_error(&transcript_path, error))?;
            if transcript.len() > MAX_PI_EVENT_BYTES {
                return Err(invalid_configuration(
                    "frontier judge recovery transcript is too large",
                ));
            }
            let mut parsed = parse_events(&transcript, model, 0)?;
            parsed.usage.elapsed_milliseconds =
                recovered_elapsed_milliseconds(&attempt, &transcript_path)?;
            if parsed.model != *model
                || parsed.error_message.is_some()
                || !parsed.has_submitted_verdict
            {
                continue;
            }
            return Ok(Some(JudgeResult {
                verdict: parse_verdict(&parsed.response, &input.checks)?,
                model: parsed.model,
                usage: parsed.usage,
            }));
        }
        Ok(None)
    }
}

impl<P: Process> Judge for PiJudge<P> {
    fn grade(
        &mut self,
        model: &ModelIdentity,
        input: &JudgeInput,
    ) -> Result<JudgeResult, SkillEvalError> {
        validate_identities(model, Some(&input.candidate.model))?;
        ensure_external(model, Some(&input.candidate.model))?;
        let packet = prepare_judge_packet(input)?;
        let prompt = grade_prompt_text(input)?;
        let result = run_judge(
            self,
            model,
            &prompt,
            GRADE_TIMEOUT,
            Some(&input.candidate.model),
            Some(&packet),
        )?;
        Ok(JudgeResult {
            verdict: parse_verdict(&result.response, &input.checks)?,
            model: result.model,
            usage: result.usage,
        })
    }

    fn grade_prompt(
        &mut self,
        model: &ModelIdentity,
        request: &PromptJudgeRequest,
    ) -> Result<PromptJudgeResult, SkillEvalError> {
        validate_prompt_request(request)?;
        validate_identities(model, request.candidate_model.as_ref())?;
        ensure_external(model, request.candidate_model.as_ref())?;
        run_judge(
            self,
            model,
            &request.prompt,
            Duration::from_secs(u64::from(request.timeout_seconds)),
            request.candidate_model.as_ref(),
            None,
        )
    }
}

fn run_judge<P: Process>(
    judge: &mut PiJudge<P>,
    model: &ModelIdentity,
    prompt: &str,
    timeout: Duration,
    candidate: Option<&ModelIdentity>,
    packet: Option<&JudgePacket>,
) -> Result<PromptJudgeResult, SkillEvalError> {
    let working_directory = match packet {
        Some(packet) => packet.root.clone(),
        None => env::current_dir().map_err(|error| SkillEvalError::Io {
            path: PathBuf::from("."),
            message: error.to_string(),
        })?,
    };
    let request = ProcessRequest {
        program: "pi".to_owned(),
        arguments: pi_arguments(
            model,
            prompt,
            packet.map(|packet| packet.read_extension.as_path()),
        )?,
        working_directory,
        timeout,
    };
    let started_at = Instant::now();
    let output = judge
        .process
        .run(&request)
        .map_err(|error| SkillEvalError::Process {
            program: request.program.clone(),
            exit_code: None,
            standard_error: error.to_string(),
        })?;
    if let Some(packet) = packet {
        write_private_file(&packet.transcript, &output.standard_output)?;
    }
    let raw_output = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.standard_output),
        String::from_utf8_lossy(&output.standard_error)
    );
    if output.exit_code != Some(0) && is_quota_text(&raw_output) {
        return Err(SkillEvalError::Quota {
            model: model.clone(),
            reset_at: quota_reset_at(&raw_output),
        });
    }
    if output.is_timed_out {
        return Err(SkillEvalError::Process {
            program: request.program,
            exit_code: output.exit_code,
            standard_error: format!("Pi exceeded its {} second timeout", timeout.as_secs()),
        });
    }

    let parsed = match parse_events(
        &output.standard_output,
        model,
        duration_milliseconds(started_at.elapsed()),
    ) {
        Ok(parsed) => parsed,
        Err(_) if is_quota_text(&raw_output) => {
            return Err(SkillEvalError::Quota {
                model: model.clone(),
                reset_at: quota_reset_at(&raw_output),
            });
        }
        Err(error) => return Err(error),
    };
    ensure_external(&parsed.model, candidate)?;
    if packet.is_some() && !parsed.has_submitted_verdict {
        return Err(SkillEvalError::InvalidEvent {
            line: 0,
            message: "judge did not use submit_verdict".to_owned(),
        });
    }
    if let Some(message) = parsed.error_message.as_deref() {
        let error_text = format!(
            "{message}\n{}",
            String::from_utf8_lossy(&output.standard_error)
        );
        if is_quota_text(&error_text) {
            return Err(SkillEvalError::Quota {
                model: parsed.model,
                reset_at: quota_reset_at(&error_text),
            });
        }
    }
    if output.exit_code != Some(0) || parsed.error_message.is_some() {
        return Err(SkillEvalError::Process {
            program: request.program,
            exit_code: output.exit_code,
            standard_error: process_error_text(&output, parsed.error_message.as_deref()),
        });
    }
    Ok(PromptJudgeResult {
        model: parsed.model,
        response: parsed.response,
        usage: parsed.usage,
    })
}

fn validate_prompt_request(request: &PromptJudgeRequest) -> Result<(), SkillEvalError> {
    if request.prompt.trim().is_empty() {
        return Err(SkillEvalError::InvalidArguments(
            "judge prompt must not be empty".to_owned(),
        ));
    }
    if request.timeout_seconds == 0 {
        return Err(SkillEvalError::InvalidArguments(
            "judge timeout must be greater than zero".to_owned(),
        ));
    }
    Ok(())
}

fn validate_identities(
    judge: &ModelIdentity,
    candidate: Option<&ModelIdentity>,
) -> Result<(), SkillEvalError> {
    validate_model(judge)?;
    if let Some(candidate) = candidate {
        validate_model(candidate)?;
    }
    Ok(())
}

fn validate_model(identity: &ModelIdentity) -> Result<(), SkillEvalError> {
    let is_provider_valid =
        !identity.provider.contains('/') && is_model_segment(&identity.provider);
    let is_model_valid = identity.model.split('/').all(is_model_segment);
    if !is_provider_valid || !is_model_valid || identity.thinking.is_empty() {
        return Err(SkillEvalError::InvalidConfiguration(
            "judge model identity is malformed".to_owned(),
        ));
    }
    Ok(())
}

fn is_model_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.is_ascii()
        && segment != "."
        && segment != ".."
        && !segment
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

fn ensure_external(
    judge: &ModelIdentity,
    candidate: Option<&ModelIdentity>,
) -> Result<(), SkillEvalError> {
    if let Some(candidate) = candidate
        && candidate.provider == judge.provider
        && candidate.model == judge.model
    {
        return Err(SkillEvalError::JudgeUnavailable {
            candidate: candidate.clone(),
            judge_tier: judge.tier,
        });
    }
    Ok(())
}

fn grade_prompt_text(_input: &JudgeInput) -> Result<String, SkillEvalError> {
    let evidence = json!({
        "artifact_path": "artifact",
        "response_path": "response.txt",
        "transcript_path": "transcript.jsonl",
        "expectation_path": "expectation.txt",
        "rubric_path": "rubric.md",
        "deterministic_checks_path": "checks.json",
    });
    let evidence = serde_json::to_string_pretty(&evidence).map_err(|error| {
        SkillEvalError::InvalidConfiguration(format!("judge input cannot be serialized: {error}"))
    })?;
    Ok(format!(
        "Grade one anonymous candidate artifact. Read the named artifact, transcript, and rubric. Use the deterministic checks as fixed evidence. Do not infer or report the candidate model. Call submit_verdict exactly once with score (an integer from 0 through 10), is_catastrophic (a boolean), and failure_mode (a string or null). Do not return the verdict as plain text.\n\n{evidence}"
    ))
}

fn prepare_judge_packet(input: &JudgeInput) -> Result<JudgePacket, SkillEvalError> {
    let artifact = canonical_file_or_directory(&input.candidate.artifact_path)?;
    let transcript = canonical_file(&input.candidate.transcript_path)?;
    let trial_root = artifact
        .parent()
        .ok_or_else(|| invalid_configuration("candidate artifact has no trial directory"))?;
    if transcript.parent() != Some(trial_root) {
        return Err(invalid_configuration(
            "candidate artifact and transcript do not share one trial directory",
        ));
    }

    let root = create_judge_attempt_directory(trial_root)?;
    let mut packet_bytes = 0_u64;
    let packet_artifact = root.join("artifact");
    create_private_directory(&packet_artifact)?;
    if artifact.is_dir() {
        copy_packet_directory(
            &artifact,
            &packet_artifact,
            &input.candidate.model,
            &mut packet_bytes,
        )?;
    } else {
        let name = artifact
            .file_name()
            .ok_or_else(|| invalid_configuration("candidate artifact has no file name"))?;
        let name = sanitized_entry_name(name, &input.candidate.model)?;
        copy_candidate_file(
            &artifact,
            &packet_artifact.join(name),
            &input.candidate.model,
            &mut packet_bytes,
        )?;
    }

    let response = if artifact.is_file() {
        artifact.clone()
    } else {
        canonical_file(&trial_root.join("response.txt"))?
    };
    copy_candidate_file(
        &response,
        &root.join("response.txt"),
        &input.candidate.model,
        &mut packet_bytes,
    )?;
    write_sanitized_transcript(
        &transcript,
        &root.join("transcript.jsonl"),
        &input.candidate.model,
        &mut packet_bytes,
    )?;
    copy_candidate_file(
        &canonical_file(&input.rubric_path)?,
        &root.join("rubric.md"),
        &input.candidate.model,
        &mut packet_bytes,
    )?;
    let expectation = redact_candidate_text(&input.expect, &input.candidate.model);
    reserve_packet_bytes(
        &mut packet_bytes,
        expectation.len() as u64,
        &root.join("expectation.txt"),
    )?;
    write_private_file(&root.join("expectation.txt"), expectation.as_bytes())?;
    let checks = serde_json::to_string_pretty(&input.checks).map_err(|error| {
        invalid_configuration(format!("judge checks cannot be serialized: {error}"))
    })?;
    let checks = redact_candidate_text(&checks, &input.candidate.model);
    reserve_packet_bytes(
        &mut packet_bytes,
        checks.len() as u64,
        &root.join("checks.json"),
    )?;
    write_private_file(&root.join("checks.json"), checks.as_bytes())?;

    let read_extension = root.join(LOCKED_READ_EXTENSION_NAME);
    let source = locked_read_extension_source(&root)?;
    write_private_file(&read_extension, source.as_bytes())?;
    Ok(JudgePacket {
        transcript: root.join(JUDGE_TRANSCRIPT_NAME),
        root,
        read_extension,
    })
}

fn canonical_file(path: &Path) -> Result<PathBuf, SkillEvalError> {
    reject_symbolic_link(path)?;
    let canonical = fs::canonicalize(path).map_err(|error| io_error(path, error))?;
    if !canonical.is_file() {
        return Err(invalid_configuration(format!(
            "judge evidence {} is not a file",
            path.display()
        )));
    }
    Ok(canonical)
}

fn canonical_file_or_directory(path: &Path) -> Result<PathBuf, SkillEvalError> {
    reject_symbolic_link(path)?;
    let canonical = fs::canonicalize(path).map_err(|error| io_error(path, error))?;
    if !canonical.is_file() && !canonical.is_dir() {
        return Err(invalid_configuration(format!(
            "judge evidence {} is not a file or directory",
            path.display()
        )));
    }
    Ok(canonical)
}

fn reject_symbolic_link(path: &Path) -> Result<(), SkillEvalError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink() {
        return Err(invalid_configuration(format!(
            "judge evidence {} must not be a symbolic link",
            path.display()
        )));
    }
    Ok(())
}

fn create_judge_attempt_directory(trial_root: &Path) -> Result<PathBuf, SkillEvalError> {
    let base = trial_root.join(JUDGE_PACKET_DIRECTORY);
    match fs::symlink_metadata(&base) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(invalid_configuration(format!(
                "judge evidence directory {} is not a plain directory",
                base.display()
            )));
        }
        Ok(_) => {
            fs::set_permissions(&base, fs::Permissions::from_mode(0o700))
                .map_err(|error| io_error(&base, error))?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_private_directory(&base)?;
        }
        Err(error) => return Err(io_error(&base, error)),
    }

    let mut sequence = 1_u32;
    loop {
        let attempt = base.join(format!("attempt-{sequence:04}"));
        match fs::create_dir(&attempt) {
            Ok(()) => {
                fs::set_permissions(&attempt, fs::Permissions::from_mode(0o700))
                    .map_err(|error| io_error(&attempt, error))?;
                return fs::canonicalize(&attempt).map_err(|error| io_error(&attempt, error));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                sequence = sequence.checked_add(1).ok_or_else(|| {
                    invalid_configuration("judge evidence attempt sequence overflowed")
                })?;
            }
            Err(error) => return Err(io_error(&attempt, error)),
        }
    }
}

fn create_private_directory(path: &Path) -> Result<(), SkillEvalError> {
    fs::create_dir_all(path).map_err(|error| io_error(path, error))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| io_error(path, error))
}

fn copy_packet_directory(
    source: &Path,
    destination: &Path,
    candidate: &ModelIdentity,
    packet_bytes: &mut u64,
) -> Result<(), SkillEvalError> {
    let mut entries = fs::read_dir(source)
        .map_err(|error| io_error(source, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error(source, error))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    if entries.len() > MAX_JUDGE_DIRECTORY_ENTRIES {
        return Err(invalid_configuration(format!(
            "judge evidence directory {} has too many entries",
            source.display()
        )));
    }
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if is_generated_python_cache(&entry.file_name(), &metadata) {
            continue;
        }
        let name = sanitized_entry_name(&entry.file_name(), candidate)?;
        let target = destination.join(name);
        if target.exists() {
            return Err(invalid_configuration(format!(
                "judge evidence names collide at {}",
                target.display()
            )));
        }
        if metadata.file_type().is_symlink() {
            return Err(invalid_configuration(format!(
                "judge evidence entry {} must not be a symbolic link",
                path.display()
            )));
        }
        if metadata.is_dir() {
            create_private_directory(&target)?;
            copy_packet_directory(&path, &target, candidate, packet_bytes)?;
        } else if metadata.is_file() {
            copy_candidate_file(&path, &target, candidate, packet_bytes)?;
        } else {
            return Err(invalid_configuration(format!(
                "judge evidence entry {} is not a file or directory",
                path.display()
            )));
        }
    }
    Ok(())
}

fn is_generated_python_cache(name: &std::ffi::OsStr, metadata: &fs::Metadata) -> bool {
    metadata.is_dir() && name == "__pycache__"
        || metadata.is_file()
            && Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "pyc")
}

fn sanitized_entry_name(
    name: &std::ffi::OsStr,
    candidate: &ModelIdentity,
) -> Result<String, SkillEvalError> {
    let name = name
        .to_str()
        .ok_or_else(|| invalid_configuration("judge evidence file name is not UTF-8"))?;
    let sanitized = redact_candidate_text(name, candidate);
    if sanitized.is_empty()
        || matches!(sanitized.as_str(), "." | "..")
        || sanitized.contains(['/', '\\'])
    {
        return Err(invalid_configuration(
            "judge evidence file name is not one safe path component",
        ));
    }
    Ok(sanitized)
}

fn reserve_packet_bytes(
    packet_bytes: &mut u64,
    bytes: u64,
    path: &Path,
) -> Result<(), SkillEvalError> {
    if bytes > MAX_JUDGE_FILE_BYTES {
        return Err(invalid_configuration(format!(
            "judge evidence file {} exceeds the size limit",
            path.display()
        )));
    }
    *packet_bytes = packet_bytes
        .checked_add(bytes)
        .ok_or_else(|| invalid_configuration("judge evidence size overflowed"))?;
    if *packet_bytes > MAX_JUDGE_PACKET_BYTES {
        return Err(invalid_configuration(
            "judge evidence packet exceeds the size limit",
        ));
    }
    Ok(())
}

fn copy_candidate_file(
    source: &Path,
    destination: &Path,
    candidate: &ModelIdentity,
    packet_bytes: &mut u64,
) -> Result<(), SkillEvalError> {
    let metadata = fs::metadata(source).map_err(|error| io_error(source, error))?;
    if metadata.len() > MAX_JUDGE_FILE_BYTES {
        return Err(invalid_configuration(format!(
            "judge evidence file {} exceeds the size limit",
            source.display()
        )));
    }
    let bytes = fs::read(source).map_err(|error| io_error(source, error))?;
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(());
    };
    reserve_packet_bytes(packet_bytes, metadata.len(), source)?;
    let sanitized = redact_candidate_text(&text, candidate);
    write_private_file(destination, sanitized.as_bytes())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), SkillEvalError> {
    fs::write(path, bytes).map_err(|error| io_error(path, error))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| io_error(path, error))
}

fn write_sanitized_transcript(
    source: &Path,
    destination: &Path,
    candidate: &ModelIdentity,
    packet_bytes: &mut u64,
) -> Result<(), SkillEvalError> {
    let metadata = fs::metadata(source).map_err(|error| io_error(source, error))?;
    if metadata.len() > MAX_PI_EVENT_BYTES as u64 {
        return Err(invalid_configuration(format!(
            "candidate transcript {} exceeds the size limit",
            source.display()
        )));
    }
    let text = fs::read_to_string(source).map_err(|error| io_error(source, error))?;
    let mut output = String::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut event: Value =
            serde_json::from_str(line).map_err(|error| SkillEvalError::InvalidEvent {
                line: (index + 1) as u64,
                message: format!("candidate transcript is malformed: {error}"),
            })?;
        if matches!(
            event.get("type").and_then(Value::as_str),
            Some(
                "agent_end"
                    | "agent_settled"
                    | "agent_start"
                    | "message_start"
                    | "message_update"
                    | "tool_execution_update"
                    | "turn_end"
                    | "turn_start"
            )
        ) {
            continue;
        }
        remove_identity_fields(&mut event, candidate);
        output.push_str(&serde_json::to_string(&event).map_err(|error| {
            invalid_configuration(format!(
                "sanitized transcript cannot be serialized: {error}"
            ))
        })?);
        output.push('\n');
    }
    reserve_packet_bytes(packet_bytes, output.len() as u64, source)?;
    write_private_file(destination, output.as_bytes())
}

fn remove_identity_fields(value: &mut Value, candidate: &ModelIdentity) {
    match value {
        Value::Array(values) => {
            for value in values {
                remove_identity_fields(value, candidate);
            }
        }
        Value::Object(object) => {
            let is_assistant = object.get("role").and_then(Value::as_str) == Some("assistant");
            if is_assistant {
                for field in [
                    "api",
                    "model",
                    "modelId",
                    "provider",
                    "rawStopReason",
                    "responseId",
                    "responseModel",
                ] {
                    object.remove(field);
                }
            }
            if matches!(
                object.get("type").and_then(Value::as_str),
                Some("thinking" | "toolCall")
            ) {
                object.remove("thinkingSignature");
                object.remove("thoughtSignature");
            }
            for value in object.values_mut() {
                remove_identity_fields(value, candidate);
            }
        }
        Value::String(text) => {
            *text = redact_candidate_text(text, candidate);
        }
        _ => {}
    }
}

fn redact_candidate_text(text: &str, candidate: &ModelIdentity) -> String {
    let mut sanitized = text.to_owned();
    let mut identities = [
        format!("{}/{}", candidate.provider, candidate.model),
        candidate.model.clone(),
        candidate.provider.clone(),
    ];
    identities.sort_by_key(|identity| std::cmp::Reverse(identity.len()));
    for identity in identities {
        sanitized = replace_ascii_case_insensitive(&sanitized, &identity, "[***]");
    }
    sanitized
}

fn replace_ascii_case_insensitive(text: &str, target: &str, replacement: &str) -> String {
    let normalized = text.to_ascii_lowercase();
    let target = target.to_ascii_lowercase();
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(relative) = normalized[cursor..].find(&target) {
        let index = cursor + relative;
        output.push_str(&text[cursor..index]);
        output.push_str(replacement);
        cursor = index + target.len();
    }
    output.push_str(&text[cursor..]);
    output
}

fn locked_read_extension_source(root: &Path) -> Result<String, SkillEvalError> {
    let mut files = BTreeMap::new();
    let mut directories = BTreeMap::new();
    let mut total_bytes = 0_u64;
    collect_locked_snapshot(root, root, &mut files, &mut directories, &mut total_bytes)?;
    let files = serde_json::to_string(&files).map_err(|error| {
        invalid_configuration(format!(
            "judge evidence files cannot be serialized: {error}"
        ))
    })?;
    let directories = serde_json::to_string(&directories).map_err(|error| {
        invalid_configuration(format!(
            "judge evidence directories cannot be serialized: {error}"
        ))
    })?;
    Ok(format!(
        r#"import type {{ ExtensionAPI }} from "@earendil-works/pi-coding-agent";
import {{ Type }} from "typebox";

const FILES: Record<string, string> = {files};
const DIRECTORIES: Record<string, string> = {directories};

function lockedKey(input: string): string {{
  const normalized = input.replaceAll("\\", "/");
  if (normalized.startsWith("/") || /^[A-Za-z]:\//.test(normalized) || normalized.includes("\0")) {{
    throw new Error("read path escapes the locked evidence folder");
  }}
  const parts: string[] = [];
  for (const part of normalized.split("/")) {{
    if (part === "" || part === ".") continue;
    if (part === "..") throw new Error("read path escapes the locked evidence folder");
    parts.push(part);
  }}
  return parts.join("/");
}}

export default function (pi: ExtensionAPI): void {{
  pi.registerTool({{
    name: "submit_verdict",
    label: "Submit verdict",
    description: "Submit the final grading verdict.",
    parameters: Type.Object({{
      score: Type.Integer({{ minimum: 0, maximum: 10 }}),
      is_catastrophic: Type.Boolean(),
      failure_mode: Type.Union([Type.String(), Type.Null()]),
    }}),
    async execute(_id, input) {{
      return {{
        content: [{{ type: "text", text: JSON.stringify(input) }}],
        details: {{ verdict: input }},
        terminate: true,
      }};
    }},
  }});
  pi.registerTool({{
    name: "read",
    label: "Read locked evidence",
    description: "Read a text file or list a directory inside the locked judge evidence folder.",
    parameters: Type.Object({{
      path: Type.String(),
      offset: Type.Optional(Type.Integer({{ minimum: 1 }})),
      limit: Type.Optional(Type.Integer({{ minimum: 1, maximum: 2000 }})),
    }}),
    async execute(_id, input) {{
      const key = lockedKey(input.path);
      const directory = DIRECTORIES[key];
      if (directory !== undefined) {{
        const lines = directory.split("\n");
        const offset = input.offset ?? 1;
        const limit = input.limit ?? 2000;
        const text = lines.slice(offset - 1, offset - 1 + limit).join("\n").slice(0, 50000);
        return {{ content: [{{ type: "text", text }}], details: {{}} }};
      }}
      const file = FILES[key];
      if (file === undefined) throw new Error("read path does not exist in the locked evidence folder");
      const lines = file.split(/\r?\n/).flatMap((line) =>
        line.length === 0 ? [""] : (line.match(/[\s\S]{{1,4000}}/g) ?? []),
      );
      const offset = input.offset ?? 1;
      const limit = input.limit ?? 2000;
      const text = lines.slice(offset - 1, offset - 1 + limit).join("\n").slice(0, 50000);
      return {{ content: [{{ type: "text", text }}], details: {{}} }};
    }},
  }});
}}
"#
    ))
}

fn collect_locked_snapshot(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<String, String>,
    directories: &mut BTreeMap<String, String>,
    total_bytes: &mut u64,
) -> Result<(), SkillEvalError> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| io_error(path, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error(path, error))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    if entries.len() > MAX_JUDGE_DIRECTORY_ENTRIES {
        return Err(invalid_configuration(format!(
            "judge evidence directory {} has too many entries",
            path.display()
        )));
    }
    let key = packet_key(root, path)?;
    let listing = entries
        .iter()
        .map(|entry| {
            entry
                .file_type()
                .map(|kind| {
                    format!(
                        "{}{}",
                        entry.file_name().to_string_lossy(),
                        if kind.is_dir() { "/" } else { "" }
                    )
                })
                .map_err(|error| io_error(&entry.path(), error))
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");
    directories.insert(key, listing);

    for entry in entries {
        let entry_path = entry.path();
        let metadata =
            fs::symlink_metadata(&entry_path).map_err(|error| io_error(&entry_path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(invalid_configuration(format!(
                "judge evidence entry {} must not be a symbolic link",
                entry_path.display()
            )));
        }
        if metadata.is_dir() {
            collect_locked_snapshot(root, &entry_path, files, directories, total_bytes)?;
        } else if metadata.is_file() {
            if metadata.len() > MAX_JUDGE_FILE_BYTES {
                return Err(invalid_configuration(format!(
                    "judge evidence file {} exceeds the size limit",
                    entry_path.display()
                )));
            }
            *total_bytes = total_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| invalid_configuration("judge evidence size overflowed"))?;
            if *total_bytes > MAX_JUDGE_PACKET_BYTES {
                return Err(invalid_configuration(
                    "judge evidence packet exceeds the size limit",
                ));
            }
            let text =
                fs::read_to_string(&entry_path).map_err(|error| io_error(&entry_path, error))?;
            files.insert(packet_key(root, &entry_path)?, text);
        } else {
            return Err(invalid_configuration(format!(
                "judge evidence entry {} is not a file or directory",
                entry_path.display()
            )));
        }
    }
    Ok(())
}

fn packet_key(root: &Path, path: &Path) -> Result<String, SkillEvalError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        invalid_configuration(format!(
            "judge evidence {} is outside {}",
            path.display(),
            root.display()
        ))
    })?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn invalid_configuration(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(message.into())
}

fn recovered_elapsed_milliseconds(
    execution_directory: &Path,
    transcript_path: &Path,
) -> Result<u64, SkillEvalError> {
    let started = fs::metadata(execution_directory)
        .and_then(|metadata| metadata.created())
        .map_err(|error| io_error(execution_directory, error))?;
    let finished = fs::metadata(transcript_path)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| io_error(transcript_path, error))?;
    let elapsed = finished.duration_since(started).map_err(|_| {
        invalid_configuration("frontier judge recovery evidence timestamps are inconsistent")
    })?;
    if elapsed.is_zero() {
        return Err(invalid_configuration(
            "frontier judge recovery elapsed time is zero",
        ));
    }
    let milliseconds = u64::try_from(elapsed.as_millis())
        .map_err(|_| invalid_configuration("frontier judge recovery elapsed time overflows"))?;
    Ok(milliseconds.max(1))
}

fn io_error(path: &Path, error: io::Error) -> SkillEvalError {
    SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

fn pi_arguments(
    model: &ModelIdentity,
    prompt: &str,
    read_extension: Option<&Path>,
) -> Result<Vec<String>, SkillEvalError> {
    let mut arguments = vec![
        "--mode".to_owned(),
        "json".to_owned(),
        "--no-session".to_owned(),
        "--no-skills".to_owned(),
        "--model".to_owned(),
        format!("{}/{}", model.provider, model.model),
        "--thinking".to_owned(),
        model.thinking.clone(),
        "--no-extensions".to_owned(),
        "--no-prompt-templates".to_owned(),
        "--no-themes".to_owned(),
        "--no-context-files".to_owned(),
        "--no-approve".to_owned(),
    ];
    append_pi_auth_extension(&mut arguments, model)?;
    match read_extension {
        Some(extension) => {
            arguments.push("--extension".to_owned());
            arguments.push(extension.to_string_lossy().into_owned());
            arguments.extend(["--tools".to_owned(), "read,submit_verdict".to_owned()]);
        }
        None => arguments.push("--no-tools".to_owned()),
    }
    arguments.push(pi_positional_prompt(prompt));
    Ok(arguments)
}

fn parse_verdict(
    response: &str,
    checks: &[crate::model::CheckResult],
) -> Result<TrialVerdict, SkillEvalError> {
    let response = response.trim();
    let verdict_json = if response.starts_with("```") {
        fenced_verdict(response)?
    } else {
        response
    };
    let verdict: Value = serde_json::from_str(verdict_json)
        .map_err(|error| malformed_verdict(format!("judge verdict is malformed: {error}")))?;
    let object = verdict
        .as_object()
        .ok_or_else(|| malformed_verdict("judge verdict must be a JSON object"))?;
    let score = object
        .get("score")
        .and_then(Value::as_u64)
        .filter(|score| *score <= 10)
        .ok_or_else(|| malformed_verdict("judge score must be an integer from 0 through 10"))?;
    let is_catastrophic = object
        .get("is_catastrophic")
        .and_then(Value::as_bool)
        .ok_or_else(|| malformed_verdict("judge catastrophic flag must be a boolean"))?;
    let failure_mode = match object.get("failure_mode") {
        Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        _ => {
            return Err(malformed_verdict(
                "judge failure mode must be a string or null",
            ));
        }
    };
    if object.len() != 3 {
        return Err(malformed_verdict("judge verdict has unexpected fields"));
    }
    Ok(TrialVerdict {
        score: score as u8,
        is_catastrophic,
        failure_mode,
        checks: checks.to_vec(),
    })
}

fn fenced_verdict(response: &str) -> Result<&str, SkillEvalError> {
    let body = response
        .strip_prefix("```json\n")
        .or_else(|| response.strip_prefix("```\n"))
        .ok_or_else(|| malformed_verdict("judge verdict fence must be JSON or unlabelled"))?;
    let body = body
        .strip_suffix("\n```")
        .ok_or_else(|| malformed_verdict("judge verdict fence is malformed"))?;
    if body.trim().is_empty() {
        return Err(malformed_verdict("judge verdict fence is malformed"));
    }
    Ok(body)
}

fn malformed_verdict(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidEvent {
        line: 0,
        message: message.into(),
    }
}

struct ParsedResponse {
    response: String,
    model: ModelIdentity,
    usage: TrialUsage,
    error_message: Option<String>,
    has_submitted_verdict: bool,
}

fn parse_events(
    bytes: &[u8],
    requested_model: &ModelIdentity,
    elapsed_milliseconds: u64,
) -> Result<ParsedResponse, SkillEvalError> {
    let text = std::str::from_utf8(bytes).map_err(|error| SkillEvalError::InvalidEvent {
        line: 0,
        message: format!("Pi event stream is not UTF-8: {error}"),
    })?;
    let mut final_message = None;
    let mut submitted_verdict = None;
    let mut usage = empty_usage(elapsed_milliseconds);
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event: Value =
            serde_json::from_str(line).map_err(|error| SkillEvalError::InvalidEvent {
                line: (index + 1) as u64,
                message: error.to_string(),
            })?;
        match event.get("type").and_then(Value::as_str) {
            Some("turn_end") => usage.turns = checked_increment(usage.turns, index)?,
            Some("tool_execution_start") => {
                usage.tool_calls = checked_increment(usage.tool_calls, index)?;
            }
            Some("tool_execution_end")
                if event.get("toolName").and_then(Value::as_str) == Some("submit_verdict") =>
            {
                let verdict = event.pointer("/result/details/verdict").ok_or_else(|| {
                    invalid_event(index, "submit_verdict result has no verdict details")
                })?;
                let verdict = serde_json::to_string(verdict).map_err(|error| {
                    invalid_event(index, &format!("submitted verdict is malformed: {error}"))
                })?;
                if submitted_verdict.replace(verdict).is_some() {
                    return Err(invalid_event(
                        index,
                        "judge submitted more than one verdict",
                    ));
                }
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
    let message = final_message.ok_or_else(|| SkillEvalError::InvalidEvent {
        line: 0,
        message: "Pi event stream has no authoritative assistant message_end".to_owned(),
    })?;
    let model = completed_model(&message, requested_model)?;
    let has_submitted_verdict = submitted_verdict.is_some();
    let response = match submitted_verdict {
        Some(verdict) => verdict,
        None => final_text(&message)?,
    };
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
    Ok(ParsedResponse {
        response,
        model,
        usage,
        error_message,
        has_submitted_verdict,
    })
}

fn empty_usage(elapsed_milliseconds: u64) -> TrialUsage {
    TrialUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        turns: 0,
        tool_calls: 0,
        elapsed_milliseconds,
        cost_millionths_of_dollar: 0,
    }
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
    total.cost_millionths_of_dollar = total
        .cost_millionths_of_dollar
        .checked_add((cost * 1_000_000.0).round() as u64)
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
        thinking: requested.thinking.clone(),
    })
}

fn final_text(message: &Value) -> Result<String, SkillEvalError> {
    let content = message
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_event(0, "final assistant message has no content array"))?;
    let mut response = String::new();
    for part in content {
        if part.get("type").and_then(Value::as_str) == Some("text") {
            response.push_str(
                part.get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid_event(0, "text content has no text"))?,
            );
        }
    }
    Ok(response)
}

fn invalid_event(line_index: usize, message: &str) -> SkillEvalError {
    SkillEvalError::InvalidEvent {
        line: (line_index + 1) as u64,
        message: message.to_owned(),
    }
}

fn duration_milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
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
    let standard_error = String::from_utf8_lossy(&output.standard_error);
    match (assistant_error, standard_error.trim()) {
        (Some(message), "") => message.to_owned(),
        (Some(message), standard_error) => format!("{message}\n{standard_error}"),
        (None, standard_error) => standard_error.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::io;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::model::{
        ArtifactName, CandidateArtifact, CaseId, CheckResult, CheckStatus, HarnessIdentity,
        JudgeInput, ModelIdentity, PromptJudgeRequest, SkillEvalError, Tier, TrialKey, TrialUsage,
    };
    use crate::ports::Judge;

    use super::{
        GRADE_TIMEOUT, JUDGE_PACKET_DIRECTORY, JUDGE_TRANSCRIPT_NAME, LOCKED_READ_EXTENSION_NAME,
        MAX_JUDGE_FILE_BYTES, PiJudge, Process, ProcessOutput, ProcessRequest, parse_verdict,
        redact_candidate_text,
    };

    static NEXT_INPUT: AtomicU64 = AtomicU64::new(0);

    #[derive(Default)]
    struct FakeProcess {
        outputs: VecDeque<ProcessOutput>,
        requests: Vec<ProcessRequest>,
    }

    impl FakeProcess {
        fn returning(output: &str) -> Self {
            Self {
                outputs: VecDeque::from([ProcessOutput {
                    exit_code: Some(0),
                    standard_output: output.as_bytes().to_vec(),
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
                .ok_or_else(|| io::Error::other("no fake output"))
        }
    }

    #[test]
    fn identity_redaction_does_not_match_its_own_replacement() {
        let candidate = model_identity(Tier::T2, "candidate", "model", "low");

        let sanitized = redact_candidate_text("CANDIDATE/model candidate response", &candidate);

        assert_eq!(sanitized, "[***] [***] response");
    }

    #[test]
    fn judge_accepts_the_structured_verdict_tool() {
        let output = submitted_verdict_event_stream();
        let mut judge = PiJudge::with_process(FakeProcess::returning(&output));

        let result = judge.grade(&judge_model(), &judge_input()).unwrap();

        assert_eq!(result.verdict.score, 9);
        assert!(!result.verdict.is_catastrophic);
        assert_eq!(result.verdict.failure_mode, None);
        assert_eq!(result.usage.tool_calls, 1);
        assert_eq!(judge.process.requests[0].timeout, GRADE_TIMEOUT);
    }

    #[test]
    fn frontier_recovers_completed_judge_without_launch() {
        let output = submitted_verdict_event_stream();
        let mut judge = PiJudge::with_process(FakeProcess::returning(&output));
        let input = judge_input();
        let result = judge.grade(&judge_model(), &input).unwrap();

        let recovered = judge
            .recover_frontier_grade(&judge_model(), &input)
            .unwrap()
            .unwrap();

        assert_eq!(judge.process.requests.len(), 1);
        assert_eq!(recovered.model, result.model);
        assert_eq!(recovered.verdict, result.verdict);
        let mut expected_usage = result.usage;
        expected_usage.elapsed_milliseconds = recovered.usage.elapsed_milliseconds;
        assert_eq!(recovered.usage, expected_usage);
        assert!(recovered.usage.elapsed_milliseconds > 0);
        assert!(recovered.usage.elapsed_milliseconds < 1_000);
    }

    #[test]
    fn judge_grades_blindly_and_preserves_checks() {
        let output = submitted_verdict_event_stream();
        let mut judge = PiJudge::with_process(FakeProcess::returning(&output));
        let input = judge_input();

        let result = judge.grade(&judge_model(), &input).unwrap();

        assert_eq!(result.verdict.score, 9);
        assert!(!result.verdict.is_catastrophic);
        assert_eq!(result.verdict.failure_mode, None);
        assert_eq!(result.verdict.checks, input.checks);
        assert_eq!(result.model.provider, "judge-provider");
        assert_eq!(result.model.model, "judge-model");
        assert_eq!(result.usage.input_tokens, 1);
        assert_eq!(result.usage.output_tokens, 1);
        assert_eq!(result.usage.cache_read_tokens, 0);
        assert_eq!(result.usage.cache_write_tokens, 0);
        assert_eq!(result.usage.turns, 0);
        assert_eq!(result.usage.tool_calls, 1);
        assert!(result.usage.elapsed_milliseconds < 1_000);
        assert_eq!(result.usage.cost_millionths_of_dollar, 0);
        let request = &judge.process.requests[0];
        let prompt = request.arguments.last().unwrap();
        assert!(!prompt.contains("candidate-secret"));
        assert!(!prompt.contains("candidate-provider"));
        assert!(prompt.contains("\"artifact_path\": \"artifact\""));
        assert_eq!(
            request
                .working_directory
                .parent()
                .unwrap()
                .file_name()
                .unwrap(),
            JUDGE_PACKET_DIRECTORY
        );
        assert!(
            request
                .working_directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("attempt-")
        );
        assert_eq!(
            fs::metadata(&request.working_directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let sanitized =
            fs::read_to_string(request.working_directory.join("transcript.jsonl")).unwrap();
        assert!(!sanitized.contains("candidate-secret"));
        assert!(!sanitized.contains("candidate-provider"));
        assert!(!sanitized.contains("candidate-api"));
        assert!(sanitized.contains("candidate response"));
        assert!(sanitized.contains("\"model\":\"widget\""));
        assert!(sanitized.contains("\"provider\":\"factory\""));
        assert!(sanitized.contains("\"api\":\"v1\""));
        let artifact_entries = fs::read_dir(request.working_directory.join("artifact"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(artifact_entries.len(), 1);
        assert!(
            !artifact_entries[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase()
                .contains("candidate-secret")
        );
        for path in [
            artifact_entries[0].clone(),
            request.working_directory.join("response.txt"),
        ] {
            let text = fs::read_to_string(path).unwrap();
            assert!(text.contains("candidate response"));
            assert!(!text.to_ascii_lowercase().contains("candidate-provider"));
            assert!(!text.to_ascii_lowercase().contains("candidate-secret"));
        }
        for path in ["rubric.md", "expectation.txt", "checks.json"] {
            let text = fs::read_to_string(request.working_directory.join(path)).unwrap();
            assert!(!text.to_ascii_lowercase().contains("candidate-provider"));
            assert!(!text.to_ascii_lowercase().contains("candidate-secret"));
        }
        let extension_index = request
            .arguments
            .iter()
            .rposition(|argument| argument == "--extension")
            .unwrap();
        assert_eq!(
            request.arguments[extension_index + 1],
            request
                .working_directory
                .join(LOCKED_READ_EXTENSION_NAME)
                .to_string_lossy()
        );
    }

    #[test]
    fn judge_drops_duplicate_events_before_enforcing_packet_size() {
        let input = judge_input();
        let duplicate_types = [
            "agent_end",
            "agent_settled",
            "agent_start",
            "message_start",
            "message_update",
            "tool_execution_update",
            "turn_end",
            "turn_start",
        ];
        let mut transcript = String::new();
        for event_type in duplicate_types {
            let event = serde_json::json!({
                "type": event_type,
                "content": "x".repeat(1024),
            });
            for _ in 0..700 {
                transcript.push_str(&serde_json::to_string(&event).unwrap());
                transcript.push('\n');
            }
        }
        for event_type in [
            "session",
            "entry_appended",
            "tool_execution_start",
            "tool_execution_end",
        ] {
            transcript.push_str(
                &serde_json::to_string(&serde_json::json!({"type": event_type})).unwrap(),
            );
            transcript.push('\n');
        }
        transcript.push_str(
            "{\"type\":\"message_end\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"candidate response\"}]}}\n",
        );
        assert!(transcript.len() as u64 > MAX_JUDGE_FILE_BYTES);
        fs::write(&input.candidate.transcript_path, transcript).unwrap();
        let output = submitted_verdict_event_stream();
        let mut judge = PiJudge::with_process(FakeProcess::returning(&output));

        let result = judge.grade(&judge_model(), &input).unwrap();

        assert_eq!(result.verdict.score, 9);
        let sanitized = fs::read_to_string(
            judge.process.requests[0]
                .working_directory
                .join("transcript.jsonl"),
        )
        .unwrap();
        for event_type in duplicate_types {
            assert!(!sanitized.contains(event_type));
        }
        for event_type in [
            "session",
            "entry_appended",
            "tool_execution_start",
            "tool_execution_end",
            "message_end",
        ] {
            assert!(sanitized.contains(event_type));
        }
        assert!(sanitized.contains("candidate response"));
        assert!(sanitized.len() as u64 <= MAX_JUDGE_FILE_BYTES);
    }

    #[test]
    fn judge_rejects_a_symlinked_candidate_entry_before_process_launch() {
        let mut input = judge_input();
        let trial_root = input.candidate.artifact_path.parent().unwrap();
        let fixture = trial_root.join("fixture");
        fs::create_dir(&fixture).unwrap();
        fs::write(fixture.join("safe.txt"), "safe").unwrap();
        fs::write(trial_root.join("response.txt"), "candidate response").unwrap();
        let outside = trial_root.parent().unwrap().join("outside-evidence.txt");
        fs::write(&outside, "outside").unwrap();
        symlink(&outside, fixture.join("escape.txt")).unwrap();
        input.candidate.artifact_path = fixture;
        let mut judge = PiJudge::with_process(FakeProcess::default());

        assert!(matches!(
            judge.grade(&judge_model(), &input),
            Err(SkillEvalError::InvalidConfiguration(_))
        ));
        assert!(judge.process.requests.is_empty());
    }

    #[test]
    fn judge_omits_generated_python_bytecode_from_candidate_evidence() {
        let mut input = judge_input();
        let trial_root = input.candidate.artifact_path.parent().unwrap();
        let fixture = trial_root.join("fixture");
        let cache = fixture.join("__pycache__");
        let git_fixture = fixture.join(".git-fixture");
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&git_fixture).unwrap();
        fs::write(fixture.join("safe.txt"), "candidate response").unwrap();
        fs::write(cache.join("slugify.cpython-314.pyc"), [0xff, 0x00, 0xfe]).unwrap();
        fs::write(git_fixture.join("index"), [0xff, 0x00, 0xfe]).unwrap();
        fs::write(trial_root.join("response.txt"), "candidate response").unwrap();
        input.candidate.artifact_path = fixture;
        let output = submitted_verdict_event_stream();
        let mut judge = PiJudge::with_process(FakeProcess::returning(&output));

        let result = judge.grade(&judge_model(), &input).unwrap();

        assert_eq!(result.verdict.score, 9);
        let artifact = judge.process.requests[0].working_directory.join("artifact");
        assert!(artifact.join("safe.txt").is_file());
        assert!(!artifact.join("__pycache__").exists());
        assert!(!artifact.join(".git-fixture/index").exists());
    }

    #[test]
    fn nested_model_identity_is_passed_to_pi_and_preserves_effective_identity() {
        let output = event_stream("raw", "openrouter", "google/gemini-2.5-flash");
        let mut judge = PiJudge::with_process(FakeProcess::returning(&output));
        let judge_model = model_identity(Tier::T5, "openrouter", "google/gemini-2.5-flash", "high");
        let candidate = model_identity(
            Tier::T2,
            "candidate-provider",
            "anthropic/catalog/claude-sonnet",
            "low",
        );
        let request = PromptJudgeRequest {
            prompt: "legacy prompt".to_owned(),
            candidate_model: Some(candidate),
            timeout_seconds: 7,
        };

        let result = judge.grade_prompt(&judge_model, &request).unwrap();

        let arguments = &judge.process.requests[0].arguments;
        let model_index = arguments
            .iter()
            .position(|value| value == "--model")
            .unwrap();
        assert_eq!(
            arguments[model_index + 1],
            "openrouter/google/gemini-2.5-flash"
        );
        assert_eq!(result.model, judge_model);
        assert_eq!(result.usage.input_tokens, 1);
        assert_eq!(result.usage.output_tokens, 1);
        assert!(arguments.contains(&"--no-tools".to_owned()));
        assert!(!arguments.contains(&"--tools".to_owned()));
    }

    #[test]
    fn nested_model_identity_rejects_malformed_paths_before_process_launch() {
        let malformed_identities = [
            ("", "model"),
            ("provider/catalog", "model"),
            (".", "model"),
            ("..", "model"),
            ("provider name", "model"),
            ("provider\u{7}", "model"),
            ("provider", ""),
            ("provider", "/google/gemini"),
            ("provider", "google/gemini/"),
            ("provider", "google//gemini"),
            ("provider", "google/./gemini"),
            ("provider", "google/../gemini"),
            ("provider", "google/gem ini"),
            ("provider", "google/gem\u{7}ini"),
        ];

        for (provider, model) in malformed_identities {
            for is_candidate in [false, true] {
                let mut judge = PiJudge::with_process(FakeProcess::default());
                let mut judge_model = judge_model();
                let mut candidate = candidate_model();
                let malformed = model_identity(Tier::T2, provider, model, "low");
                if is_candidate {
                    candidate = malformed;
                } else {
                    judge_model = malformed;
                }
                let request = PromptJudgeRequest {
                    prompt: "legacy prompt".to_owned(),
                    candidate_model: Some(candidate),
                    timeout_seconds: 7,
                };

                assert!(matches!(
                    judge.grade_prompt(&judge_model, &request),
                    Err(SkillEvalError::InvalidConfiguration(_))
                ));
                assert!(
                    judge.process.requests.is_empty(),
                    "launched Pi for provider {provider:?}, model {model:?}, candidate {is_candidate}"
                );
            }
        }
    }

    #[test]
    fn judge_rejects_self_grade_before_process_launch() {
        let mut judge = PiJudge::with_process(FakeProcess::default());
        let mut input = judge_input();
        input.candidate.model = judge_model();

        assert!(matches!(
            judge.grade(&judge_model(), &input),
            Err(SkillEvalError::JudgeUnavailable { .. })
        ));
        assert!(judge.process.requests.is_empty());
    }

    #[test]
    fn prompt_judge_returns_raw_response_actual_fallback_and_usage() {
        let mut judge = PiJudge::with_process(FakeProcess::returning(include_str!(
            "../tests/fixtures/judge/prompt.jsonl"
        )));
        let request = PromptJudgeRequest {
            prompt: "legacy prompt".to_owned(),
            candidate_model: Some(candidate_model()),
            timeout_seconds: 7,
        };

        let result = judge.grade_prompt(&judge_model(), &request).unwrap();

        assert_eq!(result.response, "raw legacy response");
        assert_eq!(result.model.provider, "fallback-provider");
        assert_eq!(result.model.model, "fallback-judge");
        assert_eq!(result.usage.input_tokens, 11);
        assert_eq!(result.usage.output_tokens, 4);
        assert_eq!(result.usage.turns, 1);
        assert_eq!(result.usage.cost_millionths_of_dollar, 125_000);
        assert_eq!(judge.process.requests[0].timeout.as_secs(), 7);
    }

    #[test]
    fn judge_accepts_mistral_json_fence() {
        let response = "```json\n{\n  \"score\": 8,\n  \"is_catastrophic\": false,\n  \"failure_mode\": null\n}\n```";

        let verdict = parse_verdict(response, &judge_input().checks).unwrap();

        assert_eq!(verdict.score, 8);
        assert!(!verdict.is_catastrophic);
        assert_eq!(verdict.failure_mode, None);
    }

    #[test]
    fn judge_accepts_unlabelled_verdict_fence() {
        let response = "```\n{\"score\":8,\"is_catastrophic\":false,\"failure_mode\":null}\n```";

        assert!(parse_verdict(response, &judge_input().checks).is_ok());
    }

    #[test]
    fn judge_rejects_wrapped_verdicts() {
        let verdict = r#"{"score":8,"is_catastrophic":false,"failure_mode":null}"#;
        for response in [
            format!("Here is the verdict:\n```json\n{verdict}\n```"),
            format!("```json\n{verdict}\n```\nHope this helps."),
            format!("```json\n{verdict}\n```\n```json\n{verdict}\n```"),
            "```json\n\n```".to_owned(),
            format!("```text\n{verdict}\n```"),
            format!("```rust\n{verdict}\n```"),
            format!("```json\n{verdict}\n``` trailing text"),
            format!("```json\n{verdict}\nextra text\n```"),
            format!("{verdict} trailing text"),
        ] {
            assert!(
                matches!(
                    parse_verdict(&response, &judge_input().checks),
                    Err(SkillEvalError::InvalidEvent { line: 0, .. })
                ),
                "accepted wrapped verdict: {response:?}"
            );
        }
    }

    #[test]
    fn judge_rejects_plain_verdict_without_submission_tool() {
        let output = event_stream(
            r#"{"score":8,"is_catastrophic":false,"failure_mode":null}"#,
            "judge-provider",
            "judge-model",
        );
        let mut judge = PiJudge::with_process(FakeProcess::returning(&output));

        assert!(matches!(
            judge.grade(&judge_model(), &judge_input()),
            Err(SkillEvalError::InvalidEvent { .. })
        ));
    }

    #[test]
    fn judge_rejects_malformed_and_out_of_range_verdicts() {
        for response in [
            "not json",
            r#"{"score":11,"is_catastrophic":false,"failure_mode":null}"#,
        ] {
            let output = event_stream(response, "judge-provider", "judge-model");
            let mut judge = PiJudge::with_process(FakeProcess::returning(&output));
            assert!(matches!(
                judge.grade(&judge_model(), &judge_input()),
                Err(SkillEvalError::InvalidEvent { .. })
            ));
        }
    }

    #[test]
    fn judge_rejects_actual_self_grade_after_fallback() {
        let response = r#"{"score":8,"is_catastrophic":false,"failure_mode":null}"#;
        let output = event_stream(response, "candidate-provider", "candidate-secret");
        let mut judge = PiJudge::with_process(FakeProcess::returning(&output));

        assert!(matches!(
            judge.grade(&judge_model(), &judge_input()),
            Err(SkillEvalError::JudgeUnavailable { .. })
        ));
    }

    #[test]
    fn prompt_judge_rejects_actual_self_grade_after_fallback() {
        let output = event_stream("raw", "candidate-provider", "candidate-secret");
        let mut judge = PiJudge::with_process(FakeProcess::returning(&output));
        let request = PromptJudgeRequest {
            prompt: "legacy prompt".to_owned(),
            candidate_model: Some(candidate_model()),
            timeout_seconds: 7,
        };

        assert!(matches!(
            judge.grade_prompt(&judge_model(), &request),
            Err(SkillEvalError::JudgeUnavailable { .. })
        ));
    }

    #[test]
    fn judge_reports_timeout() {
        let mut process = FakeProcess::returning("");
        process.outputs[0].is_timed_out = true;
        process.outputs[0].exit_code = None;
        let mut judge = PiJudge::with_process(process);

        assert!(matches!(
            judge.grade(&judge_model(), &judge_input()),
            Err(SkillEvalError::Process {
                exit_code: None,
                ..
            })
        ));
    }

    #[test]
    fn judge_reports_quota_and_reset() {
        let mut process = FakeProcess::returning("");
        process.outputs[0].exit_code = Some(1);
        process.outputs[0].standard_error =
            b"usage limit reached; reset_at=2026-08-23T00:00:00-0400".to_vec();
        let mut judge = PiJudge::with_process(process);

        assert!(matches!(
            judge.grade(&judge_model(), &judge_input()),
            Err(SkillEvalError::Quota {
                reset_at: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn judge_retry_preserves_each_evidence_attempt() {
        let input = judge_input();
        let verdict = submitted_verdict_event_stream();
        let process = FakeProcess {
            outputs: VecDeque::from([
                ProcessOutput {
                    exit_code: Some(1),
                    standard_output: Vec::new(),
                    standard_error: b"usage limit reached".to_vec(),
                    is_timed_out: false,
                },
                ProcessOutput {
                    exit_code: Some(0),
                    standard_output: verdict.into_bytes(),
                    standard_error: Vec::new(),
                    is_timed_out: false,
                },
            ]),
            requests: Vec::new(),
        };
        let mut judge = PiJudge::with_process(process);

        assert!(matches!(
            judge.grade(&judge_model(), &input),
            Err(SkillEvalError::Quota { .. })
        ));
        assert!(judge.grade(&judge_model(), &input).is_ok());

        let evidence = input
            .candidate
            .artifact_path
            .parent()
            .unwrap()
            .join(JUDGE_PACKET_DIRECTORY);
        for attempt in ["attempt-0001", "attempt-0002"] {
            assert!(evidence.join(attempt).join(JUDGE_TRANSCRIPT_NAME).is_file());
        }
    }

    #[test]
    fn judge_does_not_treat_verdict_text_as_quota() {
        let output =
            submitted_verdict_event_stream_with(7, Some("candidate mishandled rate_limit_error"));
        let mut judge = PiJudge::with_process(FakeProcess::returning(&output));

        let result = judge.grade(&judge_model(), &judge_input()).unwrap();

        assert_eq!(result.verdict.score, 7);
        assert_eq!(
            result.verdict.failure_mode.as_deref(),
            Some("candidate mishandled rate_limit_error")
        );
    }

    #[test]
    fn judge_treats_an_account_rate_limit_as_quota() {
        let mut process = FakeProcess::returning("");
        process.outputs[0].exit_code = Some(0);
        process.outputs[0].standard_error =
            b"429 {\"error\":{\"type\":\"rate_limit_error\",\"message\":\"This request would exceed your account's rate limit.\"}}".to_vec();
        let mut judge = PiJudge::with_process(process);

        assert!(matches!(
            judge.grade(&judge_model(), &judge_input()),
            Err(SkillEvalError::Quota { .. })
        ));
    }

    #[test]
    fn prompt_judge_reports_timeout() {
        let mut process = FakeProcess::returning("");
        process.outputs[0].is_timed_out = true;
        process.outputs[0].exit_code = None;
        let mut judge = PiJudge::with_process(process);

        assert!(matches!(
            judge.grade_prompt(&judge_model(), &prompt_request()),
            Err(SkillEvalError::Process {
                exit_code: None,
                ..
            })
        ));
    }

    #[test]
    fn prompt_judge_reports_quota_and_reset() {
        let mut process = FakeProcess::returning("");
        process.outputs[0].exit_code = Some(1);
        process.outputs[0].standard_error =
            b"usage limit reached; reset_at=2026-08-23T00:00:00-0400".to_vec();
        let mut judge = PiJudge::with_process(process);

        assert!(matches!(
            judge.grade_prompt(&judge_model(), &prompt_request()),
            Err(SkillEvalError::Quota {
                reset_at: Some(_),
                ..
            })
        ));
    }

    fn judge_input() -> JudgeInput {
        let identifier = NEXT_INPUT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "skill-eval-judge-input-{}-{identifier}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let artifact_path = root.join("CANDIDATE-SECRET.txt");
        let transcript_path = root.join("transcript.jsonl");
        let rubric_path = root.join("rubric.md");
        fs::write(
            &artifact_path,
            "candidate response from CANDIDATE-PROVIDER/CANDIDATE-SECRET",
        )
        .unwrap();
        fs::write(
            &transcript_path,
            "{\"type\":\"message_end\",\"message\":{\"role\":\"assistant\",\"provider\":\"candidate-provider\",\"model\":\"candidate-secret\",\"api\":\"candidate-api\",\"domain\":{\"model\":\"widget\",\"provider\":\"factory\",\"api\":\"v1\"},\"content\":[{\"type\":\"text\",\"text\":\"candidate response\"},{\"type\":\"text\",\"text\":\"CANDIDATE-PROVIDER/CANDIDATE-SECRET\"}]}}\n",
        )
        .unwrap();
        fs::write(&rubric_path, "grade CANDIDATE-PROVIDER fairly").unwrap();
        JudgeInput {
            candidate: CandidateArtifact {
                key: TrialKey {
                    artifact: ArtifactName("artifact".to_owned()),
                    tier: Tier::T2,
                    route_index: 0,
                    case: CaseId("case".to_owned()),
                    attempt: 1,
                },
                model: candidate_model(),
                harness: HarnessIdentity {
                    runner_version: "1".to_owned(),
                    pi_version: "1".to_owned(),
                    artifact_revision: "candidate".to_owned(),
                    tool_policy_digest: "digest".to_owned(),
                },
                artifact_path,
                transcript_path,
                usage: usage(),
            },
            expect: "correct answer from CANDIDATE-SECRET".to_owned(),
            rubric_path,
            checks: vec![CheckResult {
                name: "fixture".to_owned(),
                status: CheckStatus::Failed,
                detail: Some("wrong result from CANDIDATE-PROVIDER".to_owned()),
            }],
        }
    }

    fn prompt_request() -> PromptJudgeRequest {
        PromptJudgeRequest {
            prompt: "legacy prompt".to_owned(),
            candidate_model: Some(candidate_model()),
            timeout_seconds: 7,
        }
    }

    fn candidate_model() -> ModelIdentity {
        model_identity(Tier::T2, "candidate-provider", "candidate-secret", "low")
    }

    fn judge_model() -> ModelIdentity {
        model_identity(Tier::T5, "judge-provider", "judge-model", "high")
    }

    fn model_identity(tier: Tier, provider: &str, model: &str, thinking: &str) -> ModelIdentity {
        ModelIdentity {
            tier,
            provider: provider.to_owned(),
            model: model.to_owned(),
            thinking: thinking.to_owned(),
        }
    }

    fn usage() -> TrialUsage {
        TrialUsage {
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            turns: 1,
            tool_calls: 0,
            elapsed_milliseconds: 1,
            cost_millionths_of_dollar: 1,
        }
    }

    fn submitted_verdict_event_stream() -> String {
        submitted_verdict_event_stream_with(9, None)
    }

    fn submitted_verdict_event_stream_with(score: u8, failure_mode: Option<&str>) -> String {
        let message = serde_json::json!({
            "type": "message_end",
            "message": {
                "role": "assistant",
                "provider": "judge-provider",
                "model": "judge-model",
                "content": [{"type": "toolCall", "name": "submit_verdict", "arguments": {}}],
                "usage": {
                    "input": 1,
                    "output": 1,
                    "cacheRead": 0,
                    "cacheWrite": 0,
                    "cost": {"total": 0.0}
                },
                "stopReason": "toolUse"
            }
        });
        let start = serde_json::json!({
            "type": "tool_execution_start",
            "toolName": "submit_verdict"
        });
        let end = serde_json::json!({
            "type": "tool_execution_end",
            "toolName": "submit_verdict",
            "result": {
                "details": {
                    "verdict": {
                        "score": score,
                        "is_catastrophic": false,
                        "failure_mode": failure_mode
                    }
                }
            }
        });
        format!("{message}\n{start}\n{end}\n")
    }

    fn event_stream(response: &str, provider: &str, model: &str) -> String {
        let message = serde_json::json!({
            "type": "message_end",
            "message": {
                "role": "assistant",
                "provider": provider,
                "model": model,
                "content": [{"type": "text", "text": response}],
                "usage": {
                    "input": 1,
                    "output": 1,
                    "cacheRead": 0,
                    "cacheWrite": 0,
                    "cost": {"total": 0.0}
                },
                "stopReason": "stop"
            }
        });
        format!("{message}\n")
    }
}
