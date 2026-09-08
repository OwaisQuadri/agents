use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const USAGE: &str = "usage: skill-eval --eval-dir <artifact/evals> [--holdout] [--tier Tn] [--jobs N] [--restart] [--accept-if-winning] [candidate]";
const STATE_FORMAT_VERSION: u8 = 2;
const MAX_JOBS: usize = 16;

#[derive(Clone, Debug)]
struct Args {
    eval_dir: PathBuf,
    is_holdout_only: bool,
    tier: Option<String>,
    candidate: Option<PathBuf>,
    is_accept_if_winning: bool,
    jobs: Option<usize>,
    is_restart: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct Case {
    id: String,
    input: Value,
    expect: String,
    #[serde(rename = "holdout")]
    is_holdout: bool,
    #[serde(default)]
    files: Vec<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct TiersFile {
    tiers: Map<String, Value>,
}

#[derive(Clone, Debug)]
struct Settings {
    args: Args,
    repeats: usize,
    cases_file: PathBuf,
    tiers_file: PathBuf,
    tier_dispatch_bin: PathBuf,
    auth_extension: PathBuf,
    jobs: usize,
}

#[derive(Debug)]
struct DispatchResult {
    kind: DispatchKind,
    stdout: String,
    model_ran: Option<String>,
    elapsed_ms: u64,
    attempts: Vec<AttemptRecord>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct AttemptRecord {
    model: String,
    thinking: String,
    elapsed_ms: Option<u64>,
    result: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct RepeatTiming {
    generation_ms: Option<u64>,
    judge_ms: Option<u64>,
    requested_tier: String,
    final_model: Option<String>,
    attempts: Vec<AttemptRecord>,
    judge_requested_tier: String,
    judge_final_model: Option<String>,
    judge_attempts: Vec<AttemptRecord>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct CaseTiming {
    total_ms: Option<u64>,
    repeats: Vec<RepeatTiming>,
}

#[derive(Debug, PartialEq, Eq)]
enum DispatchKind {
    Success,
    Exhausted,
    Failed,
}

#[derive(Clone, Debug)]
struct SliceResult {
    scores: Vec<Option<f64>>,
    repeats: BTreeMap<String, Vec<Option<u8>>>,
    models: BTreeSet<String>,
    case_timings: BTreeMap<String, CaseTiming>,
}

#[derive(Clone, Debug)]
struct TierResult {
    nonholdout: SliceResult,
    holdout: SliceResult,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FrontierEntry {
    #[serde(default)]
    candidate_id: String,
    #[serde(default)]
    tested_against: String,
    #[serde(default)]
    tier: String,
    #[serde(default)]
    judge_tier: String,
    #[serde(default)]
    model_ran: Vec<String>,
    #[serde(default)]
    scores_nonholdout: Vec<Option<f64>>,
    #[serde(default)]
    scores_holdout: Vec<Option<f64>>,
    #[serde(default)]
    repeat_scores_nonholdout: BTreeMap<String, Vec<Option<u8>>>,
    #[serde(default)]
    repeat_scores_holdout: BTreeMap<String, Vec<Option<u8>>>,
    #[serde(default)]
    case_timings_nonholdout: BTreeMap<String, CaseTiming>,
    #[serde(default)]
    case_timings_holdout: BTreeMap<String, CaseTiming>,
    #[serde(default)]
    mean_nonholdout: Option<f64>,
    #[serde(default, rename = "accepted")]
    is_accepted: bool,
    #[serde(default)]
    ts: String,
    #[serde(default)]
    comparison_id: String,
    #[serde(default)]
    incumbent_id: String,
    #[serde(default)]
    incumbent_model_ran: Vec<String>,
    #[serde(default)]
    incumbent_scores_nonholdout: Vec<Option<f64>>,
    #[serde(default)]
    incumbent_scores_holdout: Vec<Option<f64>>,
    #[serde(default)]
    incumbent_repeat_scores_nonholdout: BTreeMap<String, Vec<Option<u8>>>,
    #[serde(default)]
    incumbent_repeat_scores_holdout: BTreeMap<String, Vec<Option<u8>>>,
    #[serde(default)]
    incumbent_case_timings_nonholdout: BTreeMap<String, CaseTiming>,
    #[serde(default)]
    incumbent_case_timings_holdout: BTreeMap<String, CaseTiming>,
    #[serde(default)]
    selected_minimum_tier: Option<String>,
    #[serde(default)]
    decision: String,
    #[serde(flatten)]
    legacy: Map<String, Value>,
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn create(parent: &Path, prefix: &str) -> Result<Self, String> {
        for attempt in 0..1000_u32 {
            let path = parent.join(format!(
                ".{prefix}-{}-{}-{attempt}",
                std::process::id(),
                now_nanos()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("cannot create {}: {error}", path.display())),
            }
        }
        Err(format!(
            "cannot create temporary directory in {}",
            parent.display()
        ))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct FrontierLock {
    path: PathBuf,
}

struct EvalContext<'a> {
    settings: &'a Settings,
    wrapper: &'a Path,
    temp: &'a TempDir,
    candidate: &'a str,
    rubric: &'a str,
    eval_dir: &'a Path,
    artifact_dir: &'a Path,
}

impl FrontierLock {
    fn acquire(eval_dir: &Path) -> Result<Self, String> {
        let path = eval_dir.join(".skill-eval.lock");
        let started = Instant::now();
        loop {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    if started.elapsed() >= Duration::from_secs(30) {
                        return Err(format!("timed out waiting for {}", path.display()));
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(format!("cannot lock {}: {error}", path.display())),
            }
        }
    }
}

impl Drop for FrontierLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn elapsed_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn parse_args(raw: &[OsString]) -> Result<Args, String> {
    let mut eval_dir = None;
    let mut is_holdout_only = false;
    let mut tier = None;
    let mut candidate = None;
    let mut is_accept_if_winning = false;
    let mut jobs = None;
    let mut is_restart = false;
    let mut index = 0;
    while index < raw.len() {
        match raw[index].to_str() {
            Some("--eval-dir") => {
                index += 1;
                eval_dir = raw.get(index).map(PathBuf::from);
                if eval_dir.is_none() {
                    return Err(format!("--eval-dir needs a value\n{USAGE}"));
                }
            }
            Some("--holdout") => is_holdout_only = true,
            Some("--accept-if-winning") => is_accept_if_winning = true,
            Some("--restart") => is_restart = true,
            Some("--jobs") => {
                index += 1;
                let value = raw
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| format!("--jobs needs a positive integer\n{USAGE}"))?;
                jobs = Some(worker_count(value, "--jobs")?);
            }
            Some("--tier") => {
                index += 1;
                tier = raw
                    .get(index)
                    .and_then(|value| value.to_str())
                    .map(str::to_owned);
                if tier.is_none() {
                    return Err(format!("--tier needs a value\n{USAGE}"));
                }
            }
            Some(flag) if flag.starts_with('-') => {
                return Err(format!("unknown flag {flag}\n{USAGE}"));
            }
            Some(_) => {
                if candidate.is_some() {
                    return Err(format!("only one candidate is allowed\n{USAGE}"));
                }
                candidate = Some(PathBuf::from(&raw[index]));
            }
            None => return Err("arguments must be valid UTF-8".to_string()),
        }
        index += 1;
    }
    if is_accept_if_winning && (is_holdout_only || tier.is_some() || candidate.is_none()) {
        return Err(format!(
            "--accept-if-winning requires one candidate and cannot be combined with --holdout or --tier\n{USAGE}"
        ));
    }
    if (is_restart || jobs.is_some()) && (is_holdout_only || tier.is_some() || candidate.is_none())
    {
        return Err(format!(
            "--jobs and --restart require one full paired candidate run\n{USAGE}"
        ));
    }
    Ok(Args {
        eval_dir: eval_dir.ok_or_else(|| format!("--eval-dir is required\n{USAGE}"))?,
        is_holdout_only,
        tier,
        candidate,
        is_accept_if_winning,
        jobs,
        is_restart,
    })
}

fn env_path(name: &str, default: PathBuf) -> PathBuf {
    env::var_os(name).map_or(default, PathBuf::from)
}

fn worker_count(value: &str, name: &str) -> Result<usize, String> {
    let parsed = value.parse::<usize>().map_err(|_| {
        format!("{name} must be a positive integer no greater than {MAX_JOBS}\n{USAGE}")
    })?;
    if parsed == 0 || parsed > MAX_JOBS {
        return Err(format!(
            "{name} must be a positive integer no greater than {MAX_JOBS}\n{USAGE}"
        ));
    }
    Ok(parsed)
}

fn positive_env_value(name: &str, value: &str) -> Result<usize, String> {
    worker_count(value, name)
}

fn positive_env(name: &str, default: usize) -> Result<usize, String> {
    match env::var(name) {
        Ok(value) => positive_env_value(name, &value),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid UTF-8\n{USAGE}")),
    }
}

fn settings(args: Args) -> Result<Settings, String> {
    let repeats = env::var("REPEATS")
        .unwrap_or_else(|_| "3".to_string())
        .parse::<usize>()
        .map_err(|error| format!("REPEATS must be a positive integer: {error}"))?;
    if repeats == 0 {
        return Err("REPEATS must be a positive integer".to_string());
    }
    let jobs = if args.candidate.is_some() && !args.is_holdout_only && args.tier.is_none() {
        match args.jobs {
            Some(jobs) => jobs,
            None => positive_env("SKILL_EVAL_JOBS", 4)?,
        }
    } else {
        1
    };
    let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    Ok(Settings {
        cases_file: env_path("CASES_FILE", args.eval_dir.join("cases.jsonl")),
        tiers_file: env_path("TIERS_FILE", PathBuf::from("config/model-tiers.json")),
        tier_dispatch_bin: env_path(
            "TIER_DISPATCH_BIN",
            PathBuf::from("tools/tier-dispatch/target/debug/tier-dispatch"),
        ),
        auth_extension: env_path(
            "PI_ANTHROPIC_AUTH_EXTENSION",
            home.join(".pi/agent/extensions/pi-anthropic-auth"),
        ),
        args,
        repeats,
        jobs,
    })
}

fn load_cases(path: &Path) -> Result<Vec<Case>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line)
                .map_err(|error| format!("{} line {}: {error}", path.display(), index + 1))
        })
        .collect()
}

fn load_tiers(path: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let file: TiersFile =
        serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut tiers: Vec<String> = file.tiers.into_iter().map(|(tier, _)| tier).collect();
    tiers.sort_by_key(|tier| {
        tier.strip_prefix('T')
            .and_then(|number| number.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    });
    if tiers.is_empty() {
        return Err(format!("{} has no tiers", path.display()));
    }
    Ok(tiers)
}

fn candidate_path(args: &Args) -> PathBuf {
    args.candidate
        .clone()
        .unwrap_or_else(|| args.eval_dir.join("../SKILL.md"))
}

fn run_preflight(eval_dir: &Path, candidate: &Path, cases_file: &Path) -> Result<(), String> {
    let path = eval_dir.join("preflight.sh");
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot inspect {}: {error}", path.display())),
    };
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(format!("{} is not executable", path.display()));
    }
    let status = Command::new(&path)
        .arg(candidate)
        .current_dir(eval_dir)
        .env("CASES_FILE", cases_file)
        .status()
        .map_err(|error| format!("cannot run {}: {error}", path.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("preflight failed with {status}"))
    }
}

fn run_output_check(eval_dir: &Path, artifact: &str) -> Result<Option<String>, String> {
    let path = eval_dir.join("output-check.sh");
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot inspect {}: {error}", path.display())),
    };
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(format!("{} is not executable", path.display()));
    }
    let mut child = Command::new(&path)
        .current_dir(eval_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot run {}: {error}", path.display()))?;
    child
        .stdin
        .take()
        .ok_or_else(|| format!("cannot open stdin for {}", path.display()))?
        .write_all(artifact.as_bytes())
        .map_err(|error| format!("cannot write to {}: {error}", path.display()))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("cannot wait for {}: {error}", path.display()))?;
    if output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let details = format!("{} {}", stdout.trim(), stderr.trim())
        .trim()
        .to_string();
    Ok(Some(if details.is_empty() {
        format!("{} failed with {}", path.display(), output.status)
    } else {
        details
    }))
}

fn output_check_failure_diagnostic(case_id: &str, tier: &str, details: &str) -> String {
    format!("output check failed for {case_id} on {tier}: {details}")
}

fn write_wrapper(temp: &TempDir, extension: &Path) -> Result<PathBuf, String> {
    if !extension.exists() {
        return Err(format!(
            "pi-anthropic-auth not found at {}",
            extension.display()
        ));
    }
    let path = temp.path.join("pi-minimal");
    fs::write(
        &path,
        "#!/bin/zsh\nexec pi --no-extensions -e \"$PI_ANTHROPIC_AUTH_EXTENSION\" \"$@\"\n",
    )
    .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    let mut permissions = fs::metadata(&path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions)
        .map_err(|error| format!("cannot make {} executable: {error}", path.display()))?;
    Ok(path)
}

fn dispatch(
    settings: &Settings,
    wrapper: &Path,
    tier: &str,
    system_prompt: &Path,
    input: &str,
) -> Result<DispatchResult, String> {
    let started = Instant::now();
    let output = Command::new(&settings.tier_dispatch_bin)
        .arg("--tiers-file")
        .arg(&settings.tiers_file)
        .arg("--tier")
        .arg(tier)
        .arg("--system-prompt-file")
        .arg(system_prompt)
        .arg("--input")
        .arg(input)
        .arg("--dispatch-bin")
        .arg(wrapper)
        .env("PI_ANTHROPIC_AUTH_EXTENSION", &settings.auth_extension)
        .output();
    let elapsed_ms = elapsed_millis(started.elapsed());
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            eprintln!("tier-dispatch failed to start on tier {tier}: {error}");
            return Ok(DispatchResult {
                kind: DispatchKind::Failed,
                stdout: String::new(),
                model_ran: None,
                elapsed_ms,
                attempts: Vec::new(),
            });
        }
    };
    decode_dispatch(tier, output, elapsed_ms)
}

fn decode_dispatch(tier: &str, output: Output, elapsed_ms: u64) -> Result<DispatchResult, String> {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let attempts = stderr
        .lines()
        .filter_map(|line| line.strip_prefix("attempt: "))
        .filter_map(|line| serde_json::from_str::<AttemptRecord>(line).ok())
        .collect();
    match output.status.code() {
        Some(0) => {
            let model_ran = stderr
                .lines()
                .filter_map(|line| line.strip_prefix("model_ran: "))
                .next_back()
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_owned);
            if model_ran.is_none() {
                eprintln!("tier-dispatch on tier {tier} exited 0 without model_ran");
            }
            Ok(DispatchResult {
                kind: if model_ran.is_some() {
                    DispatchKind::Success
                } else {
                    DispatchKind::Failed
                },
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                model_ran,
                elapsed_ms,
                attempts,
            })
        }
        Some(2) => Err(format!(
            "tier-dispatch config or usage error on tier {tier}: {}",
            stderr.trim()
        )),
        Some(3) => Ok(DispatchResult {
            kind: DispatchKind::Exhausted,
            stdout: String::new(),
            model_ran: None,
            elapsed_ms,
            attempts,
        }),
        code => {
            eprintln!(
                "tier-dispatch failed on tier {tier} with exit {}: {}",
                code.map_or_else(|| "signal".to_string(), |value| value.to_string()),
                stderr.trim()
            );
            Ok(DispatchResult {
                kind: DispatchKind::Failed,
                stdout: String::new(),
                model_ran: None,
                elapsed_ms,
                attempts,
            })
        }
    }
}

fn case_input(input: &Value) -> Result<String, String> {
    match input {
        Value::String(text) => Ok(text.clone()),
        value => serde_json::to_string(value)
            .map_err(|error| format!("cannot serialize case input: {error}")),
    }
}

fn prompt_for_case(candidate: &str, artifact_dir: &Path, case: &Case) -> Result<String, String> {
    let mut prompt = candidate.to_string();
    for relative in &case.files {
        let path = artifact_dir.join(relative);
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read case file {}: {error}", path.display()))?;
        prompt.push_str(&format!("\n\n--- {} ---\n{content}", relative.display()));
    }
    Ok(prompt)
}

fn judge_prompt(rubric: &str, case: &Case, artifact: &str) -> Result<String, String> {
    Ok(format!(
        "Grade the actual output from one artifact run. Reply with only a JSON object {{\"score\": <integer 0-10>, \"failure_mode\": <string or null>}}.\n\nRUBRIC:\n{rubric}\n\nCASE INPUT:\n{}\n\nEXPECT:\n{}\n\nACTUAL OUTPUT:\n{artifact}",
        case_input(&case.input)?,
        case.expect
    ))
}

fn parse_score(output: &str) -> Option<u8> {
    let start = output.find('{')?;
    let end = output.rfind('}')?;
    let verdict: Value = serde_json::from_str(&output[start..=end]).ok()?;
    let score = verdict.get("score")?.as_u64()?;
    u8::try_from(score).ok().filter(|score| *score <= 10)
}

fn write_prompt(temp: &TempDir, name: &str, text: &str) -> Result<PathBuf, String> {
    let path = temp.path.join(name);
    fs::write(&path, text).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(path)
}

fn median(scores: &[u8]) -> Option<f64> {
    if scores.is_empty() {
        return None;
    }
    let mut sorted = scores.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        Some((f64::from(sorted[middle - 1]) + f64::from(sorted[middle])) / 2.0)
    } else {
        Some(f64::from(sorted[middle]))
    }
}

fn run_slice(
    context: &EvalContext<'_>,
    tier: &str,
    judge_tier: &str,
    cases: &[&Case],
    slice_name: &str,
) -> Result<SliceResult, String> {
    let empty_prompt = write_prompt(context.temp, "judge.md", "")?;
    let mut result = SliceResult {
        scores: Vec::new(),
        repeats: BTreeMap::new(),
        models: BTreeSet::new(),
        case_timings: BTreeMap::new(),
    };
    let mut ungraded = 0;
    for case in cases {
        let case_started = Instant::now();
        let prompt = prompt_for_case(context.candidate, context.artifact_dir, case)?;
        let prompt_path = write_prompt(
            context.temp,
            &format!("prompt-{tier}-{}.md", candidate_id(&case.id)),
            &prompt,
        )?;
        let input = case_input(&case.input)?;
        let mut repeat_scores = Vec::with_capacity(context.settings.repeats);
        let mut repeat_timings = Vec::with_capacity(context.settings.repeats);
        let mut output_check_failures = 0;
        for _ in 0..context.settings.repeats {
            let actual = dispatch(
                context.settings,
                context.wrapper,
                tier,
                &prompt_path,
                &input,
            )?;
            let mut timing = RepeatTiming {
                generation_ms: Some(actual.elapsed_ms),
                judge_ms: None,
                requested_tier: tier.to_string(),
                final_model: actual.model_ran.clone(),
                attempts: actual.attempts.clone(),
                judge_requested_tier: judge_tier.to_string(),
                judge_final_model: None,
                judge_attempts: Vec::new(),
            };
            if actual.kind != DispatchKind::Success {
                repeat_scores.push(None);
                repeat_timings.push(timing);
                ungraded += 1;
                continue;
            }
            if let Some(model) = actual.model_ran.clone() {
                result.models.insert(model);
            }
            let output_check_failure = run_output_check(context.eval_dir, &actual.stdout)?;
            if let Some(details) = &output_check_failure {
                output_check_failures += 1;
                eprintln!(
                    "{}",
                    output_check_failure_diagnostic(&case.id, tier, details)
                );
            }
            let prompt = judge_prompt(context.rubric, case, &actual.stdout)?;
            let judged = dispatch(
                context.settings,
                context.wrapper,
                judge_tier,
                &empty_prompt,
                &prompt,
            )?;
            timing.judge_ms = Some(judged.elapsed_ms);
            timing.judge_final_model = judged.model_ran.clone();
            timing.judge_attempts = judged.attempts.clone();
            let mut score = if judged.kind == DispatchKind::Success {
                parse_score(&judged.stdout)
            } else {
                None
            };
            if output_check_failure.is_some() {
                score = score.map(|value| value.min(4));
            }
            if score.is_none() {
                ungraded += 1;
            }
            repeat_scores.push(score);
            repeat_timings.push(timing);
        }
        let graded: Vec<u8> = repeat_scores.iter().flatten().copied().collect();
        let case_median = median(&graded);
        let timing = CaseTiming {
            total_ms: Some(elapsed_millis(case_started.elapsed())),
            repeats: repeat_timings,
        };
        println!(
            "{}",
            json!({
                "id": case.id,
                "tier": tier,
                "repeat_scores": repeat_scores,
                "median": case_median,
                "output_check_failures": output_check_failures,
                "total_ms": timing.total_ms,
                "repeats": timing.repeats,
            })
        );
        result.scores.push(case_median);
        result.repeats.insert(case.id.clone(), repeat_scores);
        result.case_timings.insert(case.id.clone(), timing);
    }
    let graded: Vec<f64> = result.scores.iter().flatten().copied().collect();
    if graded.is_empty() {
        if cases.is_empty() {
            eprintln!("tier {tier}: no cases ({slice_name} slice)");
        } else {
            eprintln!(
                "tier {tier}: every case ungraded, {ungraded} ungraded repeats ({slice_name} slice)"
            );
        }
    } else {
        let mean = graded.iter().sum::<f64>() / graded.len() as f64;
        let verdict = if mean >= 5.0 { "PASS" } else { "FAIL" };
        eprintln!(
            "tier {tier}: mean {mean:.2} over {} graded cases, {ungraded} ungraded repeats, {verdict} (>= 5 threshold) ({slice_name} slice)",
            graded.len()
        );
    }
    Ok(result)
}

fn candidate_id(text: &str) -> String {
    format!("{:x}", Sha1::digest(text.as_bytes()))[..8].to_string()
}

fn prompt_version(artifact_dir: &Path) -> String {
    Command::new("git")
        .arg("-C")
        .arg(artifact_dir)
        .args([
            "log",
            "-1",
            "--format=%h",
            "--",
            ".",
            ":(exclude,glob)**/evals/**",
            ":(exclude,glob)**/votes/**",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default()
}

fn timestamp() -> String {
    Command::new("date")
        .arg("+%Y-%m-%dT%H:%M:%S%z")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| now_nanos().to_string())
}

fn mean(scores: &[Option<f64>]) -> Option<f64> {
    let graded: Vec<f64> = scores.iter().flatten().copied().collect();
    if graded.is_empty() {
        None
    } else {
        Some((graded.iter().sum::<f64>() / graded.len() as f64 * 100.0).round() / 100.0)
    }
}

fn score_vector(entry: &FrontierEntry) -> Vec<Option<f64>> {
    entry
        .scores_nonholdout
        .iter()
        .chain(&entry.scores_holdout)
        .copied()
        .collect()
}

fn has_incomplete_scores(entry: &FrontierEntry) -> bool {
    let scores = score_vector(entry);
    scores.is_empty()
        || scores.iter().any(Option::is_none)
        || entry
            .repeat_scores_nonholdout
            .values()
            .chain(entry.repeat_scores_holdout.values())
            .flatten()
            .any(Option::is_none)
}

fn dominates(left: &FrontierEntry, right: &FrontierEntry) -> bool {
    let left_scores = score_vector(left);
    let right_scores = score_vector(right);
    if left_scores.len() != right_scores.len() || left_scores.is_empty() {
        return false;
    }
    let mut is_strict = false;
    for (left, right) in left_scores.iter().zip(right_scores) {
        let (Some(left), Some(right)) = (left, right) else {
            return false;
        };
        if left < &right {
            return false;
        }
        is_strict |= left > &right;
    }
    is_strict
}

fn is_prunable(entry: &FrontierEntry, accepted_comparisons: &BTreeSet<String>) -> bool {
    !entry.is_accepted
        && (entry.comparison_id.is_empty() || !accepted_comparisons.contains(&entry.comparison_id))
}

fn prune(entries: &mut Vec<FrontierEntry>) {
    let tiers: BTreeSet<String> = entries.iter().map(|entry| entry.tier.clone()).collect();
    let accepted_comparisons: BTreeSet<String> = entries
        .iter()
        .filter(|entry| entry.is_accepted && !entry.comparison_id.is_empty())
        .map(|entry| entry.comparison_id.clone())
        .collect();
    for tier in tiers {
        loop {
            let indices: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| (entry.tier == tier).then_some(index))
                .collect();
            let unaccepted = indices
                .iter()
                .filter(|index| is_prunable(&entries[**index], &accepted_comparisons))
                .count();
            if unaccepted <= 20 {
                break;
            }
            let newest = indices
                .iter()
                .rev()
                .copied()
                .find(|index| is_prunable(&entries[*index], &accepted_comparisons))
                .expect("tier has too many prunable entries");
            let remove = indices
                .iter()
                .copied()
                .find(|candidate| {
                    *candidate != newest
                        && is_prunable(&entries[*candidate], &accepted_comparisons)
                        && has_incomplete_scores(&entries[*candidate])
                })
                .or_else(|| {
                    indices.iter().copied().find(|candidate| {
                        *candidate != newest
                            && is_prunable(&entries[*candidate], &accepted_comparisons)
                            && indices.iter().copied().any(|other| {
                                other != *candidate
                                    && dominates(&entries[other], &entries[*candidate])
                            })
                    })
                })
                .or_else(|| {
                    indices.iter().copied().find(|candidate| {
                        *candidate != newest
                            && is_prunable(&entries[*candidate], &accepted_comparisons)
                    })
                });
            let Some(remove) = remove else {
                break;
            };
            if entries[remove].comparison_id.is_empty() {
                entries.remove(remove);
            } else {
                let comparison_id = entries[remove].comparison_id.clone();
                entries.retain(|entry| entry.comparison_id != comparison_id);
            }
        }
    }
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent", path.display()))?;
    let temporary = parent.join(format!(
        ".skill-eval-write-{}-{}",
        std::process::id(),
        now_nanos()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("cannot create {}: {error}", temporary.display()))?;
    file.write_all(content)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))
}

fn read_frontier(path: &Path) -> Result<Vec<FrontierEntry>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            let value: Value = serde_json::from_str(line)
                .map_err(|error| format!("{} line {}: {error}", path.display(), index + 1))?;
            let object = value.as_object().ok_or_else(|| {
                format!("{} line {} must be an object", path.display(), index + 1)
            })?;
            let is_current = [
                "candidate_id",
                "tested_against",
                "scores_nonholdout",
                "scores_holdout",
                "mean_nonholdout",
                "accepted",
                "ts",
            ]
            .iter()
            .all(|field| object.contains_key(*field));
            let is_legacy_mechanical = ["candidate", "slice", "cases", "mean", "date"]
                .iter()
                .all(|field| object.contains_key(*field))
                && (object.contains_key("runner") || object.contains_key("tier"));
            if !is_current && !is_legacy_mechanical {
                return Err(format!(
                    "{} line {} has an unknown or incomplete frontier schema",
                    path.display(),
                    index + 1
                ));
            }
            serde_json::from_value(value)
                .map_err(|error| format!("{} line {}: {error}", path.display(), index + 1))
        })
        .collect()
}

fn update_frontier_locked(
    eval_dir: &Path,
    candidate: &str,
    new_entries: Vec<FrontierEntry>,
) -> Result<(), String> {
    if new_entries.is_empty() {
        return Ok(());
    }
    let frontier_path = eval_dir.join("frontier.jsonl");
    let mut entries = read_frontier(&frontier_path)?;
    entries.extend(new_entries);
    prune(&mut entries);
    let mut serialized = Vec::new();
    for entry in &entries {
        serde_json::to_writer(&mut serialized, entry)
            .map_err(|error| format!("cannot serialize frontier entry: {error}"))?;
        serialized.push(b'\n');
    }
    let snapshot_dir = eval_dir.join("frontier");
    fs::create_dir_all(&snapshot_dir)
        .map_err(|error| format!("cannot create {}: {error}", snapshot_dir.display()))?;
    let id = candidate_id(candidate);
    atomic_write(&snapshot_dir.join(format!("{id}.md")), candidate.as_bytes())?;
    atomic_write(&frontier_path, &serialized)?;
    let referenced: BTreeSet<&str> = entries
        .iter()
        .map(|entry| entry.candidate_id.as_str())
        .collect();
    for item in fs::read_dir(&snapshot_dir)
        .map_err(|error| format!("cannot read {}: {error}", snapshot_dir.display()))?
    {
        let item = item.map_err(|error| format!("cannot read frontier snapshot: {error}"))?;
        let path = item.path();
        if path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !referenced.contains(stem) {
            fs::remove_file(&path)
                .map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
        }
    }
    Ok(())
}

fn update_frontier(
    eval_dir: &Path,
    candidate: &str,
    new_entries: Vec<FrontierEntry>,
) -> Result<(), String> {
    let _lock = FrontierLock::acquire(eval_dir)?;
    update_frontier_locked(eval_dir, candidate, new_entries)
}

fn update_comparison_locked(
    eval_dir: &Path,
    comparison_id: &str,
    selected_tiers: &BTreeSet<String>,
    is_accepted: bool,
    decision: &str,
) -> Result<(), String> {
    let frontier_path = eval_dir.join("frontier.jsonl");
    let mut entries = read_frontier(&frontier_path)?;
    let mut is_found = false;
    for entry in &mut entries {
        if entry.comparison_id != comparison_id {
            continue;
        }
        is_found = true;
        if selected_tiers.contains(&entry.tier) {
            entry.is_accepted = is_accepted;
            entry.decision = decision.to_string();
        } else {
            entry.is_accepted = false;
            entry.decision = "excluded_below_floor".to_string();
        }
    }
    if !is_found {
        return Err(format!(
            "comparison {comparison_id} is missing from the frontier"
        ));
    }
    let mut serialized = Vec::new();
    for entry in &entries {
        serde_json::to_writer(&mut serialized, entry)
            .map_err(|error| format!("cannot serialize frontier entry: {error}"))?;
        serialized.push(b'\n');
    }
    atomic_write(&frontier_path, &serialized)
}

fn run_legacy(mut settings: Settings) -> Result<(), String> {
    let eval_dir = fs::canonicalize(&settings.args.eval_dir).map_err(|error| {
        format!(
            "cannot resolve {}: {error}",
            settings.args.eval_dir.display()
        )
    })?;
    settings.cases_file = fs::canonicalize(&settings.cases_file)
        .map_err(|error| format!("cannot resolve {}: {error}", settings.cases_file.display()))?;
    let candidate_path = candidate_path(&settings.args);
    let candidate_path = if candidate_path.is_absolute() {
        candidate_path
    } else {
        env::current_dir()
            .map_err(|error| format!("cannot read current directory: {error}"))?
            .join(candidate_path)
    };
    let submitted = fs::read_to_string(&candidate_path)
        .map_err(|error| format!("cannot read {}: {error}", candidate_path.display()))?;
    let candidate = normalize_minimum_tier(&submitted)?;
    let rubric_path = eval_dir.join("rubric.md");
    let rubric = fs::read_to_string(&rubric_path)
        .map_err(|error| format!("cannot read {}: {error}", rubric_path.display()))?;
    let cases = load_cases(&settings.cases_file)?;
    let mut case_ids = BTreeSet::new();
    for case in &cases {
        if !case_ids.insert(&case.id) {
            return Err(format!(
                "{} has duplicate case id {}",
                settings.cases_file.display(),
                case.id
            ));
        }
    }
    if !cases.iter().any(|case| !case.is_holdout) {
        return Err(format!(
            "{} has no non-holdout cases",
            settings.cases_file.display()
        ));
    }
    if !cases.iter().any(|case| case.is_holdout) {
        return Err(format!(
            "{} has no holdout cases",
            settings.cases_file.display()
        ));
    }
    run_preflight(&eval_dir, &candidate_path, &settings.cases_file)?;
    let full_tiers = load_tiers(&settings.tiers_file)?;
    let tiers = match &settings.args.tier {
        Some(tier) if full_tiers.contains(tier) => vec![tier.clone()],
        Some(tier) => {
            return Err(format!(
                "unknown tier {tier}; known tiers: {}",
                full_tiers.join(", ")
            ));
        }
        None => full_tiers.clone(),
    };
    let temp = TempDir::create(&env::temp_dir(), "skill-eval")?;
    let wrapper = write_wrapper(&temp, &settings.auth_extension)?;
    let artifact_dir = eval_dir
        .parent()
        .ok_or_else(|| format!("{} has no parent", eval_dir.display()))?;
    let nonholdout: Vec<&Case> = cases.iter().filter(|case| !case.is_holdout).collect();
    let holdout: Vec<&Case> = cases.iter().filter(|case| case.is_holdout).collect();
    let context = EvalContext {
        settings: &settings,
        wrapper: &wrapper,
        temp: &temp,
        candidate: &candidate,
        rubric: &rubric,
        eval_dir: &eval_dir,
        artifact_dir,
    };
    let mut results = BTreeMap::new();
    for tier in tiers {
        let index = full_tiers
            .iter()
            .position(|item| item == &tier)
            .expect("validated tier");
        let judge_tier = full_tiers.get(index + 1).unwrap_or(&tier).clone();
        if settings.args.is_holdout_only {
            run_slice(&context, &tier, &judge_tier, &holdout, "holdout")?;
            continue;
        }
        let nonholdout_result = run_slice(&context, &tier, &judge_tier, &nonholdout, "nonholdout")?;
        let holdout_result = run_slice(&context, &tier, &judge_tier, &holdout, "holdout")?;
        results.insert(
            tier,
            TierResult {
                nonholdout: nonholdout_result,
                holdout: holdout_result,
            },
        );
    }
    if settings.args.is_holdout_only || settings.args.tier.is_some() {
        return Ok(());
    }
    let id = candidate_id(&candidate);
    let tested_against = prompt_version(artifact_dir);
    let ts = timestamp();
    let entries = results
        .into_iter()
        .map(|(tier, result)| {
            let index = full_tiers
                .iter()
                .position(|item| item == &tier)
                .expect("validated tier");
            let judge_tier = full_tiers.get(index + 1).unwrap_or(&tier).clone();
            let models = result
                .nonholdout
                .models
                .union(&result.holdout.models)
                .cloned()
                .collect();
            FrontierEntry {
                candidate_id: id.clone(),
                tested_against: tested_against.clone(),
                tier,
                judge_tier,
                model_ran: models,
                mean_nonholdout: mean(&result.nonholdout.scores),
                scores_nonholdout: result.nonholdout.scores,
                scores_holdout: result.holdout.scores,
                repeat_scores_nonholdout: result.nonholdout.repeats,
                repeat_scores_holdout: result.holdout.repeats,
                case_timings_nonholdout: result.nonholdout.case_timings,
                case_timings_holdout: result.holdout.case_timings,
                is_accepted: false,
                ts: ts.clone(),
                comparison_id: String::new(),
                incumbent_id: String::new(),
                incumbent_model_ran: Vec::new(),
                incumbent_scores_nonholdout: Vec::new(),
                incumbent_scores_holdout: Vec::new(),
                incumbent_repeat_scores_nonholdout: BTreeMap::new(),
                incumbent_repeat_scores_holdout: BTreeMap::new(),
                incumbent_case_timings_nonholdout: BTreeMap::new(),
                incumbent_case_timings_holdout: BTreeMap::new(),
                selected_minimum_tier: None,
                decision: String::new(),
                legacy: Map::new(),
            }
        })
        .collect();
    update_frontier(&eval_dir, &candidate, entries)?;
    eprintln!("candidate_id: {id}");
    Ok(())
}

#[derive(Clone)]
struct PairedArm {
    text: String,
    id: String,
    repeats: usize,
    results: BTreeMap<String, TierResult>,
}
#[derive(Clone, Copy)]
struct Ratio {
    n: i64,
    d: i64,
}
impl Ratio {
    fn cmp(self, other: Self) -> std::cmp::Ordering {
        (i128::from(self.n) * i128::from(other.d)).cmp(&(i128::from(other.n) * i128::from(self.d)))
    }

    fn as_f64(self) -> f64 {
        self.n as f64 / self.d as f64
    }
}
struct Selection {
    start: Option<usize>,
    is_accepted: bool,
    reason: &'static str,
    candidate_nonholdout: Option<Ratio>,
    incumbent_nonholdout: Option<Ratio>,
    candidate_holdout: Option<Ratio>,
    incumbent_holdout: Option<Ratio>,
}

fn trim_line_ending(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}

fn normalize_minimum_tier(text: &str) -> Result<String, String> {
    let mut lines: Vec<&str> = text.split_inclusive('\n').collect();
    if lines
        .first()
        .is_none_or(|line| trim_line_ending(line) != "---")
    {
        return Err("frontmatter must start with ---".to_string());
    }
    let end = lines
        .iter()
        .skip(1)
        .position(|line| trim_line_ending(line) == "---")
        .map(|index| index + 1)
        .ok_or_else(|| "frontmatter must end with ---".to_string())?;
    let mut is_metadata = false;
    let mut metadata_index = None;
    let mut found = 0;
    for (index, line) in lines.iter_mut().enumerate().take(end).skip(1) {
        let value = trim_line_ending(line);
        if !value.is_empty()
            && !value.trim_start().starts_with('#')
            && !value.starts_with(char::is_whitespace)
        {
            is_metadata = false;
        }
        if value == "metadata:" {
            if metadata_index.is_some() {
                return Err("frontmatter has duplicate metadata".to_string());
            }
            metadata_index = Some(index);
            is_metadata = true;
            continue;
        }
        if value.starts_with("metadata:") {
            return Err("metadata must be a mapping".to_string());
        }
        if is_metadata
            && value.trim_start().starts_with("minimum-tier:")
            && !value.starts_with("  minimum-tier:")
        {
            return Err("metadata.minimum-tier must use two-space indentation".to_string());
        }
        if is_metadata && value.starts_with("  minimum-tier:") {
            let floor = value
                .strip_prefix("  minimum-tier:")
                .expect("checked prefix")
                .trim();
            if floor.is_empty() || floor.contains('#') || floor.contains(": ") {
                return Err("metadata.minimum-tier must be one unambiguous scalar".to_string());
            }
            *line = "";
            found += 1;
        }
    }
    if found > 1 {
        return Err("frontmatter has duplicate metadata.minimum-tier".to_string());
    }
    if let Some(index) = metadata_index {
        let block_end = (index + 1..end)
            .find(|item| {
                let value = trim_line_ending(lines[*item]);
                !value.is_empty()
                    && !value.trim_start().starts_with('#')
                    && !value.starts_with(char::is_whitespace)
            })
            .unwrap_or(end);
        if !lines[index + 1..block_end].iter().any(|line| {
            let value = trim_line_ending(line).trim();
            !value.is_empty() && !value.starts_with('#')
        }) {
            lines[index] = "";
        }
    }
    Ok(lines.concat())
}

fn is_minimum_tier_declared(text: &str) -> bool {
    let mut is_metadata = false;
    for line in text.lines().skip(1) {
        if line == "---" {
            break;
        }
        if !line.is_empty()
            && !line.trim_start().starts_with('#')
            && !line.starts_with(char::is_whitespace)
        {
            is_metadata = line == "metadata:";
            continue;
        }
        if is_metadata && line.starts_with("  minimum-tier:") {
            return true;
        }
    }
    false
}

fn set_minimum_tier(submitted: &str, tier: &str) -> Result<String, String> {
    let expected = normalize_minimum_tier(submitted)?;
    let mut lines: Vec<String> = submitted.split_inclusive('\n').map(str::to_owned).collect();
    let end = lines
        .iter()
        .skip(1)
        .position(|line| trim_line_ending(line) == "---")
        .map(|index| index + 1)
        .ok_or_else(|| "frontmatter must end with ---".to_string())?;
    let newline = if submitted
        .split_once('\n')
        .is_some_and(|(first, _)| first.ends_with('\r'))
    {
        "\r\n"
    } else {
        "\n"
    };
    if let Some(metadata) = (1..end).find(|index| trim_line_ending(&lines[*index]) == "metadata:") {
        let block_end = (metadata + 1..end)
            .find(|index| {
                let value = trim_line_ending(&lines[*index]);
                !value.is_empty()
                    && !value.trim_start().starts_with('#')
                    && !value.starts_with(char::is_whitespace)
            })
            .unwrap_or(end);
        if let Some(floor) = (metadata + 1..block_end)
            .find(|index| trim_line_ending(&lines[*index]).starts_with("  minimum-tier:"))
        {
            lines[floor] = format!("  minimum-tier: {tier}{newline}");
        } else {
            lines.insert(metadata + 1, format!("  minimum-tier: {tier}{newline}"));
        }
    } else {
        lines.insert(end, format!("metadata:{newline}"));
        lines.insert(end + 1, format!("  minimum-tier: {tier}{newline}"));
    }
    let result = lines.concat();
    if normalize_minimum_tier(&result)? != expected {
        return Err("setting metadata.minimum-tier changed normalized text".to_string());
    }
    Ok(result)
}
fn exact_mean(scores: &[Option<f64>]) -> Option<Ratio> {
    if scores.is_empty() || scores.iter().any(Option::is_none) {
        return None;
    }
    Some(Ratio {
        n: scores
            .iter()
            .map(|score| (score.unwrap() * 2.0).round() as i64)
            .sum(),
        d: 2 * i64::try_from(scores.len()).ok()?,
    })
}
fn suffix_mean(arm: &PairedArm, tiers: &[String], start: usize, is_holdout: bool) -> Option<Ratio> {
    let values: Option<Vec<Ratio>> = tiers[start..]
        .iter()
        .map(|tier| {
            exact_mean(if is_holdout {
                &arm.results.get(tier)?.holdout.scores
            } else {
                &arm.results.get(tier)?.nonholdout.scores
            })
        })
        .collect();
    let values = values?;
    let denominator = values.iter().fold(1_i64, |value, item| value * item.d);
    Some(Ratio {
        n: values
            .iter()
            .map(|item| item.n * (denominator / item.d))
            .sum::<i64>(),
        d: denominator * i64::try_from(values.len()).ok()?,
    })
}
fn is_complete_slice(slice: &SliceResult, cases: &[&Case], expected_repeats: usize) -> bool {
    slice.scores.len() == cases.len()
        && slice.scores.iter().all(Option::is_some)
        && cases.iter().all(|case| {
            slice.repeats.get(&case.id).is_some_and(|repeats| {
                repeats.len() == expected_repeats && repeats.iter().all(Option::is_some)
            })
        })
}
fn is_new_zero_present(
    candidate: &PairedArm,
    incumbent: &PairedArm,
    tiers: &[String],
    start: usize,
) -> bool {
    tiers[start..].iter().any(|tier| {
        [
            (
                &candidate.results[tier].nonholdout,
                &incumbent.results[tier].nonholdout,
            ),
            (
                &candidate.results[tier].holdout,
                &incumbent.results[tier].holdout,
            ),
        ]
        .into_iter()
        .any(|(candidate, incumbent)| {
            candidate
                .scores
                .iter()
                .zip(&incumbent.scores)
                .any(|(candidate, incumbent)| candidate == &Some(0.0) && incumbent != &Some(0.0))
        })
    })
}
fn select_suffix(
    candidate: &PairedArm,
    incumbent: &PairedArm,
    tiers: &[String],
    nonholdout: &[&Case],
    holdout: &[&Case],
) -> Selection {
    let mut selected = None;
    for start in 0..tiers.len() {
        if let (Some(score), Some(_)) = (
            suffix_mean(candidate, tiers, start, false),
            suffix_mean(incumbent, tiers, start, false),
        ) && selected.is_none_or(|(_, best): (usize, Ratio)| score.cmp(best).is_gt())
        {
            selected = Some((start, score));
        }
    }
    let Some((start, candidate_nonholdout)) = selected else {
        return Selection {
            start: None,
            is_accepted: false,
            reason: "incomplete",
            candidate_nonholdout: None,
            incumbent_nonholdout: None,
            candidate_holdout: None,
            incumbent_holdout: None,
        };
    };
    let incumbent_nonholdout = suffix_mean(incumbent, tiers, start, false);
    let candidate_holdout = suffix_mean(candidate, tiers, start, true);
    let incumbent_holdout = suffix_mean(incumbent, tiers, start, true);
    let is_complete = tiers[start..].iter().all(|tier| {
        is_complete_slice(
            &candidate.results[tier].nonholdout,
            nonholdout,
            candidate.repeats,
        ) && is_complete_slice(&candidate.results[tier].holdout, holdout, candidate.repeats)
            && is_complete_slice(
                &incumbent.results[tier].nonholdout,
                nonholdout,
                incumbent.repeats,
            )
            && is_complete_slice(&incumbent.results[tier].holdout, holdout, incumbent.repeats)
    });
    let reason = if !is_complete {
        "incomplete"
    } else if is_new_zero_present(candidate, incumbent, tiers, start) {
        "new_catastrophic_score"
    } else if !candidate_nonholdout
        .cmp(incumbent_nonholdout.expect("complete suffix has incumbent scores"))
        .is_gt()
    {
        "nonholdout_not_strictly_better"
    } else if !candidate_holdout
        .expect("complete suffix has candidate holdout scores")
        .cmp(incumbent_holdout.expect("complete suffix has incumbent holdout scores"))
        .is_gt()
    {
        "holdout_not_strictly_better"
    } else {
        "accepted"
    };
    Selection {
        start: Some(start),
        is_accepted: reason == "accepted",
        reason,
        candidate_nonholdout: Some(candidate_nonholdout),
        incumbent_nonholdout,
        candidate_holdout,
        incumbent_holdout,
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct WorkUnit {
    arm: String,
    tier: String,
    judge_tier: String,
    slice: String,
    case_id: String,
    repeat: usize,
    prompt_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UnitResult {
    unit: WorkUnit,
    score: Option<u8>,
    actual_model: Option<String>,
    is_output_check_failed: bool,
    #[serde(default)]
    timing: RepeatTiming,
}

#[derive(Deserialize, Serialize)]
struct RunManifest {
    format_version: u8,
    run_key: String,
}

struct RunLock {
    file: File,
}

impl RunLock {
    fn acquire(eval_dir: &Path) -> Result<Self, String> {
        let state_root = eval_dir.join(".skill-eval-state");
        fs::create_dir_all(&state_root)
            .map_err(|error| format!("cannot create {}: {error}", state_root.display()))?;
        let path = state_root.join("run.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        file.try_lock()
            .map_err(|error| run_lock_error(eval_dir, &path, error))?;
        Ok(Self { file })
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn run_lock_error(eval_dir: &Path, path: &Path, error: TryLockError) -> String {
    match error {
        TryLockError::WouldBlock => {
            format!("paired evaluation already runs in {}", eval_dir.display())
        }
        TryLockError::Error(error) => format!(
            "cannot acquire advisory lock {} in {}: {error}",
            path.display(),
            eval_dir.display()
        ),
    }
}

struct WorkerContext<'a> {
    settings: &'a Settings,
    wrapper: &'a Path,
    rubric: &'a str,
    eval_dir: &'a Path,
    cases: &'a [Case],
    prompts_dir: &'a Path,
    judge_prompt: &'a Path,
    output_check_lock: &'a Mutex<()>,
}

fn scoring_file(path: &Path) -> Result<Vec<u8>, String> {
    match fs::read(path) {
        Ok(content) => {
            let mode = fs::metadata(path)
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?
                .permissions()
                .mode();
            Ok([format!("mode:{mode:o}\n").into_bytes(), content].concat())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(b"missing\n".to_vec()),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn append_run_key_input(output: &mut Vec<u8>, label: &[u8], value: &[u8]) {
    output.extend_from_slice(label);
    output.push(0);
    output.extend_from_slice(value.len().to_string().as_bytes());
    output.push(0);
    output.extend_from_slice(value);
    output.push(0);
}

fn run_key_input(path: &Path) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut directory_stack = BTreeSet::new();
    append_run_key_input(&mut output, b"root", b"");
    append_path_to_run_key(path, Path::new(""), &mut output, &mut directory_stack)?;
    Ok(output)
}

fn append_path_to_run_key(
    path: &Path,
    relative: &Path,
    output: &mut Vec<u8>,
    directory_stack: &mut BTreeSet<PathBuf>,
) -> Result<(), String> {
    let metadata = if relative.as_os_str().is_empty() {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    }
    .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    append_run_key_input(output, b"path", relative.as_os_str().as_encoded_bytes());
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path)
            .map_err(|error| format!("cannot read link {}: {error}", path.display()))?;
        append_run_key_input(output, b"link", target.as_os_str().as_encoded_bytes());
        let resolved = match fs::canonicalize(path) {
            Ok(resolved) => resolved,
            Err(_) => return Ok(()),
        };
        if directory_stack.contains(&resolved) {
            append_run_key_input(output, b"link-cycle", b"");
            return Ok(());
        }
        append_run_key_input(output, b"link-target", b"");
        return append_path_to_run_key(&resolved, Path::new(""), output, directory_stack);
    }
    append_run_key_input(
        output,
        b"mode",
        format!("{:o}", metadata.permissions().mode()).as_bytes(),
    );
    if metadata.is_file() {
        append_run_key_input(
            output,
            b"file",
            &fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?,
        );
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(format!("{} is not a file or directory", path.display()));
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("cannot resolve {}: {error}", path.display()))?;
    if !directory_stack.insert(canonical.clone()) {
        append_run_key_input(output, b"link-cycle", b"");
        return Ok(());
    }
    let result = (|| {
        append_run_key_input(output, b"directory", b"");
        let mut entries: Vec<_> = fs::read_dir(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?
            .collect::<Result<_, _>>()
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let is_git_metadata_dir = entry.file_name() == ".git"
                && entry
                    .file_type()
                    .map_err(|error| format!("cannot inspect {}: {error}", entry.path().display()))?
                    .is_dir();
            if !is_git_metadata_dir {
                append_path_to_run_key(
                    &entry.path(),
                    &relative.join(entry.file_name()),
                    output,
                    directory_stack,
                )?;
            }
        }
        Ok(())
    })();
    directory_stack.remove(&canonical);
    result
}

fn update_key(hasher: &mut Sha1, label: &str, value: &[u8]) {
    hasher.update(label.as_bytes());
    hasher.update(b"\0");
    hasher.update(value.len().to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(value);
    hasher.update(b"\0");
}

fn paired_run_key(
    settings: &Settings,
    candidate: &str,
    incumbent: &str,
    submitted: &str,
    cases: &[Case],
    rubric: &str,
    artifact_dir: &Path,
) -> Result<String, String> {
    let mut hasher = Sha1::new();
    update_key(&mut hasher, "format", &[STATE_FORMAT_VERSION]);
    update_key(
        &mut hasher,
        "mode",
        if settings.args.is_accept_if_winning {
            b"accept"
        } else {
            b"dry"
        },
    );
    update_key(&mut hasher, "candidate", candidate.as_bytes());
    update_key(&mut hasher, "incumbent", incumbent.as_bytes());
    update_key(&mut hasher, "submitted", submitted.as_bytes());
    update_key(
        &mut hasher,
        "cases",
        &fs::read(&settings.cases_file)
            .map_err(|error| format!("cannot read {}: {error}", settings.cases_file.display()))?,
    );
    for case in cases {
        for relative in &case.files {
            update_key(
                &mut hasher,
                "attachment-path",
                relative.as_os_str().as_encoded_bytes(),
            );
            let path = artifact_dir.join(relative);
            update_key(
                &mut hasher,
                "attachment",
                &fs::read(&path).map_err(|error| {
                    format!("cannot read case file {}: {error}", path.display())
                })?,
            );
        }
    }
    update_key(&mut hasher, "rubric", rubric.as_bytes());
    update_key(
        &mut hasher,
        "output-check",
        &scoring_file(&settings.args.eval_dir.join("output-check.sh"))?,
    );
    update_key(
        &mut hasher,
        "preflight",
        &scoring_file(&settings.args.eval_dir.join("preflight.sh"))?,
    );
    update_key(
        &mut hasher,
        "tiers",
        &fs::read(&settings.tiers_file)
            .map_err(|error| format!("cannot read {}: {error}", settings.tiers_file.display()))?,
    );
    update_key(
        &mut hasher,
        "tier-dispatch",
        &run_key_input(&settings.tier_dispatch_bin)?,
    );
    update_key(
        &mut hasher,
        "auth-extension",
        &run_key_input(&settings.auth_extension)?,
    );
    update_key(
        &mut hasher,
        "repeats",
        settings.repeats.to_string().as_bytes(),
    );
    Ok(format!("{:x}", hasher.finalize()))
}

fn initialize_state(
    eval_dir: &Path,
    run_key: &str,
    is_restart: bool,
) -> Result<(PathBuf, bool), String> {
    let state_root = eval_dir.join(".skill-eval-state");
    fs::create_dir_all(&state_root)
        .map_err(|error| format!("cannot create {}: {error}", state_root.display()))?;
    let state_dir = state_root.join(run_key);
    if is_restart && state_dir.exists() {
        fs::remove_dir_all(&state_dir)
            .map_err(|error| format!("cannot restart {}: {error}", state_dir.display()))?;
    }
    fs::create_dir_all(&state_dir)
        .map_err(|error| format!("cannot create {}: {error}", state_dir.display()))?;
    let manifest_path = state_dir.join("manifest.json");
    let is_resumed = manifest_path.exists();
    if is_resumed {
        let manifest: RunManifest = serde_json::from_slice(
            &fs::read(&manifest_path)
                .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?,
        )
        .map_err(|error| format!("corrupt state {}: {error}", manifest_path.display()))?;
        if manifest.format_version != STATE_FORMAT_VERSION || manifest.run_key != run_key {
            return Err(format!("corrupt state {}", manifest_path.display()));
        }
    } else {
        for entry in fs::read_dir(&state_dir)
            .map_err(|error| format!("cannot read {}: {error}", state_dir.display()))?
        {
            let entry =
                entry.map_err(|error| format!("cannot read {}: {error}", state_dir.display()))?;
            if !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".skill-eval-write-")
            {
                return Err(format!("corrupt state {}", state_dir.display()));
            }
        }
        let manifest = serde_json::to_vec(&RunManifest {
            format_version: STATE_FORMAT_VERSION,
            run_key: run_key.to_string(),
        })
        .map_err(|error| format!("cannot serialize run manifest: {error}"))?;
        atomic_write(&manifest_path, &manifest)?;
    }
    fs::create_dir_all(state_dir.join("units"))
        .map_err(|error| format!("cannot create state units: {error}"))?;
    fs::create_dir_all(state_dir.join("prompts"))
        .map_err(|error| format!("cannot create state prompts: {error}"))?;
    Ok((state_dir, is_resumed))
}

fn unit_name(unit: &WorkUnit) -> Result<String, String> {
    let bytes =
        serde_json::to_vec(unit).map_err(|error| format!("cannot serialize unit: {error}"))?;
    Ok(format!("{:x}.json", Sha1::digest(bytes)))
}

fn unit_path(state_dir: &Path, unit: &WorkUnit) -> Result<PathBuf, String> {
    Ok(state_dir.join("units").join(unit_name(unit)?))
}

fn ensure_immutable_file(path: &Path, content: &[u8]) -> Result<(), String> {
    match fs::read(path) {
        Ok(existing) if existing == content => Ok(()),
        Ok(_) => Err(format!("corrupt state prompt {}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => atomic_write(path, content),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn prompt_name(arm: &str, case_id: &str) -> String {
    format!("{arm}-{:x}.md", Sha1::digest(case_id.as_bytes()))
}

fn prepare_prompts(
    state_dir: &Path,
    incumbent: &str,
    candidate: &str,
    artifact_dir: &Path,
    cases: &[Case],
) -> Result<(), String> {
    for (arm, text) in [("incumbent", incumbent), ("candidate", candidate)] {
        for case in cases {
            let name = prompt_name(arm, &case.id);
            ensure_immutable_file(
                &state_dir.join("prompts").join(name),
                prompt_for_case(text, artifact_dir, case)?.as_bytes(),
            )?;
        }
    }
    ensure_immutable_file(&state_dir.join("prompts").join("judge.md"), b"")
}

fn make_work_units(tiers: &[String], cases: &[Case], repeats: usize) -> Vec<WorkUnit> {
    let nonholdout: Vec<&Case> = cases.iter().filter(|case| !case.is_holdout).collect();
    let holdout: Vec<&Case> = cases.iter().filter(|case| case.is_holdout).collect();
    let mut units = Vec::new();
    for (index, tier) in tiers.iter().enumerate() {
        let judge_tier = tiers.get(index + 1).unwrap_or(tier);
        for (slice, cases) in [("nonholdout", &nonholdout), ("holdout", &holdout)] {
            for case in cases {
                for repeat in 0..repeats {
                    for arm in ["incumbent", "candidate"] {
                        units.push(WorkUnit {
                            arm: arm.to_string(),
                            tier: tier.clone(),
                            judge_tier: judge_tier.clone(),
                            slice: slice.to_string(),
                            case_id: case.id.clone(),
                            repeat,
                            prompt_name: prompt_name(arm, &case.id),
                        });
                    }
                }
            }
        }
    }
    units
}

fn read_completed_units(
    state_dir: &Path,
    units: &[WorkUnit],
) -> Result<BTreeMap<String, UnitResult>, String> {
    let expected: BTreeSet<String> = units.iter().map(unit_name).collect::<Result<_, _>>()?;
    let units_dir = state_dir.join("units");
    for entry in fs::read_dir(&units_dir)
        .map_err(|error| format!("cannot read {}: {error}", units_dir.display()))?
    {
        let path = entry
            .map_err(|error| format!("cannot read state unit: {error}"))?
            .path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if name.starts_with(".skill-eval-write-") {
            continue;
        }
        if !expected.contains(name) {
            return Err(format!("corrupt state unit {}", path.display()));
        }
    }
    let mut completed = BTreeMap::new();
    for unit in units {
        let name = unit_name(unit)?;
        let path = units_dir.join(&name);
        match fs::read(&path) {
            Ok(content) => {
                let result: UnitResult = serde_json::from_slice(&content)
                    .map_err(|error| format!("corrupt state unit {}: {error}", path.display()))?;
                if result.unit != *unit {
                    return Err(format!("corrupt state unit {}", path.display()));
                }
                completed.insert(name, result);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
        }
    }
    Ok(completed)
}

fn run_work_unit(context: &WorkerContext<'_>, unit: &WorkUnit) -> Result<UnitResult, String> {
    let case = context
        .cases
        .iter()
        .find(|case| case.id == unit.case_id)
        .ok_or_else(|| format!("missing case {}", unit.case_id))?;
    let actual = dispatch(
        context.settings,
        context.wrapper,
        &unit.tier,
        &context.prompts_dir.join(&unit.prompt_name),
        &case_input(&case.input)?,
    )?;
    let mut timing = RepeatTiming {
        generation_ms: Some(actual.elapsed_ms),
        judge_ms: None,
        requested_tier: unit.tier.clone(),
        final_model: actual.model_ran.clone(),
        attempts: actual.attempts.clone(),
        judge_requested_tier: unit.judge_tier.clone(),
        judge_final_model: None,
        judge_attempts: Vec::new(),
    };
    if actual.kind != DispatchKind::Success {
        return Ok(UnitResult {
            unit: unit.clone(),
            score: None,
            actual_model: None,
            is_output_check_failed: false,
            timing,
        });
    }
    let output_check_failure = {
        let _output_check = context
            .output_check_lock
            .lock()
            .map_err(|_| "output check lock poisoned".to_string())?;
        let failure = run_output_check(context.eval_dir, &actual.stdout)?;
        if let Some(details) = &failure {
            eprintln!(
                "{}",
                output_check_failure_diagnostic(&unit.case_id, &unit.tier, details)
            );
        }
        failure
    };
    let judge = dispatch(
        context.settings,
        context.wrapper,
        &unit.judge_tier,
        context.judge_prompt,
        &judge_prompt(context.rubric, case, &actual.stdout)?,
    )?;
    timing.judge_ms = Some(judge.elapsed_ms);
    timing.judge_final_model = judge.model_ran.clone();
    timing.judge_attempts = judge.attempts.clone();
    let mut score = (judge.kind == DispatchKind::Success)
        .then(|| parse_score(&judge.stdout))
        .flatten();
    if output_check_failure.is_some() {
        score = score.map(|value| value.min(4));
    }
    Ok(UnitResult {
        unit: unit.clone(),
        score,
        actual_model: actual.model_ran,
        is_output_check_failed: output_check_failure.is_some(),
        timing,
    })
}

fn progress_line(completed: usize, total: usize, result: &UnitResult) -> String {
    format!(
        "skill-eval: {completed}/{total} {} {} {} {} {} {}",
        result.unit.arm,
        result.unit.tier,
        result.unit.slice,
        result.unit.case_id,
        result.unit.repeat,
        result
            .score
            .map_or("ungraded".to_string(), |score| score.to_string())
    )
}

fn emit_paired_summary(arm: &str, tier: &str, slice_name: &str, result: &SliceResult) {
    let graded: Vec<f64> = result.scores.iter().flatten().copied().collect();
    let ungraded = result
        .repeats
        .values()
        .flatten()
        .filter(|score| score.is_none())
        .count();
    if graded.is_empty() {
        eprintln!(
            "{arm} tier {tier}: every case ungraded, {ungraded} ungraded repeats ({slice_name} slice)"
        );
        return;
    }
    let mean = graded.iter().sum::<f64>() / graded.len() as f64;
    let verdict = if mean >= 5.0 { "PASS" } else { "FAIL" };
    eprintln!(
        "{arm} tier {tier}: mean {mean:.2} over {} graded cases, {ungraded} ungraded repeats, {verdict} (>= 5 threshold) ({slice_name} slice)",
        graded.len()
    );
}

struct WorkQueue {
    pending: std::collections::VecDeque<WorkUnit>,
    is_fatal: bool,
}

fn run_work_units(
    context: &WorkerContext<'_>,
    state_dir: &Path,
    units: &[WorkUnit],
    is_resumed: bool,
) -> Result<BTreeMap<String, UnitResult>, String> {
    let mut completed = read_completed_units(state_dir, units)?;
    let missing: Vec<WorkUnit> = units
        .iter()
        .filter(|unit| !completed.contains_key(&unit_name(unit).expect("serializable unit")))
        .cloned()
        .collect();
    let worker_count = context.settings.jobs.min(missing.len());
    eprintln!(
        "skill-eval: {} paired run: {}/{} units complete, {} workers",
        if is_resumed { "resumed" } else { "new" },
        completed.len(),
        units.len(),
        worker_count
    );
    if missing.is_empty() {
        return Ok(completed);
    }
    let queue = Arc::new(Mutex::new(WorkQueue {
        pending: std::collections::VecDeque::from(missing),
        is_fatal: false,
    }));
    let (sender, receiver) = mpsc::channel();
    let mut worker_error = None;
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let sender = sender.clone();
            scope.spawn(move || {
                loop {
                    let unit = {
                        let mut queue = queue.lock().expect("work queue lock poisoned");
                        if queue.is_fatal {
                            return;
                        }
                        queue.pending.pop_front()
                    };
                    let Some(unit) = unit else {
                        return;
                    };
                    let result = run_work_unit(context, &unit);
                    if result.is_err() {
                        queue.lock().expect("work queue lock poisoned").is_fatal = true;
                    }
                    if sender.send((unit, result)).is_err() {
                        return;
                    }
                }
            });
        }
        drop(sender);
        while let Ok((unit, result)) = receiver.recv() {
            match result {
                Ok(result) => match unit_path(state_dir, &unit).and_then(|path| {
                    let bytes = serde_json::to_vec(&result)
                        .map_err(|error| format!("cannot serialize unit: {error}"))?;
                    atomic_write(&path, &bytes)
                }) {
                    Ok(()) => {
                        let name = unit_name(&unit).expect("serializable unit");
                        completed.insert(name, result.clone());
                        eprintln!("{}", progress_line(completed.len(), units.len(), &result));
                    }
                    Err(error) => {
                        queue.lock().expect("work queue lock poisoned").is_fatal = true;
                        worker_error.get_or_insert(error);
                    }
                },
                Err(error) => {
                    worker_error.get_or_insert(error);
                }
            }
        }
    });
    if let Some(error) = worker_error {
        return Err(error);
    }
    if completed.len() != units.len() {
        return Err("paired evaluation stopped before every unit completed".to_string());
    }
    Ok(completed)
}

fn completed_unit<'a>(
    completed: &'a BTreeMap<String, UnitResult>,
    unit: &WorkUnit,
) -> Result<&'a UnitResult, String> {
    let name = unit_name(unit)?;
    completed.get(&name).ok_or_else(|| {
        format!(
            "incomplete unit {} {} {} {} {}",
            unit.arm, unit.tier, unit.slice, unit.case_id, unit.repeat
        )
    })
}

fn make_unit(
    arm: &str,
    tier: &str,
    judge_tier: &str,
    slice: &str,
    case_id: &str,
    repeat: usize,
) -> WorkUnit {
    WorkUnit {
        arm: arm.to_string(),
        tier: tier.to_string(),
        judge_tier: judge_tier.to_string(),
        slice: slice.to_string(),
        case_id: case_id.to_string(),
        repeat,
        prompt_name: prompt_name(arm, case_id),
    }
}

fn slice_from_units(
    arm: &str,
    tier: &str,
    judge_tier: &str,
    slice: &str,
    cases: &[&Case],
    repeats: usize,
    completed: &BTreeMap<String, UnitResult>,
) -> Result<SliceResult, String> {
    let mut result = SliceResult {
        scores: Vec::new(),
        repeats: BTreeMap::new(),
        models: BTreeSet::new(),
        case_timings: BTreeMap::new(),
    };
    for case in cases {
        let mut repeat_scores = Vec::with_capacity(repeats);
        let mut repeat_timings = Vec::with_capacity(repeats);
        for repeat in 0..repeats {
            let unit = make_unit(arm, tier, judge_tier, slice, &case.id, repeat);
            let unit_result = completed_unit(completed, &unit)?;
            if let Some(model) = &unit_result.actual_model {
                result.models.insert(model.clone());
            }
            repeat_scores.push(unit_result.score);
            repeat_timings.push(unit_result.timing.clone());
        }
        let graded: Vec<u8> = repeat_scores.iter().flatten().copied().collect();
        result.scores.push(median(&graded));
        result.repeats.insert(case.id.clone(), repeat_scores);
        result.case_timings.insert(
            case.id.clone(),
            CaseTiming {
                total_ms: Some(repeat_timings.iter().fold(0_u64, |total, timing| {
                    total
                        .saturating_add(timing.generation_ms.unwrap_or(0))
                        .saturating_add(timing.judge_ms.unwrap_or(0))
                })),
                repeats: repeat_timings,
            },
        );
    }
    Ok(result)
}

fn paired_arm_from_units(
    arm: &str,
    text: &str,
    tiers: &[String],
    nonholdout: &[&Case],
    holdout: &[&Case],
    repeats: usize,
    completed: &BTreeMap<String, UnitResult>,
) -> Result<PairedArm, String> {
    let mut results = BTreeMap::new();
    for (index, tier) in tiers.iter().enumerate() {
        let judge_tier = tiers.get(index + 1).unwrap_or(tier);
        results.insert(
            tier.clone(),
            TierResult {
                nonholdout: slice_from_units(
                    arm,
                    tier,
                    judge_tier,
                    "nonholdout",
                    nonholdout,
                    repeats,
                    completed,
                )?,
                holdout: slice_from_units(
                    arm, tier, judge_tier, "holdout", holdout, repeats, completed,
                )?,
            },
        );
    }
    Ok(PairedArm {
        text: text.to_string(),
        id: candidate_id(text),
        repeats,
        results,
    })
}

fn paired_record_values(
    arm: &str,
    paired: &PairedArm,
    tiers: &[String],
    nonholdout: &[&Case],
    holdout: &[&Case],
    completed: &BTreeMap<String, UnitResult>,
) -> Result<Vec<Value>, String> {
    let mut records = Vec::new();
    for (index, tier) in tiers.iter().enumerate() {
        let judge_tier = tiers.get(index + 1).unwrap_or(tier);
        for (slice, cases, result) in [
            ("nonholdout", nonholdout, &paired.results[tier].nonholdout),
            ("holdout", holdout, &paired.results[tier].holdout),
        ] {
            for (case, median_score) in cases.iter().zip(&result.scores) {
                let mut output_check_failures = 0;
                for repeat in 0..paired.repeats {
                    let unit = make_unit(arm, tier, judge_tier, slice, &case.id, repeat);
                    if completed_unit(completed, &unit)?.is_output_check_failed {
                        output_check_failures += 1;
                    }
                }
                let timing = result
                    .case_timings
                    .get(&case.id)
                    .cloned()
                    .unwrap_or_default();
                records.push(json!({"arm":arm,"id":case.id,"tier":tier,"repeat_scores":result.repeats[&case.id],"median":median_score,"output_check_failures":output_check_failures,"total_ms":timing.total_ms,"repeats":timing.repeats}));
            }
        }
    }
    Ok(records)
}

fn emit_paired_records(
    arm: &str,
    paired: &PairedArm,
    tiers: &[String],
    nonholdout: &[&Case],
    holdout: &[&Case],
    completed: &BTreeMap<String, UnitResult>,
) -> Result<(), String> {
    for record in paired_record_values(arm, paired, tiers, nonholdout, holdout, completed)? {
        println!("{record}");
    }
    Ok(())
}

fn paired_entries(
    candidate: &PairedArm,
    incumbent: &PairedArm,
    tiers: &[String],
    selection: &Selection,
    tested_against: &str,
) -> Vec<FrontierEntry> {
    let comparison_id = format!("{}-{}", candidate.id, now_nanos());
    let ts = timestamp();
    tiers
        .iter()
        .enumerate()
        .map(|(index, tier)| {
            let result = &candidate.results[tier];
            let current = &incumbent.results[tier];
            FrontierEntry {
                candidate_id: candidate.id.clone(),
                tested_against: tested_against.to_string(),
                tier: tier.clone(),
                judge_tier: tiers.get(index + 1).unwrap_or(tier).clone(),
                model_ran: result
                    .nonholdout
                    .models
                    .union(&result.holdout.models)
                    .cloned()
                    .collect(),
                scores_nonholdout: result.nonholdout.scores.clone(),
                scores_holdout: result.holdout.scores.clone(),
                repeat_scores_nonholdout: result.nonholdout.repeats.clone(),
                repeat_scores_holdout: result.holdout.repeats.clone(),
                case_timings_nonholdout: result.nonholdout.case_timings.clone(),
                case_timings_holdout: result.holdout.case_timings.clone(),
                mean_nonholdout: mean(&result.nonholdout.scores),
                is_accepted: false,
                ts: ts.clone(),
                comparison_id: comparison_id.clone(),
                incumbent_id: incumbent.id.clone(),
                incumbent_model_ran: current
                    .nonholdout
                    .models
                    .union(&current.holdout.models)
                    .cloned()
                    .collect(),
                incumbent_scores_nonholdout: current.nonholdout.scores.clone(),
                incumbent_scores_holdout: current.holdout.scores.clone(),
                incumbent_repeat_scores_nonholdout: current.nonholdout.repeats.clone(),
                incumbent_repeat_scores_holdout: current.holdout.repeats.clone(),
                incumbent_case_timings_nonholdout: current.nonholdout.case_timings.clone(),
                incumbent_case_timings_holdout: current.holdout.case_timings.clone(),
                selected_minimum_tier: selection.start.map(|start| tiers[start].clone()),
                decision: if selection.start.is_some_and(|start| index < start) {
                    "excluded_below_floor".to_string()
                } else {
                    selection.reason.to_string()
                },
                legacy: Map::new(),
            }
        })
        .collect()
}
fn run(mut settings: Settings) -> Result<i32, String> {
    if settings.args.candidate.is_none()
        || settings.args.is_holdout_only
        || settings.args.tier.is_some()
    {
        run_legacy(settings)?;
        return Ok(0);
    }
    let eval_dir = fs::canonicalize(&settings.args.eval_dir).map_err(|error| {
        format!(
            "cannot resolve {}: {error}",
            settings.args.eval_dir.display()
        )
    })?;
    settings.cases_file = fs::canonicalize(&settings.cases_file)
        .map_err(|error| format!("cannot resolve {}: {error}", settings.cases_file.display()))?;
    let candidate_path = candidate_path(&settings.args);
    let candidate_path = if candidate_path.is_absolute() {
        candidate_path
    } else {
        env::current_dir()
            .map_err(|error| format!("cannot read current directory: {error}"))?
            .join(candidate_path)
    };
    let submitted = fs::read_to_string(&candidate_path)
        .map_err(|error| format!("cannot read {}: {error}", candidate_path.display()))?;
    run_preflight(&eval_dir, &candidate_path, &settings.cases_file)?;
    let artifact_path = eval_dir.join("../SKILL.md");
    let live = fs::read_to_string(&artifact_path)
        .map_err(|error| format!("cannot read {}: {error}", artifact_path.display()))?;
    let candidate = normalize_minimum_tier(&submitted)?;
    let incumbent = normalize_minimum_tier(&live)?;
    let is_floor_declared = is_minimum_tier_declared(&submitted);
    let cases = load_cases(&settings.cases_file)?;
    let mut case_ids = BTreeSet::new();
    for case in &cases {
        if !case_ids.insert(&case.id) {
            return Err(format!(
                "{} has duplicate case id {}",
                settings.cases_file.display(),
                case.id
            ));
        }
    }
    if !cases.iter().any(|case| !case.is_holdout) {
        return Err(format!(
            "{} has no non-holdout cases",
            settings.cases_file.display()
        ));
    }
    if !cases.iter().any(|case| case.is_holdout) {
        return Err(format!(
            "{} has no holdout cases",
            settings.cases_file.display()
        ));
    }
    let nonholdout: Vec<&Case> = cases.iter().filter(|case| !case.is_holdout).collect();
    let holdout: Vec<&Case> = cases.iter().filter(|case| case.is_holdout).collect();
    let tiers = load_tiers(&settings.tiers_file)?;
    if settings.args.is_accept_if_winning && is_floor_declared {
        for tier in &tiers {
            set_minimum_tier(&submitted, tier)?;
        }
    }
    let rubric = fs::read_to_string(eval_dir.join("rubric.md"))
        .map_err(|error| format!("cannot read rubric.md: {error}"))?;
    let artifact_dir = eval_dir
        .parent()
        .ok_or_else(|| format!("{} has no parent", eval_dir.display()))?;
    let run_key = paired_run_key(
        &settings,
        &candidate,
        &incumbent,
        &submitted,
        &cases,
        &rubric,
        artifact_dir,
    )?;
    let _run_lock = RunLock::acquire(&eval_dir)?;
    let (state_dir, is_resumed) = initialize_state(&eval_dir, &run_key, settings.args.is_restart)?;
    prepare_prompts(&state_dir, &incumbent, &candidate, artifact_dir, &cases)?;
    let temp = TempDir::create(&env::temp_dir(), "skill-eval")?;
    let wrapper = write_wrapper(&temp, &settings.auth_extension)?;
    let prompts_dir = state_dir.join("prompts");
    let judge_prompt = prompts_dir.join("judge.md");
    let output_check_lock = Mutex::new(());
    let worker_context = WorkerContext {
        settings: &settings,
        wrapper: &wrapper,
        rubric: &rubric,
        eval_dir: &eval_dir,
        cases: &cases,
        prompts_dir: &prompts_dir,
        judge_prompt: &judge_prompt,
        output_check_lock: &output_check_lock,
    };
    let units = make_work_units(&tiers, &cases, settings.repeats);
    let completed = run_work_units(&worker_context, &state_dir, &units, is_resumed)?;
    let incumbent = paired_arm_from_units(
        "incumbent",
        &incumbent,
        &tiers,
        &nonholdout,
        &holdout,
        settings.repeats,
        &completed,
    )?;
    let candidate = paired_arm_from_units(
        "candidate",
        &candidate,
        &tiers,
        &nonholdout,
        &holdout,
        settings.repeats,
        &completed,
    )?;
    for tier in &tiers {
        emit_paired_summary(
            "incumbent",
            tier,
            "nonholdout",
            &incumbent.results[tier].nonholdout,
        );
        emit_paired_summary(
            "incumbent",
            tier,
            "holdout",
            &incumbent.results[tier].holdout,
        );
        emit_paired_summary(
            "candidate",
            tier,
            "nonholdout",
            &candidate.results[tier].nonholdout,
        );
        emit_paired_summary(
            "candidate",
            tier,
            "holdout",
            &candidate.results[tier].holdout,
        );
    }
    emit_paired_records(
        "incumbent",
        &incumbent,
        &tiers,
        &nonholdout,
        &holdout,
        &completed,
    )?;
    emit_paired_records(
        "candidate",
        &candidate,
        &tiers,
        &nonholdout,
        &holdout,
        &completed,
    )?;
    let selection = select_suffix(&candidate, &incumbent, &tiers, &nonholdout, &holdout);
    let tested_against = prompt_version(artifact_dir);
    let mut evidence = paired_entries(&candidate, &incumbent, &tiers, &selection, &tested_against);
    if settings.args.is_accept_if_winning && selection.is_accepted {
        let _lock = FrontierLock::acquire(&eval_dir)?;
        let locked_live = fs::read_to_string(&artifact_path)
            .map_err(|error| format!("cannot read {}: {error}", artifact_path.display()))?;
        if normalize_minimum_tier(&locked_live)? != incumbent.text {
            for entry in &mut evidence {
                entry.decision = "stale_incumbent".to_string();
            }
            update_frontier_locked(&eval_dir, &candidate.text, evidence)?;
            return Err("live incumbent changed during comparison".to_string());
        }
        let selected = selection.start.expect("accepted selection has a suffix");
        let selected_tiers: BTreeSet<String> = tiers[selected..].iter().cloned().collect();
        let comparison_id = evidence
            .first()
            .expect("configured tiers produce evidence")
            .comparison_id
            .clone();
        let winner = if is_floor_declared {
            set_minimum_tier(&submitted, &tiers[selected])?
        } else {
            submitted.clone()
        };
        for entry in &mut evidence {
            if selected_tiers.contains(&entry.tier) {
                entry.decision = "pending_acceptance".to_string();
            }
        }
        update_frontier_locked(&eval_dir, &candidate.text, evidence)?;
        if let Err(write_error) = atomic_write(&artifact_path, winner.as_bytes()) {
            let evidence_result = update_comparison_locked(
                &eval_dir,
                &comparison_id,
                &selected_tiers,
                false,
                "live_write_failed",
            );
            return match evidence_result {
                Ok(()) => Err(format!(
                    "cannot write live definition; preserved frontier evidence: {write_error}"
                )),
                Err(frontier_error) => Err(format!(
                    "cannot write live definition ({write_error}); cannot update frontier evidence ({frontier_error})"
                )),
            };
        }
        if let Err(frontier_error) =
            update_comparison_locked(&eval_dir, &comparison_id, &selected_tiers, true, "accepted")
        {
            return match atomic_write(&artifact_path, locked_live.as_bytes()) {
                Ok(()) => Err(format!(
                    "frontier acceptance failed; restored live definition: {frontier_error}"
                )),
                Err(rollback_error) => Err(format!(
                    "frontier acceptance failed ({frontier_error}); live rollback failed ({rollback_error})"
                )),
            };
        }
    } else {
        if selection.is_accepted {
            for entry in &mut evidence {
                if entry.decision == "accepted" {
                    entry.decision = "would_accept".to_string();
                }
            }
        }
        update_frontier(&eval_dir, &candidate.text, evidence)?;
    }
    let selected_minimum_tier = selection.start.map(|start| tiers[start].clone());
    let decision = if selection.is_accepted && !settings.args.is_accept_if_winning {
        "would_accept"
    } else {
        selection.reason
    };
    println!(
        "{}",
        json!({"type":"decision","candidate_id":candidate.id,"incumbent_id":incumbent.id,"selected_minimum_tier":selected_minimum_tier,"decision":decision,"candidate_nonholdout":selection.candidate_nonholdout.map(Ratio::as_f64),"incumbent_nonholdout":selection.incumbent_nonholdout.map(Ratio::as_f64),"candidate_holdout":selection.candidate_holdout.map(Ratio::as_f64),"incumbent_holdout":selection.incumbent_holdout.map(Ratio::as_f64)})
    );
    if selection.reason == "incomplete" {
        return Ok(2);
    }
    if let Err(error) = fs::remove_dir_all(&state_dir) {
        eprintln!(
            "skill-eval: warning: cannot remove completed state {}: {error}",
            state_dir.display()
        );
    }
    if settings.args.is_accept_if_winning && !selection.is_accepted {
        Ok(1)
    } else {
        Ok(0)
    }
}

fn main() -> ExitCode {
    let raw: Vec<OsString> = env::args_os().skip(1).collect();
    match parse_args(&raw).and_then(settings).and_then(run) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(1) => ExitCode::from(1),
        Ok(_) => ExitCode::from(2),
        Err(error) => {
            eprintln!("skill-eval: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_temp(name: &str) -> TempDir {
        TempDir::create(&env::temp_dir(), name).unwrap()
    }

    fn write_executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).unwrap();
    }

    fn fixture(name: &str, fake: &str) -> (TempDir, Settings, PathBuf) {
        let temp = test_temp(name);
        let artifact = temp.path.join("artifact");
        let eval_dir = artifact.join("evals");
        fs::create_dir_all(&eval_dir).unwrap();
        fs::write(
            artifact.join("SKILL.md"),
            "---\nmetadata:\n  minimum-tier: T3\n---\ncandidate text",
        )
        .unwrap();
        fs::write(eval_dir.join("rubric.md"), "Score 0-10").unwrap();
        fs::write(
            eval_dir.join("cases.jsonl"),
            "{\"id\":\"n1\",\"input\":\"plain input\",\"expect\":\"works\",\"holdout\":false}\n{\"id\":\"h1\",\"input\":{\"task\":\"object input\"},\"expect\":\"works\",\"holdout\":true}\n",
        )
        .unwrap();
        let tiers_file = temp.path.join("tiers.json");
        fs::write(&tiers_file, r#"{"tiers":{"T1":{},"T2":{},"T3":{}}}"#).unwrap();
        let extension = temp.path.join("auth");
        fs::create_dir(&extension).unwrap();
        fs::write(extension.join("extension.ts"), "extension").unwrap();
        let dispatch = temp.path.join("tier-dispatch");
        write_executable(&dispatch, fake);
        let args = Args {
            eval_dir: eval_dir.clone(),
            is_holdout_only: false,
            tier: None,
            candidate: None,
            is_accept_if_winning: false,
            jobs: None,
            is_restart: false,
        };
        (
            temp,
            Settings {
                args,
                repeats: 3,
                cases_file: eval_dir.join("cases.jsonl"),
                tiers_file,
                tier_dispatch_bin: dispatch,
                auth_extension: extension,
                jobs: 4,
            },
            eval_dir,
        )
    }

    const FAKE: &str = r#"#!/bin/zsh
set -eu
log=${0:h}/calls
while (( $# )); do
  case "$1" in
    --tier) tier=$2; shift 2 ;;
    --input) input=$2; shift 2 ;;
    --system-prompt-file) prompt=$2; shift 2 ;;
    --dispatch-bin) wrapper=$2; shift 2 ;;
    *) shift 2 ;;
  esac
done
[[ -x "$wrapper" ]]
printf '%s\t%s\t%s\t%s\n' "$tier" "$input" "$prompt" "$wrapper" >> "$log"
cat "$prompt" >> "$log.prompts"
cat "$wrapper" > "$log.wrapper"
if [[ "$input" == 'Grade the actual output'* ]]; then
  print '{"score":8,"failure_mode":null}'
  print -u2 'model_ran: judge-model'
else
  print 'actual output'
  print -u2 "model_ran: actual-$tier"
fi
"#;

    #[test]
    fn minimum_tier_does_not_limit_the_exhaustive_sweep() {
        let (temp, settings, eval_dir) = fixture("exhaustive", FAKE);
        run(settings).unwrap();
        let calls = fs::read_to_string(temp.path.join("calls")).unwrap();
        assert_eq!(
            calls.lines().filter(|line| line.starts_with('T')).count(),
            36
        );
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.starts_with("T1\tplain input"))
                .count(),
            3
        );
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.starts_with("T2\tGrade the actual output"))
                .count(),
            6
        );
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.starts_with("T3\tGrade the actual output"))
                .count(),
            12
        );
        assert!(calls.contains("{\"task\":\"object input\"}"));
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].judge_tier, "T2");
        assert_eq!(entries[2].judge_tier, "T3");
        assert_eq!(entries[1].repeat_scores_nonholdout["n1"], vec![Some(8); 3]);
        assert_eq!(entries[1].model_ran, vec!["actual-T2"]);
        assert!(entries.iter().all(|entry| !entry.is_accepted));
        let wrapper_path = calls.lines().next().unwrap().split('\t').nth(3).unwrap();
        let wrapper_error = fs::read_to_string(wrapper_path).unwrap_err();
        assert_eq!(wrapper_error.kind(), io::ErrorKind::NotFound);
        let wrapper = fs::read_to_string(temp.path.join("calls.wrapper")).unwrap();
        assert!(wrapper.contains("pi --no-extensions -e \"$PI_ANTHROPIC_AUTH_EXTENSION\" \"$@\""));
        assert!(!wrapper.contains("--no-tools"));
    }

    #[test]
    fn narrow_modes_keep_full_judge_order_and_do_not_write_frontier() {
        let (temp, mut settings, eval_dir) = fixture("narrow", FAKE);
        settings.args.tier = Some("T1".to_string());
        settings.args.is_holdout_only = true;
        run(settings).unwrap();
        let calls = fs::read_to_string(temp.path.join("calls")).unwrap();
        assert_eq!(
            calls.lines().filter(|line| line.starts_with('T')).count(),
            6
        );
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.starts_with("T2\tGrade the actual output"))
                .count(),
            3
        );
        assert!(!eval_dir.join("frontier.jsonl").exists());
        assert!(!eval_dir.join("frontier").exists());

        let (_temp, mut settings, eval_dir) = fixture("narrow-full", FAKE);
        settings.args.tier = Some("T1".to_string());
        run(settings).unwrap();
        assert!(!eval_dir.join("frontier.jsonl").exists());
        assert!(!eval_dir.join("frontier").exists());
    }

    #[test]
    fn case_files_are_appended_and_wrapper_has_required_pi_arguments() {
        let (temp, mut settings, _eval_dir) = fixture("files", FAKE);
        let artifact = settings.args.eval_dir.parent().unwrap();
        fs::write(artifact.join("context.txt"), "file sentinel").unwrap();
        fs::write(
            &settings.cases_file,
            "{\"id\":\"n1\",\"input\":\"x\",\"expect\":\"y\",\"holdout\":false,\"files\":[\"context.txt\"]}\n{\"id\":\"h1\",\"input\":\"h\",\"expect\":\"y\",\"holdout\":true}\n",
        )
        .unwrap();
        settings.args.tier = Some("T1".to_string());
        settings.repeats = 1;
        let eval_temp = TempDir::create(&settings.args.eval_dir, "inspect").unwrap();
        let wrapper = write_wrapper(&eval_temp, &settings.auth_extension).unwrap();
        let wrapper_text = fs::read_to_string(&wrapper).unwrap();
        assert!(
            wrapper_text.contains("pi --no-extensions -e \"$PI_ANTHROPIC_AUTH_EXTENSION\" \"$@\"")
        );
        assert!(!wrapper_text.contains("--no-tools"));
        run(settings).unwrap();
        let calls = fs::read_to_string(temp.path.join("calls")).unwrap();
        let prompt_path = calls
            .lines()
            .find(|line| line.starts_with("T1\tx\t"))
            .unwrap()
            .split('\t')
            .nth(2)
            .unwrap();
        assert!(!Path::new(prompt_path).exists());
        let prompts = fs::read_to_string(temp.path.join("calls.prompts")).unwrap();
        assert!(!prompts.contains("minimum-tier: T3"));
        assert!(prompts.contains("candidate text"));
        assert!(prompts.contains("--- context.txt ---\nfile sentinel"));
    }

    #[test]
    fn exhaustion_records_an_ungraded_tier_and_exit_two_aborts() {
        let exhausted = r#"#!/bin/zsh
exit 3
"#;
        let (_temp, settings, eval_dir) = fixture("exhausted", exhausted);
        fs::write(&settings.tiers_file, r#"{"tiers":{"T2":{}}}"#).unwrap();
        run(settings).unwrap();
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tier, "T2");
        assert_eq!(entries[0].repeat_scores_nonholdout["n1"], vec![None; 3]);
        assert_eq!(entries[0].repeat_scores_holdout["h1"], vec![None; 3]);

        let fatal = r#"#!/bin/zsh
print -u2 bad-config
exit 2
"#;
        let (_temp, mut settings, eval_dir) = fixture("fatal", fatal);
        settings.args.tier = Some("T2".to_string());
        let error = run(settings).unwrap_err();
        assert!(error.contains("config or usage error"));
        assert!(!eval_dir.join("frontier.jsonl").exists());
    }

    #[test]
    fn an_unavailable_tier_does_not_remove_any_configured_tier() {
        let fake = r#"#!/bin/zsh
set -eu
while (( $# )); do
  case "$1" in
    --tier) tier=$2; shift 2 ;;
    --input) input=$2; shift 2 ;;
    *) shift 2 ;;
  esac
done
if [[ "$tier" == "T2" ]]; then
  exit 3
elif [[ "$input" == 'Grade the actual output'* ]]; then
  print '{"score":8,"failure_mode":null}'
  print -u2 'model_ran: judge-model'
else
  print actual
  print -u2 "model_ran: actual-$tier"
fi
"#;
        let (_temp, mut settings, eval_dir) = fixture("unavailable-tier", fake);
        settings.repeats = 1;
        run(settings).unwrap();
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.tier.as_str())
                .collect::<Vec<_>>(),
            vec!["T1", "T2", "T3"]
        );
        assert_eq!(entries[0].repeat_scores_nonholdout["n1"], vec![None]);
        assert_eq!(entries[0].model_ran, vec!["actual-T1"]);
        assert_eq!(entries[1].repeat_scores_nonholdout["n1"], vec![None]);
        assert!(entries[1].model_ran.is_empty());
        assert_eq!(entries[2].repeat_scores_nonholdout["n1"], vec![Some(8)]);
    }

    #[test]
    fn late_judge_exhaustion_keeps_prior_scores_and_actual_model() {
        let fake = r#"#!/bin/zsh
set -eu
while (( $# )); do
  case "$1" in
    --tier) tier=$2; shift 2 ;;
    --input) input=$2; shift 2 ;;
    *) shift 2 ;;
  esac
done
if [[ "$input" == 'Grade the actual output'* ]]; then
  count_file=${0:h}/judge-count
  count=0
  [[ -f "$count_file" ]] && count=$(<"$count_file")
  (( count += 1 ))
  print "$count" > "$count_file"
  if (( count > 1 )); then
    exit 3
  fi
  print '{"score":8,"failure_mode":null}'
  print -u2 'model_ran: judge-model'
else
  print actual
  print -u2 "model_ran: actual-$tier"
fi
"#;
        let (_temp, mut settings, eval_dir) = fixture("late-judge-exhaustion", fake);
        fs::write(&settings.tiers_file, r#"{"tiers":{"T1":{},"T2":{}}}"#).unwrap();
        settings.repeats = 3;
        run(settings).unwrap();
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        let tier_one = entries.iter().find(|entry| entry.tier == "T1").unwrap();
        assert_eq!(
            tier_one.repeat_scores_nonholdout["n1"],
            vec![Some(8), None, None]
        );
        assert_eq!(tier_one.model_ran, vec!["actual-T1"]);
        assert!(entries.iter().all(|entry| !entry.is_accepted));
    }

    #[test]
    fn hard_failures_and_bad_judge_json_are_null_repeats() {
        let fake = r#"#!/bin/zsh
set -eu
while (( $# )); do
  case "$1" in
    --input) input=$2; shift 2 ;;
    *) shift 2 ;;
  esac
done
if [[ "$input" == 'plain input' ]]; then
  print -u2 hard-failure
  exit 1
elif [[ "$input" == 'Grade the actual output'* ]]; then
  print not-json
  print -u2 'model_ran: judge'
else
  print actual
  print -u2 'model_ran: actual-model'
fi
"#;
        let (_temp, mut settings, eval_dir) = fixture("nulls", fake);
        fs::write(&settings.tiers_file, r#"{"tiers":{"T1":{}}}"#).unwrap();
        settings.repeats = 1;
        run(settings).unwrap();
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert_eq!(entries[0].repeat_scores_nonholdout["n1"], vec![None]);
        assert_eq!(entries[0].repeat_scores_holdout["h1"], vec![None]);
        assert_eq!(entries[0].model_ran, vec!["actual-model"]);
    }

    #[test]
    fn success_without_model_attribution_becomes_a_null_repeat() {
        let fake = r#"#!/bin/zsh
set -eu
print output-without-attribution
"#;
        let (_temp, mut settings, eval_dir) = fixture("attribution", fake);
        fs::write(&settings.tiers_file, r#"{"tiers":{"T1":{}}}"#).unwrap();
        settings.repeats = 1;
        run(settings).unwrap();
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert_eq!(entries[0].repeat_scores_nonholdout["n1"], vec![None]);
        assert!(entries[0].model_ran.is_empty());
    }

    #[test]
    fn preflight_runs_once_and_failure_stops_before_dispatch() {
        let (temp, settings, eval_dir) = fixture("preflight", FAKE);
        let preflight = eval_dir.join("preflight.sh");
        write_executable(
            &preflight,
            &format!(
                "#!/bin/zsh\nprint -r -- \"$1\" >> {}/preflight-log\nprint -r -- \"$CASES_FILE\" >> {}/preflight-log\nexit 7\n",
                temp.path.display(),
                temp.path.display()
            ),
        );
        let error = run(settings).unwrap_err();
        assert!(error.contains("preflight failed"));
        let preflight_log = fs::read_to_string(temp.path.join("preflight-log")).unwrap();
        assert_eq!(preflight_log.lines().count(), 2);
        let mut lines = preflight_log.lines();
        assert_eq!(
            Path::new(lines.next().unwrap()),
            eval_dir.join("../SKILL.md")
        );
        assert_eq!(
            Path::new(lines.next().unwrap()),
            fs::canonicalize(eval_dir.join("cases.jsonl")).unwrap()
        );
        assert!(!temp.path.join("calls").exists());
    }

    #[test]
    fn cases_require_explicit_nonholdout_and_holdout_slices() {
        let (_temp, settings, _eval_dir) = fixture("case-slices", FAKE);
        fs::write(
            &settings.cases_file,
            "{\"id\":\"n1\",\"input\":\"x\",\"expect\":\"y\"}\n",
        )
        .unwrap();
        let error = run(settings).unwrap_err();
        assert!(error.contains("missing field `holdout`"));

        let (_temp, settings, eval_dir) = fixture("missing-holdout", FAKE);
        fs::write(
            &settings.cases_file,
            "{\"id\":\"n1\",\"input\":\"x\",\"expect\":\"y\",\"holdout\":false}\n",
        )
        .unwrap();
        let error = run(settings).unwrap_err();
        assert!(error.contains("has no holdout cases"));
        assert!(!eval_dir.join("frontier.jsonl").exists());

        let (_temp, settings, _eval_dir) = fixture("duplicate-id", FAKE);
        fs::write(
            &settings.cases_file,
            "{\"id\":\"same\",\"input\":\"x\",\"expect\":\"y\",\"holdout\":false}\n{\"id\":\"same\",\"input\":\"h\",\"expect\":\"y\",\"holdout\":true}\n",
        )
        .unwrap();
        let error = run(settings).unwrap_err();
        assert!(error.contains("duplicate case id same"));
    }

    #[test]
    fn non_executable_checks_are_errors() {
        let (_temp, settings, eval_dir) = fixture("non-executable", FAKE);
        fs::write(eval_dir.join("preflight.sh"), "exit 0\n").unwrap();
        let error = run(settings).unwrap_err();
        assert!(error.contains("preflight.sh is not executable"));
    }

    #[test]
    fn output_check_caps_a_judged_repeat_at_four() {
        let (_temp, mut settings, eval_dir) = fixture("output-check", FAKE);
        write_executable(&eval_dir.join("output-check.sh"), "#!/bin/zsh\nexit 1\n");
        fs::write(&settings.tiers_file, r#"{"tiers":{"T1":{}}}"#).unwrap();
        settings.repeats = 1;
        run(settings).unwrap();
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert_eq!(entries[0].repeat_scores_nonholdout["n1"], vec![Some(4)]);
        assert_eq!(entries[0].repeat_scores_holdout["h1"], vec![Some(4)]);
    }

    #[test]
    fn output_check_failure_diagnostic_names_the_paired_case_and_tier() {
        assert_eq!(
            output_check_failure_diagnostic("case/with spaces", "T3", "script detail"),
            "output check failed for case/with spaces on T3: script detail"
        );
    }

    #[test]
    fn legacy_frontier_entries_remain_readable() {
        let temp = test_temp("legacy-frontier");
        let path = temp.path.join("frontier.jsonl");
        fs::write(
            &path,
            "{\"candidate\":\"current\",\"runner\":\"mechanical\",\"slice\":\"nonholdout\",\"cases\":17,\"mean\":5.0,\"date\":\"2026-09-03\"}\n{\"candidate\":\"current\",\"tier\":\"T3\",\"slice\":\"holdout\",\"cases\":1,\"mean\":5.0,\"date\":\"2026-09-03\"}\n",
        )
        .unwrap();
        let entries = read_frontier(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].candidate_id.is_empty());
        assert!(entries[0].tier.is_empty());
        assert_eq!(entries[0].legacy["candidate"], "current");
        assert_eq!(entries[1].tier, "T3");

        fs::write(&path, "{\"candidate_id\":\"truncated\"}\n").unwrap();
        let error = read_frontier(&path).unwrap_err();
        assert!(error.contains("unknown or incomplete frontier schema"));

        fs::write(
            &path,
            "{\"candidate\":\"current\",\"slice\":\"holdout\",\"cases\":1,\"mean\":5.0,\"date\":\"2026-09-03\"}\n",
        )
        .unwrap();
        let error = read_frontier(&path).unwrap_err();
        assert!(error.contains("unknown or incomplete frontier schema"));
    }

    fn entry(id: usize, tier: &str, score: f64) -> FrontierEntry {
        FrontierEntry {
            candidate_id: format!("id{id}"),
            tested_against: "base".to_string(),
            tier: tier.to_string(),
            judge_tier: tier.to_string(),
            model_ran: vec!["model".to_string()],
            scores_nonholdout: vec![Some(score)],
            scores_holdout: vec![Some(score)],
            repeat_scores_nonholdout: BTreeMap::new(),
            repeat_scores_holdout: BTreeMap::new(),
            case_timings_nonholdout: BTreeMap::new(),
            case_timings_holdout: BTreeMap::new(),
            mean_nonholdout: Some(score),
            is_accepted: false,
            ts: id.to_string(),
            comparison_id: String::new(),
            incumbent_id: String::new(),
            incumbent_model_ran: Vec::new(),
            incumbent_scores_nonholdout: Vec::new(),
            incumbent_scores_holdout: Vec::new(),
            incumbent_repeat_scores_nonholdout: BTreeMap::new(),
            incumbent_repeat_scores_holdout: BTreeMap::new(),
            incumbent_case_timings_nonholdout: BTreeMap::new(),
            incumbent_case_timings_holdout: BTreeMap::new(),
            selected_minimum_tier: None,
            decision: String::new(),
            legacy: Map::new(),
        }
    }

    #[test]
    fn pruning_is_same_tier_and_keeps_non_dominated_entries_and_snapshots() {
        let temp = test_temp("prune");
        let eval_dir = temp.path.join("evals");
        fs::create_dir_all(eval_dir.join("frontier")).unwrap();
        let mut entries: Vec<FrontierEntry> =
            (0..21).map(|id| entry(id, "T1", id as f64)).collect();
        entries.push(entry(99, "T2", 1.0));
        let mut text = Vec::new();
        for item in &entries {
            serde_json::to_writer(&mut text, item).unwrap();
            text.push(b'\n');
            fs::write(
                eval_dir
                    .join("frontier")
                    .join(format!("{}.md", item.candidate_id)),
                "snapshot",
            )
            .unwrap();
        }
        fs::write(eval_dir.join("frontier.jsonl"), text).unwrap();
        update_frontier(&eval_dir, "new candidate", vec![entry(100, "T1", 100.0)]).unwrap();
        let kept = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert_eq!(kept.iter().filter(|item| item.tier == "T1").count(), 20);
        assert!(kept.iter().any(|item| item.candidate_id == "id99"));
        assert!(eval_dir.join("frontier/id99.md").exists());
        assert!(!eval_dir.join("frontier/id0.md").exists());

        let mut tradeoffs: Vec<FrontierEntry> = (0..21)
            .map(|id| {
                let mut item = entry(id, "T3", id as f64);
                item.scores_holdout = vec![Some(20.0 - id as f64)];
                item
            })
            .collect();
        prune(&mut tradeoffs);
        assert_eq!(tradeoffs.len(), 20);
        assert!(tradeoffs.iter().any(|item| item.candidate_id == "id20"));

        let mut incomplete: Vec<FrontierEntry> = (0..25)
            .map(|id| {
                let mut item = entry(id, "T4", 0.0);
                item.scores_nonholdout = vec![None];
                item.mean_nonholdout = None;
                item
            })
            .collect();
        prune(&mut incomplete);
        assert_eq!(incomplete.len(), 20);

        let mut mixed: Vec<FrontierEntry> = (0..20)
            .map(|id| entry(id, "T4", if id == 0 { 0.0 } else { 1.0 }))
            .collect();
        let mut newest_incomplete = entry(20, "T4", 0.0);
        newest_incomplete.scores_nonholdout = vec![None];
        newest_incomplete.mean_nonholdout = None;
        mixed.push(newest_incomplete);
        prune(&mut mixed);
        assert!(!mixed.iter().any(|item| item.candidate_id == "id0"));
        assert!(mixed.iter().any(|item| item.candidate_id == "id20"));

        let mut empty: Vec<FrontierEntry> = (0..25)
            .map(|id| {
                let mut item = entry(id, "", 0.0);
                item.scores_nonholdout.clear();
                item.scores_holdout.clear();
                item.mean_nonholdout = None;
                item
            })
            .collect();
        prune(&mut empty);
        assert_eq!(empty.len(), 20);

        let mut accepted = entry(200, "T4", 0.0);
        accepted.is_accepted = true;
        let mut accepted_entries = vec![accepted];
        accepted_entries.extend((201..222).map(|id| entry(id, "T4", 10.0)));
        prune(&mut accepted_entries);
        assert!(
            accepted_entries
                .iter()
                .any(|item| item.candidate_id == "id200")
        );
        assert_eq!(
            accepted_entries
                .iter()
                .filter(|item| !item.is_accepted)
                .count(),
            20
        );

        let mut accepted_outside_cap: Vec<FrontierEntry> = (0..15)
            .map(|id| {
                let mut item = entry(id, "T5", 1.0);
                item.is_accepted = true;
                item
            })
            .collect();
        accepted_outside_cap.extend((100..110).map(|id| entry(id, "T5", 1.0)));
        prune(&mut accepted_outside_cap);
        assert_eq!(accepted_outside_cap.len(), 25);
    }
    #[test]
    fn normalizer_removes_only_metadata_floor() {
        let text = "---\nmetadata:\n  minimum-tier: T4\n  short: x\nother:\n  minimum-tier: keep\n---\nbody\n";
        assert!(
            normalize_minimum_tier(text)
                .unwrap()
                .contains("minimum-tier: keep")
        );
        assert!(
            !normalize_minimum_tier(text)
                .unwrap()
                .contains("minimum-tier: T4")
        );
    }

    #[test]
    fn normalizer_rejects_ambiguous_floor() {
        assert!(
            normalize_minimum_tier("---\nmetadata:\n  minimum-tier: T3 # old\n---\nx").is_err()
        );
    }

    #[test]
    fn floor_insertion_preserves_frontmatter() {
        let text = "---\nname: x\nmetadata:\n  minimum-tier: T4\n  short: x\n---\nbody\n";
        assert_eq!(
            set_minimum_tier(text, "T2").unwrap(),
            "---\nname: x\nmetadata:\n  minimum-tier: T2\n  short: x\n---\nbody\n"
        );
        assert_eq!(
            set_minimum_tier("---\nname: x\n---\nbody\n", "T2").unwrap(),
            "---\nname: x\nmetadata:\n  minimum-tier: T2\n---\nbody\n"
        );
        assert_eq!(
            set_minimum_tier("---\nname: x\nmetadata:\n---\nbody\n", "T2").unwrap(),
            "---\nname: x\nmetadata:\n  minimum-tier: T2\n---\nbody\n"
        );
        assert_eq!(
            set_minimum_tier("---\nname: x\nmetadata:\n# note\n---\nbody\n", "T2").unwrap(),
            "---\nname: x\nmetadata:\n  minimum-tier: T2\n# note\n---\nbody\n"
        );
    }

    #[test]
    fn exact_ratio_comparison_avoids_display_rounding() {
        assert!(Ratio { n: 1601, d: 200 }.cmp(Ratio { n: 8, d: 1 }).is_gt());
    }

    #[test]
    fn paired_entry_carries_incumbent_identity() {
        let mut entry = entry(1, "T1", 8.0);
        entry.comparison_id = "paired".to_string();
        entry.incumbent_id = "incumbent".to_string();
        assert_eq!(entry.comparison_id, "paired");
        assert_eq!(entry.incumbent_id, "incumbent");
    }

    #[test]
    fn selected_floor_is_not_part_of_identity() {
        let low =
            normalize_minimum_tier("---\nmetadata:\n  minimum-tier: T1\n---\nbody\n").unwrap();
        let high =
            normalize_minimum_tier("---\nmetadata:\n  minimum-tier: T5\n---\nbody\n").unwrap();
        assert_eq!(candidate_id(&low), candidate_id(&high));
    }

    #[test]
    fn empty_floor_is_an_error_and_crlf_is_supported() {
        assert!(normalize_minimum_tier("---\nmetadata:\n  minimum-tier:\n---\nbody\n").is_err());
        assert_eq!(
            normalize_minimum_tier("---\r\nmetadata:\r\n  minimum-tier: T4\r\n---\r\nbody\r\n")
                .unwrap(),
            "---\r\n---\r\nbody\r\n"
        );
        assert!(
            normalize_minimum_tier("---\nmetadata:\n    minimum-tier: T4\n---\nbody\n").is_err()
        );
        assert!(normalize_minimum_tier("---\nmetadata:\n\tminimum-tier: T4\n---\nbody\n").is_err());
        assert_eq!(
            normalize_minimum_tier(
                "---\nmetadata:\n  minimum-tier: T4\nother:\n  sub: y\n---\nbody\n"
            )
            .unwrap(),
            "---\nother:\n  sub: y\n---\nbody\n"
        );
        let commented = "---\nname: x\nmetadata:\n# note\n\n  minimum-tier: T4\n---\nbody\n";
        assert_eq!(
            normalize_minimum_tier(commented).unwrap(),
            "---\nname: x\n# note\n\n---\nbody\n"
        );
        assert_eq!(
            set_minimum_tier(commented, "T2").unwrap(),
            "---\nname: x\nmetadata:\n# note\n\n  minimum-tier: T2\n---\nbody\n"
        );
    }

    #[test]
    fn acceptance_flag_requires_a_full_candidate_run() {
        let base = [
            OsString::from("--eval-dir"),
            OsString::from("evals"),
            OsString::from("--accept-if-winning"),
        ];
        assert!(parse_args(&base).is_err());

        let with_holdout = [
            OsString::from("--eval-dir"),
            OsString::from("evals"),
            OsString::from("--accept-if-winning"),
            OsString::from("--holdout"),
            OsString::from("candidate.md"),
        ];
        assert!(parse_args(&with_holdout).is_err());

        let with_tier = [
            OsString::from("--eval-dir"),
            OsString::from("evals"),
            OsString::from("--accept-if-winning"),
            OsString::from("--tier"),
            OsString::from("T3"),
            OsString::from("candidate.md"),
        ];
        assert!(parse_args(&with_tier).is_err());
    }

    fn scored_slice(case_id: &str, score: Option<f64>) -> SliceResult {
        SliceResult {
            scores: vec![score],
            repeats: BTreeMap::from([(case_id.to_string(), vec![score.map(|value| value as u8)])]),
            models: BTreeSet::new(),
            case_timings: BTreeMap::new(),
        }
    }

    fn scored_arm(id: &str, tiers: &[String], values: &[(Option<f64>, Option<f64>)]) -> PairedArm {
        PairedArm {
            text: id.to_string(),
            id: id.to_string(),
            repeats: 1,
            results: tiers
                .iter()
                .zip(values)
                .map(|(tier, (nonholdout, holdout))| {
                    (
                        tier.clone(),
                        TierResult {
                            nonholdout: scored_slice("n", *nonholdout),
                            holdout: scored_slice("h", *holdout),
                        },
                    )
                })
                .collect(),
        }
    }

    fn selection_cases() -> (Case, Case) {
        (
            Case {
                id: "n".to_string(),
                input: Value::Null,
                expect: String::new(),
                is_holdout: false,
                files: Vec::new(),
            },
            Case {
                id: "h".to_string(),
                input: Value::Null,
                expect: String::new(),
                is_holdout: true,
                files: Vec::new(),
            },
        )
    }

    #[test]
    fn suffix_selection_uses_absolute_score_and_widest_exact_tie() {
        let tiers = ["T1", "T2", "T3"].map(str::to_string);
        let candidate = scored_arm(
            "candidate",
            &tiers,
            &[
                (Some(8.0), Some(9.0)),
                (Some(9.0), Some(9.0)),
                (Some(9.0), Some(9.0)),
            ],
        );
        let incumbent = scored_arm(
            "incumbent",
            &tiers,
            &[
                (Some(7.0), Some(7.0)),
                (Some(7.0), Some(7.0)),
                (Some(7.0), Some(7.0)),
            ],
        );
        let (nonholdout, holdout) = selection_cases();
        let selection = select_suffix(&candidate, &incumbent, &tiers, &[&nonholdout], &[&holdout]);
        assert_eq!(selection.start, Some(1));
        assert!(selection.is_accepted);
    }

    #[test]
    fn selected_suffix_failure_does_not_fall_back() {
        let tiers = ["T1", "T2", "T3"].map(str::to_string);
        let candidate = scored_arm(
            "candidate",
            &tiers,
            &[
                (Some(7.0), Some(9.0)),
                (Some(8.0), Some(9.0)),
                (Some(10.0), Some(5.0)),
            ],
        );
        let incumbent = scored_arm(
            "incumbent",
            &tiers,
            &[
                (Some(6.0), Some(6.0)),
                (Some(6.0), Some(6.0)),
                (Some(6.0), Some(6.0)),
            ],
        );
        let (nonholdout, holdout) = selection_cases();
        let selection = select_suffix(&candidate, &incumbent, &tiers, &[&nonholdout], &[&holdout]);
        assert_eq!(selection.start, Some(2));
        assert_eq!(selection.reason, "holdout_not_strictly_better");
        assert!(!selection.is_accepted);
    }

    #[test]
    fn incomplete_lower_tier_does_not_block_selected_suffix() {
        let tiers = ["T1", "T2", "T3"].map(str::to_string);
        let candidate = scored_arm(
            "candidate",
            &tiers,
            &[
                (Some(10.0), Some(10.0)),
                (Some(9.0), Some(9.0)),
                (Some(9.0), Some(9.0)),
            ],
        );
        let incumbent = scored_arm(
            "incumbent",
            &tiers,
            &[(None, None), (Some(7.0), Some(7.0)), (Some(7.0), Some(7.0))],
        );
        let (nonholdout, holdout) = selection_cases();
        let selection = select_suffix(&candidate, &incumbent, &tiers, &[&nonholdout], &[&holdout]);
        assert_eq!(selection.start, Some(1));
        assert!(selection.is_accepted);

        let temp = test_temp("selected-tier-acceptance");
        let eval_dir = temp.path.join("evals");
        fs::create_dir_all(&eval_dir).unwrap();
        let evidence = paired_entries(&candidate, &incumbent, &tiers, &selection, "base");
        let comparison_id = evidence[0].comparison_id.clone();
        update_frontier(&eval_dir, &candidate.text, evidence).unwrap();
        let selected_tiers = BTreeSet::from(["T2".to_string(), "T3".to_string()]);
        let _lock = FrontierLock::acquire(&eval_dir).unwrap();
        update_comparison_locked(&eval_dir, &comparison_id, &selected_tiers, true, "accepted")
            .unwrap();
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert!(
            !entries
                .iter()
                .find(|entry| entry.tier == "T1")
                .unwrap()
                .is_accepted
        );
        assert!(
            entries
                .iter()
                .filter(|entry| entry.tier != "T1")
                .all(|entry| entry.is_accepted)
        );
        let accepted_comparisons = BTreeSet::from([comparison_id]);
        let excluded = entries.iter().find(|entry| entry.tier == "T1").unwrap();
        assert!(!is_prunable(excluded, &accepted_comparisons));

        let mut truncated = candidate.clone();
        truncated
            .results
            .get_mut("T2")
            .unwrap()
            .nonholdout
            .repeats
            .get_mut("n")
            .unwrap()
            .clear();
        let selection = select_suffix(&truncated, &incumbent, &tiers, &[&nonholdout], &[&holdout]);
        assert_eq!(selection.start, Some(1));
        assert_eq!(selection.reason, "incomplete");

        let unscored = scored_arm(
            "unscored",
            &tiers,
            &[(None, None), (None, None), (None, None)],
        );
        let selection = select_suffix(&unscored, &incumbent, &tiers, &[&nonholdout], &[&holdout]);
        assert_eq!(selection.start, None);
        assert_eq!(selection.reason, "incomplete");
    }

    const PAIRED_FAKE: &str = r#"#!/bin/zsh
set -eu
while (( $# )); do
  case "$1" in
    --tier) tier=$2; shift 2 ;;
    --input) input=$2; shift 2 ;;
    --system-prompt-file) prompt=$2; shift 2 ;;
    *) shift 2 ;;
  esac
done
if [[ "$input" == 'Grade the actual output'* ]]; then
  if [[ "$input" == *winning-output* ]]; then score=9; else score=7; fi
  print "{\"score\":$score,\"failure_mode\":null}"
  print -u2 'model_ran: judge-model'
else
  if [[ "$(<"$prompt")" == *'winning candidate'* ]]; then
    print winning-output
  else
    print incumbent-output
  fi
  print -u2 "model_ran: actual-$tier"
fi
"#;

    const FATAL_CONCURRENT_FAKE: &str = r#"#!/bin/zsh
set -eu
base=${0:h}
while (( $# )); do
  case "$1" in
    --input) input=$2; shift 2 ;;
    --system-prompt-file) prompt=$2; shift 2 ;;
    *) shift 2 ;;
  esac
done
if [[ "$input" == 'Grade the actual output'* ]]; then
  print 'start judge' >> "$base/events"
  print '{"score":8,"failure_mode":null}'
  print -u2 'model_ran: judge-model'
  exit 0
fi
if [[ "$(<"$prompt")" == *'winning candidate'* ]]; then
  while [[ ! -f "$base/incumbent-start" ]]; do sleep 0.001; done
  print 'start actual candidate' >> "$base/events"
  print -u2 bad-config
  exit 2
fi
print 'start actual incumbent' >> "$base/events"
touch "$base/incumbent-start"
sleep 0.05
print output
print -u2 'model_ran: actual-model'
"#;

    const CONCURRENT_FAKE: &str = r#"#!/bin/zsh
set -eu
base=${0:h}
while (( $# )); do
  case "$1" in
    --input) input=$2; shift 2 ;;
    --system-prompt-file) prompt=$2; shift 2 ;;
    *) shift 2 ;;
  esac
done
if [[ "$input" == 'Grade the actual output'* ]]; then
  arm=judge
else
  if [[ "$(<"$prompt")" == *'winning candidate'* ]]; then arm=candidate; else arm=incumbent; fi
fi
while ! mkdir "$base/mutex" 2>/dev/null; do sleep 0.001; done
active=0
[[ -f "$base/active" ]] && active=$(<"$base/active")
active=$((active + 1))
print "$active" > "$base/active"
max=0
[[ -f "$base/max" ]] && max=$(<"$base/max")
if (( active > max )); then print "$active" > "$base/max"; fi
print "start $arm" >> "$base/events"
rmdir "$base/mutex"
sleep 0.05
while ! mkdir "$base/mutex" 2>/dev/null; do sleep 0.001; done
active=$(<"$base/active")
active=$((active - 1))
print "$active" > "$base/active"
print "end $arm" >> "$base/events"
rmdir "$base/mutex"
if [[ "$arm" == judge ]]; then
  print '{"score":8,"failure_mode":null}'
  print -u2 'model_ran: judge-model'
else
  print output
  print -u2 'model_ran: actual-model'
fi
"#;

    fn paired_fixture(name: &str, fake: &str) -> (TempDir, Settings, PathBuf, PathBuf) {
        let (temp, mut settings, eval_dir) = fixture(name, fake);
        fs::write(&settings.tiers_file, r#"{"tiers":{"T1":{}}}"#).unwrap();
        settings.repeats = 1;
        let candidate = temp.path.join("candidate.md");
        fs::write(
            &candidate,
            "---\nname: candidate\nmetadata:\n  minimum-tier: T4\n---\nwinning candidate\n",
        )
        .unwrap();
        settings.args.candidate = Some(candidate.clone());
        (temp, settings, eval_dir, candidate)
    }

    fn paired_state_dir(settings: &Settings, eval_dir: &Path, candidate_path: &Path) -> PathBuf {
        let artifact_dir = eval_dir.parent().unwrap();
        let submitted = fs::read_to_string(candidate_path).unwrap();
        let candidate = normalize_minimum_tier(&submitted).unwrap();
        let incumbent =
            normalize_minimum_tier(&fs::read_to_string(artifact_dir.join("SKILL.md")).unwrap())
                .unwrap();
        let cases = load_cases(&settings.cases_file).unwrap();
        let rubric = fs::read_to_string(eval_dir.join("rubric.md")).unwrap();
        let run_key = paired_run_key(
            settings,
            &candidate,
            &incumbent,
            &submitted,
            &cases,
            &rubric,
            artifact_dir,
        )
        .unwrap();
        eval_dir.join(".skill-eval-state").join(run_key)
    }

    #[test]
    fn dry_winner_stays_unaccepted_and_apply_inserts_floor() {
        let (_temp, settings, eval_dir, _candidate) = paired_fixture("paired-dry", PAIRED_FAKE);
        assert_eq!(run(settings).unwrap(), 0);
        let live = fs::read_to_string(eval_dir.join("../SKILL.md")).unwrap();
        assert!(live.contains("candidate text"));
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert!(entries.iter().all(|entry| !entry.is_accepted));
        assert!(entries.iter().all(|entry| entry.decision == "would_accept"));

        let (_temp, mut settings, eval_dir, _candidate) =
            paired_fixture("paired-accept", PAIRED_FAKE);
        settings.args.is_accept_if_winning = true;
        assert_eq!(run(settings).unwrap(), 0);
        let live = fs::read_to_string(eval_dir.join("../SKILL.md")).unwrap();
        assert!(live.contains("metadata:\n  minimum-tier: T1\n"));
        assert!(live.contains("winning candidate"));
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert!(entries.iter().all(|entry| entry.is_accepted));

        let (_temp, mut settings, eval_dir, candidate) =
            paired_fixture("paired-floorless", PAIRED_FAKE);
        fs::write(
            eval_dir.join("../SKILL.md"),
            "---\nname: incumbent\n---\nincumbent body\n",
        )
        .unwrap();
        fs::write(&candidate, "---\nname: candidate\n---\nwinning candidate\n").unwrap();
        settings.args.is_accept_if_winning = true;
        assert_eq!(run(settings).unwrap(), 0);
        let live = fs::read_to_string(eval_dir.join("../SKILL.md")).unwrap();
        assert!(!live.contains("minimum-tier"));
        assert!(live.contains("winning candidate"));

        let (_temp, mut settings, eval_dir, candidate) =
            paired_fixture("paired-floor-removal", PAIRED_FAKE);
        fs::write(&candidate, "---\nname: candidate\n---\nwinning candidate\n").unwrap();
        settings.args.is_accept_if_winning = true;
        assert_eq!(run(settings).unwrap(), 0);
        let live = fs::read_to_string(eval_dir.join("../SKILL.md")).unwrap();
        assert!(!live.contains("minimum-tier"));
    }

    #[test]
    fn live_write_failure_preserves_unaccepted_evidence() {
        let (_temp, mut settings, eval_dir, _candidate) =
            paired_fixture("live-write-failure", PAIRED_FAKE);
        settings.args.is_accept_if_winning = true;
        let artifact_dir = eval_dir.parent().unwrap();
        let original_mode = fs::metadata(artifact_dir).unwrap().permissions().mode();
        let mut permissions = fs::metadata(artifact_dir).unwrap().permissions();
        permissions.set_mode(0o500);
        fs::set_permissions(artifact_dir, permissions).unwrap();
        let error = run(settings).unwrap_err();
        let mut permissions = fs::metadata(artifact_dir).unwrap().permissions();
        permissions.set_mode(original_mode);
        fs::set_permissions(artifact_dir, permissions).unwrap();
        assert!(error.contains("preserved frontier evidence"));
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert!(entries.iter().all(|entry| !entry.is_accepted));
        assert!(
            entries
                .iter()
                .all(|entry| entry.decision == "live_write_failed")
        );
    }

    #[test]
    fn stale_incumbent_preserves_unaccepted_evidence() {
        let fake = r#"#!/bin/zsh
set -eu
while (( $# )); do
  case "$1" in
    --tier) tier=$2; shift 2 ;;
    --input) input=$2; shift 2 ;;
    --system-prompt-file) prompt=$2; shift 2 ;;
    *) shift 2 ;;
  esac
done
if [[ "$input" == 'Grade the actual output'* ]]; then
  if [[ "$input" == *winning-output* ]]; then score=9; else score=7; fi
  print "{\"score\":$score,\"failure_mode\":null}"
  print -u2 'model_ran: judge-model'
else
  if [[ "$(<"$prompt")" == *'winning candidate'* ]]; then
    print '\nchanged during evaluation' >> "${0:h}/artifact/SKILL.md"
    print winning-output
  else
    print incumbent-output
  fi
  print -u2 "model_ran: actual-$tier"
fi
"#;
        let (_temp, mut settings, eval_dir, _candidate) = paired_fixture("stale-incumbent", fake);
        settings.args.is_accept_if_winning = true;
        let error = run(settings).unwrap_err();
        assert!(error.contains("live incumbent changed"));
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert!(entries.iter().all(|entry| !entry.is_accepted));
        assert!(
            entries
                .iter()
                .all(|entry| entry.decision == "stale_incumbent")
        );
    }

    #[test]
    fn legacy_duplicate_pruning_removes_only_one_row() {
        let mut entries: Vec<FrontierEntry> = (0..21).map(|id| entry(id, "T1", 5.0)).collect();
        entries[1].candidate_id = entries[0].candidate_id.clone();
        prune(&mut entries);
        assert_eq!(entries.len(), 20);
    }

    #[test]
    fn paired_arguments_validate_jobs_and_restart() {
        let base = [
            OsString::from("--eval-dir"),
            OsString::from("evals"),
            OsString::from("--jobs"),
            OsString::from("0"),
            OsString::from("candidate.md"),
        ];
        assert!(parse_args(&base).unwrap_err().contains("positive integer"));
        let too_many = [
            OsString::from("--eval-dir"),
            OsString::from("evals"),
            OsString::from("--jobs"),
            OsString::from((MAX_JOBS + 1).to_string()),
            OsString::from("candidate.md"),
        ];
        let error = parse_args(&too_many).unwrap_err();
        assert!(error.contains("--jobs"));
        assert!(error.contains(USAGE));
        let error = positive_env_value("SKILL_EVAL_JOBS", &(MAX_JOBS + 1).to_string()).unwrap_err();
        assert!(error.contains("SKILL_EVAL_JOBS"));
        assert!(error.contains(USAGE));
        let restart = [
            OsString::from("--eval-dir"),
            OsString::from("evals"),
            OsString::from("--restart"),
        ];
        assert!(parse_args(&restart).unwrap_err().contains("full paired"));
        let valid = [
            OsString::from("--eval-dir"),
            OsString::from("evals"),
            OsString::from("--jobs"),
            OsString::from("2"),
            OsString::from("--restart"),
            OsString::from("candidate.md"),
        ];
        let args = parse_args(&valid).unwrap();
        assert_eq!(args.jobs, Some(2));
        assert!(args.is_restart);
    }

    #[test]
    fn prompt_name_hashes_long_case_ids_to_a_fixed_length() {
        let case_id = "case-".repeat(10_000);
        let name = prompt_name("candidate", &case_id);
        assert_eq!(name.len(), "candidate-".len() + 40 + ".md".len());
        assert_eq!(
            name,
            format!("candidate-{:x}.md", Sha1::digest(case_id.as_bytes()))
        );
        assert_ne!(name, prompt_name("candidate", &(case_id + "different")));
    }

    #[test]
    fn work_units_interleave_arms_for_each_repeat() {
        let cases = vec![
            Case {
                id: "n".to_string(),
                input: Value::Null,
                expect: String::new(),
                is_holdout: false,
                files: Vec::new(),
            },
            Case {
                id: "h".to_string(),
                input: Value::Null,
                expect: String::new(),
                is_holdout: true,
                files: Vec::new(),
            },
        ];
        let units = make_work_units(&["T1".to_string()], &cases, 2);
        assert_eq!(units.len(), 8);
        for pair in units.chunks_exact(2) {
            assert_eq!(pair[0].arm, "incumbent");
            assert_eq!(pair[1].arm, "candidate");
            assert_eq!(pair[0].tier, pair[1].tier);
            assert_eq!(pair[0].slice, pair[1].slice);
            assert_eq!(pair[0].case_id, pair[1].case_id);
            assert_eq!(pair[0].repeat, pair[1].repeat);
        }
    }

    #[test]
    fn run_key_covers_each_scoring_input() {
        let (temp, mut settings, eval_dir, candidate_path) =
            paired_fixture("key-inputs", PAIRED_FAKE);
        let artifact_dir = eval_dir.parent().unwrap();
        fs::write(artifact_dir.join("context.txt"), "one").unwrap();
        fs::write(&settings.cases_file, "{\"id\":\"n1\",\"input\":\"plain input\",\"expect\":\"works\",\"holdout\":false,\"files\":[\"context.txt\"]}\n{\"id\":\"h1\",\"input\":\"holdout\",\"expect\":\"works\",\"holdout\":true}\n").unwrap();
        let cases = load_cases(&settings.cases_file).unwrap();
        write_executable(&eval_dir.join("preflight.sh"), "#!/bin/zsh\nexit 0\n");
        let submitted = fs::read_to_string(&candidate_path).unwrap();
        let candidate = normalize_minimum_tier(&submitted).unwrap();
        let live = fs::read_to_string(artifact_dir.join("SKILL.md")).unwrap();
        let incumbent = normalize_minimum_tier(&live).unwrap();
        let rubric = fs::read_to_string(eval_dir.join("rubric.md")).unwrap();
        let key = paired_run_key(
            &settings,
            &candidate,
            &incumbent,
            &submitted,
            &cases,
            &rubric,
            artifact_dir,
        )
        .unwrap();
        assert_ne!(
            key,
            paired_run_key(
                &settings,
                "changed",
                &incumbent,
                &submitted,
                &cases,
                &rubric,
                artifact_dir
            )
            .unwrap()
        );
        assert_ne!(
            key,
            paired_run_key(
                &settings,
                &candidate,
                "changed",
                &submitted,
                &cases,
                &rubric,
                artifact_dir
            )
            .unwrap()
        );
        assert_ne!(
            key,
            paired_run_key(
                &settings,
                &candidate,
                &incumbent,
                "changed",
                &cases,
                &rubric,
                artifact_dir
            )
            .unwrap()
        );
        fs::write(&settings.cases_file, "{\"id\":\"n1\",\"input\":\"plain input\",\"expect\":\"changed\",\"holdout\":false,\"files\":[\"context.txt\"]}\n{\"id\":\"h1\",\"input\":\"holdout\",\"expect\":\"works\",\"holdout\":true}\n").unwrap();
        assert_ne!(
            key,
            paired_run_key(
                &settings,
                &candidate,
                &incumbent,
                &submitted,
                &cases,
                &rubric,
                artifact_dir
            )
            .unwrap()
        );
        fs::write(&settings.cases_file, "{\"id\":\"n1\",\"input\":\"plain input\",\"expect\":\"works\",\"holdout\":false,\"files\":[\"context.txt\"]}\n{\"id\":\"h1\",\"input\":\"holdout\",\"expect\":\"works\",\"holdout\":true}\n").unwrap();
        fs::write(artifact_dir.join("context.txt"), "two").unwrap();
        assert_ne!(
            key,
            paired_run_key(
                &settings,
                &candidate,
                &incumbent,
                &submitted,
                &cases,
                &rubric,
                artifact_dir
            )
            .unwrap()
        );
        fs::write(artifact_dir.join("context.txt"), "one").unwrap();
        assert_ne!(
            key,
            paired_run_key(
                &settings,
                &candidate,
                &incumbent,
                &submitted,
                &cases,
                "changed",
                artifact_dir
            )
            .unwrap()
        );
        write_executable(&eval_dir.join("output-check.sh"), "#!/bin/zsh\nexit 0\n");
        assert_ne!(
            key,
            paired_run_key(
                &settings,
                &candidate,
                &incumbent,
                &submitted,
                &cases,
                &rubric,
                artifact_dir
            )
            .unwrap()
        );
        fs::remove_file(eval_dir.join("output-check.sh")).unwrap();
        write_executable(&eval_dir.join("preflight.sh"), "#!/bin/zsh\nexit 1\n");
        assert_ne!(
            key,
            paired_run_key(
                &settings,
                &candidate,
                &incumbent,
                &submitted,
                &cases,
                &rubric,
                artifact_dir
            )
            .unwrap()
        );
        write_executable(&eval_dir.join("preflight.sh"), "#!/bin/zsh\nexit 0\n");
        fs::write(&settings.tiers_file, r#"{"tiers":{"T2":{}}}"#).unwrap();
        assert_ne!(
            key,
            paired_run_key(
                &settings,
                &candidate,
                &incumbent,
                &submitted,
                &cases,
                &rubric,
                artifact_dir
            )
            .unwrap()
        );
        fs::write(&settings.tiers_file, r#"{"tiers":{"T1":{}}}"#).unwrap();
        settings.repeats = 2;
        assert_ne!(
            key,
            paired_run_key(
                &settings,
                &candidate,
                &incumbent,
                &submitted,
                &cases,
                &rubric,
                artifact_dir
            )
            .unwrap()
        );
        settings.repeats = 1;
        assert_eq!(
            key,
            paired_run_key(
                &settings,
                &candidate,
                &incumbent,
                &submitted,
                &cases,
                &rubric,
                artifact_dir
            )
            .unwrap()
        );
        write_executable(&settings.tier_dispatch_bin, "#!/bin/zsh\nexit 0\n");
        assert_ne!(
            key,
            paired_run_key(
                &settings,
                &candidate,
                &incumbent,
                &submitted,
                &cases,
                &rubric,
                artifact_dir
            )
            .unwrap()
        );
        write_executable(&settings.tier_dispatch_bin, PAIRED_FAKE);
        fs::write(
            settings.auth_extension.join("extension.ts"),
            "changed extension",
        )
        .unwrap();
        assert_ne!(
            key,
            paired_run_key(
                &settings,
                &candidate,
                &incumbent,
                &submitted,
                &cases,
                &rubric,
                artifact_dir
            )
            .unwrap()
        );
        assert!(temp.path.exists());
    }

    #[test]
    fn auth_extension_git_metadata_does_not_change_the_paired_run_key() {
        let (_temp, settings, eval_dir, candidate_path) =
            paired_fixture("git-metadata", PAIRED_FAKE);
        let before = paired_state_dir(&settings, &eval_dir, &candidate_path);
        let git_dir = settings.auth_extension.join(".git");
        fs::create_dir(&git_dir).unwrap();
        fs::write(git_dir.join("FETCH_HEAD"), "first fetch").unwrap();
        let after_first_fetch = paired_state_dir(&settings, &eval_dir, &candidate_path);
        fs::write(git_dir.join("FETCH_HEAD"), "second fetch").unwrap();
        let after_second_fetch = paired_state_dir(&settings, &eval_dir, &candidate_path);
        assert_eq!(before, after_first_fetch);
        assert_eq!(before, after_second_fetch);
    }

    #[test]
    fn broken_nested_auth_extension_link_hashes_its_target() {
        use std::os::unix::fs::symlink;

        let (temp, mut settings, _eval_dir) = fixture("broken-nested-link", FAKE);
        let extension = settings.auth_extension.clone();
        let configured_link = temp.path.join("configured-auth");
        symlink(&extension, &configured_link).unwrap();
        settings.auth_extension = configured_link;
        let nested_link = extension.join("missing-nested-link");
        symlink("missing-first", &nested_link).unwrap();
        let first = run_key_input(&settings.auth_extension).unwrap();
        assert_eq!(first, run_key_input(&extension).unwrap());
        fs::remove_file(&nested_link).unwrap();
        symlink("missing-second", &nested_link).unwrap();
        assert_ne!(first, run_key_input(&settings.auth_extension).unwrap());
    }

    #[test]
    fn nested_auth_extension_link_hashes_target_content() {
        use std::os::unix::fs::symlink;

        let (_temp, settings, _eval_dir) = fixture("nested-link-content", FAKE);
        let target = settings.auth_extension.join("target.ts");
        fs::write(&target, "first target content").unwrap();
        symlink("target.ts", settings.auth_extension.join("nested-link")).unwrap();
        let first = run_key_input(&settings.auth_extension).unwrap();
        fs::write(&target, "second target content").unwrap();
        assert_ne!(first, run_key_input(&settings.auth_extension).unwrap());
    }

    #[test]
    fn nested_auth_extension_link_cycle_does_not_recurse() {
        use std::os::unix::fs::symlink;

        let (_temp, settings, _eval_dir) = fixture("nested-link-cycle", FAKE);
        symlink(".", settings.auth_extension.join("nested-link")).unwrap();
        let first = run_key_input(&settings.auth_extension).unwrap();
        assert_eq!(first, run_key_input(&settings.auth_extension).unwrap());
    }

    #[test]
    fn corrupt_state_and_duplicate_coordinator_fail_closed() {
        let temp = test_temp("state-lock");
        let eval_dir = temp.path.join("evals");
        fs::create_dir(&eval_dir).unwrap();
        let state_root = eval_dir.join(".skill-eval-state");
        let key = "run";
        let first = RunLock::acquire(&eval_dir).unwrap();
        assert!(RunLock::acquire(&eval_dir).is_err());
        assert_eq!(
            fs::read_dir(&state_root).unwrap().count(),
            1,
            "one stable lock file serves every run key"
        );
        assert!(state_root.join("run.lock").exists());
        drop(first);
        let (state_dir, _) = initialize_state(&temp.path, key, false).unwrap();
        fs::write(state_dir.join("manifest.json"), "not json").unwrap();
        assert!(
            initialize_state(&temp.path, key, false)
                .unwrap_err()
                .contains("corrupt state")
        );
    }

    #[test]
    fn run_lock_reports_contention_and_system_errors_separately() {
        let eval_dir = Path::new("artifact/evals");
        let path = Path::new("artifact/evals/.skill-eval-state/run.lock");
        let contention = run_lock_error(eval_dir, path, TryLockError::WouldBlock);
        assert_eq!(
            contention,
            "paired evaluation already runs in artifact/evals"
        );
        assert!(
            run_lock_error(
                eval_dir,
                path,
                TryLockError::Error(io::Error::other("disk error"))
            )
            .contains(
                "cannot acquire advisory lock artifact/evals/.skill-eval-state/run.lock in artifact/evals: disk error"
            )
        );
    }

    #[test]
    fn restart_discards_existing_units_and_resume_dispatches_only_missing_unit() {
        let (temp, mut settings, eval_dir, candidate_path) = paired_fixture("resume", FAKE);
        let artifact_dir = eval_dir.parent().unwrap();
        let submitted = fs::read_to_string(&candidate_path).unwrap();
        let candidate = normalize_minimum_tier(&submitted).unwrap();
        let incumbent =
            normalize_minimum_tier(&fs::read_to_string(artifact_dir.join("SKILL.md")).unwrap())
                .unwrap();
        let cases = load_cases(&settings.cases_file).unwrap();
        let rubric = fs::read_to_string(eval_dir.join("rubric.md")).unwrap();
        let key = paired_run_key(
            &settings,
            &candidate,
            &incumbent,
            &submitted,
            &cases,
            &rubric,
            artifact_dir,
        )
        .unwrap();
        let (state_dir, _) = initialize_state(&eval_dir, &key, false).unwrap();
        prepare_prompts(&state_dir, &incumbent, &candidate, artifact_dir, &cases).unwrap();
        let units = make_work_units(&["T1".to_string()], &cases, 1);
        for unit in &units[..units.len() - 1] {
            let result = UnitResult {
                unit: unit.clone(),
                score: Some(8),
                actual_model: Some("fake".to_string()),
                is_output_check_failed: false,
                timing: RepeatTiming::default(),
            };
            atomic_write(
                &unit_path(&state_dir, unit).unwrap(),
                &serde_json::to_vec(&result).unwrap(),
            )
            .unwrap();
        }
        run(settings.clone()).unwrap();
        let calls = fs::read_to_string(temp.path.join("calls")).unwrap();
        assert_eq!(
            calls.lines().filter(|line| line.starts_with('T')).count(),
            2
        );
        assert!(!state_dir.exists());

        let (state_dir, _) = initialize_state(&eval_dir, &key, false).unwrap();
        prepare_prompts(&state_dir, &incumbent, &candidate, artifact_dir, &cases).unwrap();
        let result = UnitResult {
            unit: units[0].clone(),
            score: Some(8),
            actual_model: Some("fake".to_string()),
            is_output_check_failed: false,
            timing: RepeatTiming::default(),
        };
        atomic_write(
            &unit_path(&state_dir, &units[0]).unwrap(),
            &serde_json::to_vec(&result).unwrap(),
        )
        .unwrap();
        settings.args.is_restart = true;
        fs::remove_file(temp.path.join("calls")).unwrap();
        run(settings).unwrap();
        assert_eq!(
            fs::read_to_string(temp.path.join("calls"))
                .unwrap()
                .lines()
                .filter(|line| line.starts_with('T'))
                .count(),
            8
        );
    }

    #[test]
    fn failed_partial_paired_run_writes_no_frontier_or_live_definition() {
        let fatal = "#!/bin/zsh\nprint -u2 bad-config\nexit 2\n";
        let (_temp, settings, eval_dir, _candidate) = paired_fixture("partial", fatal);
        let live = fs::read_to_string(eval_dir.join("../SKILL.md")).unwrap();
        assert!(run(settings).unwrap_err().contains("config or usage error"));
        assert!(!eval_dir.join("frontier.jsonl").exists());
        assert_eq!(
            fs::read_to_string(eval_dir.join("../SKILL.md")).unwrap(),
            live
        );
        assert!(eval_dir.join(".skill-eval-state").exists());
    }

    #[test]
    fn fatal_worker_stops_new_units_after_running_unit_finishes() {
        let (temp, mut settings, eval_dir, candidate) =
            paired_fixture("fatal-worker", FATAL_CONCURRENT_FAKE);
        settings.jobs = 2;
        let state_dir = paired_state_dir(&settings, &eval_dir, &candidate);
        assert!(run(settings).unwrap_err().contains("config or usage error"));
        let events = fs::read_to_string(temp.path.join("events")).unwrap();
        assert!(events.contains("start actual incumbent"));
        assert!(events.contains("start actual candidate"));
        assert!(events.contains("start judge"));
        assert_eq!(
            events
                .lines()
                .filter(|event| event.starts_with("start actual"))
                .count(),
            2
        );
        assert_eq!(fs::read_dir(state_dir.join("units")).unwrap().count(), 1);
    }

    #[test]
    fn completed_null_scores_resume_without_retry_and_restart_despite_advisory_lock() {
        let exhausted = "#!/bin/zsh\nprint call >> \"${0:h}/calls\"\nexit 3\n";
        let (temp, mut settings, eval_dir, candidate) = paired_fixture("restart-lock", exhausted);
        let state_dir = paired_state_dir(&settings, &eval_dir, &candidate);
        let run_lock = eval_dir.join(".skill-eval-state/run.lock");

        assert_eq!(run(settings.clone()).unwrap(), 2);
        assert!(state_dir.exists());
        assert!(run_lock.exists());
        assert_eq!(fs::metadata(&run_lock).unwrap().len(), 0);
        assert_eq!(
            fs::read_to_string(temp.path.join("calls"))
                .unwrap()
                .lines()
                .count(),
            4
        );

        assert_eq!(run(settings.clone()).unwrap(), 2);
        assert_eq!(
            fs::read_to_string(temp.path.join("calls"))
                .unwrap()
                .lines()
                .count(),
            4
        );

        settings.args.is_restart = true;
        assert_eq!(run(settings).unwrap(), 2);
        assert!(state_dir.exists());
        assert!(run_lock.exists());
        assert_eq!(
            fs::read_to_string(temp.path.join("calls"))
                .unwrap()
                .lines()
                .count(),
            8
        );
    }

    #[test]
    fn fake_dispatches_stay_bounded_and_start_both_arms_before_completion() {
        let (temp, mut settings, eval_dir, _candidate) = paired_fixture("bounded", CONCURRENT_FAKE);
        settings.jobs = 2;
        write_executable(
            &eval_dir.join("output-check.sh"),
            "#!/bin/zsh\nset -eu\nwhile ! mkdir \"$PWD/check-mutex\" 2>/dev/null; do sleep 0.001; done\nactive=0\n[[ -f \"$PWD/check-active\" ]] && active=$(<\"$PWD/check-active\")\nactive=$((active + 1))\nprint \"$active\" > \"$PWD/check-active\"\nmax=0\n[[ -f \"$PWD/check-max\" ]] && max=$(<\"$PWD/check-max\")\nif (( active > max )); then print \"$active\" > \"$PWD/check-max\"; fi\nrmdir \"$PWD/check-mutex\"\nsleep 0.05\nwhile ! mkdir \"$PWD/check-mutex\" 2>/dev/null; do sleep 0.001; done\nactive=$(<\"$PWD/check-active\")\nprint \"$((active - 1))\" > \"$PWD/check-active\"\nrmdir \"$PWD/check-mutex\"\n",
        );
        run(settings).unwrap();
        let max_dispatches = fs::read_to_string(temp.path.join("max"))
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap();
        assert_eq!(max_dispatches, 2);
        assert_eq!(
            fs::read_to_string(eval_dir.join("check-max"))
                .unwrap()
                .trim(),
            "1"
        );
        let events: Vec<_> = fs::read_to_string(temp.path.join("events"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        let first_end = events
            .iter()
            .position(|event| event.starts_with("end "))
            .unwrap();
        assert!(
            events[..first_end]
                .iter()
                .any(|event| event == "start incumbent")
        );
        assert!(
            events[..first_end]
                .iter()
                .any(|event| event == "start candidate")
        );
    }

    #[test]
    fn paired_records_and_progress_have_a_stable_shape() {
        let tiers = ["T1", "T2"].map(str::to_string);
        let arm = scored_arm(
            "candidate",
            &tiers,
            &[(Some(8.0), Some(7.0)), (Some(9.0), Some(8.0))],
        );
        let (nonholdout, holdout) = selection_cases();
        assert!(
            paired_record_values(
                "candidate",
                &arm,
                &tiers,
                &[&nonholdout],
                &[&holdout],
                &BTreeMap::new(),
            )
            .unwrap_err()
            .contains("incomplete unit candidate T1 nonholdout n 0")
        );

        let mut completed = BTreeMap::new();
        for (index, tier) in tiers.iter().enumerate() {
            let judge_tier = tiers.get(index + 1).unwrap_or(tier);
            for (slice, case) in [("nonholdout", &nonholdout), ("holdout", &holdout)] {
                let unit = make_unit("candidate", tier, judge_tier, slice, &case.id, 0);
                completed.insert(
                    unit_name(&unit).unwrap(),
                    UnitResult {
                        unit,
                        score: Some(8),
                        actual_model: Some("model".to_string()),
                        is_output_check_failed: tier == "T2" && slice == "holdout",
                        timing: RepeatTiming::default(),
                    },
                );
            }
        }
        let records = paired_record_values(
            "candidate",
            &arm,
            &tiers,
            &[&nonholdout],
            &[&holdout],
            &completed,
        )
        .unwrap();
        assert_eq!(records.len(), 4);
        assert_eq!(records[0]["arm"], "candidate");
        assert_eq!(records[0]["tier"], "T1");
        assert_eq!(records[0]["id"], "n");
        assert_eq!(records[1]["id"], "h");
        assert_eq!(records[2]["tier"], "T2");
        assert_eq!(records[3]["output_check_failures"], 1);
        let result = UnitResult {
            unit: WorkUnit {
                arm: "candidate".to_string(),
                tier: "T2".to_string(),
                judge_tier: "T2".to_string(),
                slice: "holdout".to_string(),
                case_id: "h".to_string(),
                repeat: 1,
                prompt_name: "prompt".to_string(),
            },
            score: Some(8),
            actual_model: Some("model".to_string()),
            is_output_check_failed: false,
            timing: RepeatTiming::default(),
        };
        assert_eq!(
            progress_line(3, 4, &result),
            "skill-eval: 3/4 candidate T2 holdout h 1 8"
        );
    }
}
