use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const USAGE: &str =
    "usage: skill-eval --eval-dir <artifact/evals> [--holdout] [--tier Tn] [candidate]";

#[derive(Debug)]
struct Args {
    eval_dir: PathBuf,
    is_holdout_only: bool,
    tier: Option<String>,
    candidate: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
struct Case {
    id: String,
    input: Value,
    expect: String,
    #[serde(default, rename = "holdout")]
    is_holdout: bool,
    #[serde(default)]
    files: Vec<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct TiersFile {
    tiers: Map<String, Value>,
}

#[derive(Debug)]
struct Settings {
    args: Args,
    repeats: usize,
    cases_file: PathBuf,
    tiers_file: PathBuf,
    tier_dispatch_bin: PathBuf,
    is_accepted: bool,
    auth_extension: PathBuf,
}

#[derive(Debug)]
struct DispatchResult {
    kind: DispatchKind,
    stdout: String,
    model_ran: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum DispatchKind {
    Success,
    Exhausted,
    Failed,
}

#[derive(Debug)]
struct SliceResult {
    scores: Vec<Option<f64>>,
    repeats: BTreeMap<String, Vec<Option<u8>>>,
    models: BTreeSet<String>,
}

#[derive(Debug)]
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
    mean_nonholdout: Option<f64>,
    #[serde(default, rename = "accepted")]
    is_accepted: bool,
    #[serde(default)]
    ts: String,
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

fn parse_args(raw: &[OsString]) -> Result<Args, String> {
    let mut eval_dir = None;
    let mut is_holdout_only = false;
    let mut tier = None;
    let mut candidate = None;
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
    Ok(Args {
        eval_dir: eval_dir.ok_or_else(|| format!("--eval-dir is required\n{USAGE}"))?,
        is_holdout_only,
        tier,
        candidate,
    })
}

fn env_path(name: &str, default: PathBuf) -> PathBuf {
    env::var_os(name).map_or(default, PathBuf::from)
}

fn settings(args: Args) -> Result<Settings, String> {
    let repeats = env::var("REPEATS")
        .unwrap_or_else(|_| "3".to_string())
        .parse::<usize>()
        .map_err(|error| format!("REPEATS must be a positive integer: {error}"))?;
    if repeats == 0 {
        return Err("REPEATS must be a positive integer".to_string());
    }
    let accepted = env::var("ACCEPTED").unwrap_or_else(|_| "false".to_string());
    let is_accepted = match accepted.as_str() {
        "true" => true,
        "false" => false,
        _ => return Err("ACCEPTED must be true or false".to_string()),
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
        is_accepted,
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

fn run_preflight(eval_dir: &Path, candidate: &Path) -> Result<(), String> {
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
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            eprintln!("tier-dispatch failed to start on tier {tier}: {error}");
            return Ok(DispatchResult {
                kind: DispatchKind::Failed,
                stdout: String::new(),
                model_ran: None,
            });
        }
    };
    decode_dispatch(tier, output)
}

fn decode_dispatch(tier: &str, output: Output) -> Result<DispatchResult, String> {
    let stderr = String::from_utf8_lossy(&output.stderr);
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
) -> Result<Option<SliceResult>, String> {
    let empty_prompt = write_prompt(context.temp, "judge.md", "")?;
    let mut result = SliceResult {
        scores: Vec::new(),
        repeats: BTreeMap::new(),
        models: BTreeSet::new(),
    };
    let mut ungraded = 0;
    for case in cases {
        let prompt = prompt_for_case(context.candidate, context.artifact_dir, case)?;
        let prompt_path = write_prompt(
            context.temp,
            &format!("prompt-{tier}-{}.md", candidate_id(&case.id)),
            &prompt,
        )?;
        let input = case_input(&case.input)?;
        let mut repeat_scores = Vec::with_capacity(context.settings.repeats);
        let mut output_check_failures = 0;
        for _ in 0..context.settings.repeats {
            let actual = dispatch(
                context.settings,
                context.wrapper,
                tier,
                &prompt_path,
                &input,
            )?;
            if actual.kind == DispatchKind::Exhausted {
                return Ok(None);
            }
            if actual.kind == DispatchKind::Failed {
                repeat_scores.push(None);
                ungraded += 1;
                continue;
            }
            if let Some(model) = actual.model_ran {
                result.models.insert(model);
            }
            let output_check_failure = run_output_check(context.eval_dir, &actual.stdout)?;
            if let Some(details) = &output_check_failure {
                output_check_failures += 1;
                eprintln!("output check failed for {} on {tier}: {details}", case.id);
            }
            let prompt = judge_prompt(context.rubric, case, &actual.stdout)?;
            let judged = dispatch(
                context.settings,
                context.wrapper,
                judge_tier,
                &empty_prompt,
                &prompt,
            )?;
            if judged.kind == DispatchKind::Exhausted {
                return Ok(None);
            }
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
        }
        let graded: Vec<u8> = repeat_scores.iter().flatten().copied().collect();
        let case_median = median(&graded);
        println!(
            "{}",
            json!({
                "id": case.id,
                "tier": tier,
                "repeat_scores": repeat_scores,
                "median": case_median,
                "output_check_failures": output_check_failures
            })
        );
        result.scores.push(case_median);
        result.repeats.insert(case.id.clone(), repeat_scores);
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
    Ok(Some(result))
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

fn prune(entries: &mut Vec<FrontierEntry>) {
    let tiers: BTreeSet<String> = entries.iter().map(|entry| entry.tier.clone()).collect();
    for tier in tiers {
        loop {
            let indices: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| (entry.tier == tier).then_some(index))
                .collect();
            if indices.len() <= 20 {
                break;
            }
            let Some(remove) = indices.iter().copied().find(|candidate| {
                !entries[*candidate].is_accepted
                    && indices.iter().copied().any(|other| {
                        other != *candidate && dominates(&entries[other], &entries[*candidate])
                    })
            }) else {
                break;
            };
            entries.remove(remove);
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
            let is_legacy_mechanical = ["candidate", "runner", "slice", "cases", "mean", "date"]
                .iter()
                .all(|field| object.contains_key(*field));
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

fn update_frontier(
    eval_dir: &Path,
    candidate: &str,
    new_entries: Vec<FrontierEntry>,
) -> Result<(), String> {
    if new_entries.is_empty() {
        return Ok(());
    }
    let _lock = FrontierLock::acquire(eval_dir)?;
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

fn run(settings: Settings) -> Result<(), String> {
    let eval_dir = fs::canonicalize(&settings.args.eval_dir).map_err(|error| {
        format!(
            "cannot resolve {}: {error}",
            settings.args.eval_dir.display()
        )
    })?;
    let candidate_path = candidate_path(&settings.args);
    let candidate_path = if candidate_path.is_absolute() {
        candidate_path
    } else {
        env::current_dir()
            .map_err(|error| format!("cannot read current directory: {error}"))?
            .join(candidate_path)
    };
    run_preflight(&eval_dir, &candidate_path)?;
    let candidate = fs::read_to_string(&candidate_path)
        .map_err(|error| format!("cannot read {}: {error}", candidate_path.display()))?;
    let rubric_path = eval_dir.join("rubric.md");
    let rubric = fs::read_to_string(&rubric_path)
        .map_err(|error| format!("cannot read {}: {error}", rubric_path.display()))?;
    let cases = load_cases(&settings.cases_file)?;
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
            if run_slice(&context, &tier, &judge_tier, &holdout, "holdout")?.is_none() {
                eprintln!("tier {tier} exhausted; skipped");
            }
            continue;
        }
        let Some(nonholdout_result) =
            run_slice(&context, &tier, &judge_tier, &nonholdout, "nonholdout")?
        else {
            eprintln!("tier {tier} exhausted; skipped for this run");
            continue;
        };
        let Some(holdout_result) = run_slice(&context, &tier, &judge_tier, &holdout, "holdout")?
        else {
            eprintln!("tier {tier} exhausted; skipped for this run");
            continue;
        };
        results.insert(
            tier,
            TierResult {
                nonholdout: nonholdout_result,
                holdout: holdout_result,
            },
        );
    }
    if settings.args.is_holdout_only {
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
                is_accepted: settings.is_accepted,
                ts: ts.clone(),
                legacy: Map::new(),
            }
        })
        .collect();
    update_frontier(&eval_dir, &candidate, entries)?;
    eprintln!("candidate_id: {id}");
    Ok(())
}

fn main() -> ExitCode {
    let raw: Vec<OsString> = env::args_os().skip(1).collect();
    let result = parse_args(&raw).and_then(settings).and_then(run);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("skill-eval: {error}");
            ExitCode::FAILURE
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
        fs::write(artifact.join("SKILL.md"), "candidate text").unwrap();
        fs::write(eval_dir.join("rubric.md"), "Score 0-10").unwrap();
        fs::write(
            eval_dir.join("cases.jsonl"),
            "{\"id\":\"n1\",\"input\":\"plain input\",\"expect\":\"works\",\"holdout\":false}\n{\"id\":\"h1\",\"input\":{\"task\":\"object input\"},\"expect\":\"works\",\"holdout\":true}\n",
        )
        .unwrap();
        let tiers_file = temp.path.join("tiers.json");
        fs::write(&tiers_file, r#"{"tiers":{"T1":{},"T2":{},"T3":{}}}"#).unwrap();
        let extension = temp.path.join("auth.ts");
        fs::write(&extension, "extension").unwrap();
        let dispatch = temp.path.join("tier-dispatch");
        write_executable(&dispatch, fake);
        let args = Args {
            eval_dir: eval_dir.clone(),
            is_holdout_only: false,
            tier: None,
            candidate: None,
        };
        (
            temp,
            Settings {
                args,
                repeats: 3,
                cases_file: eval_dir.join("cases.jsonl"),
                tiers_file,
                tier_dispatch_bin: dispatch,
                is_accepted: false,
                auth_extension: extension,
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
    fn runs_every_tier_slice_and_three_repeats_with_higher_judges() {
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
        let wrapper_path = calls.lines().next().unwrap().split('\t').nth(3).unwrap();
        let wrapper_error = fs::read_to_string(wrapper_path).unwrap_err();
        assert_eq!(wrapper_error.kind(), io::ErrorKind::NotFound);
        let wrapper = fs::read_to_string(temp.path.join("calls.wrapper")).unwrap();
        assert!(wrapper.contains("pi --no-extensions -e \"$PI_ANTHROPIC_AUTH_EXTENSION\" \"$@\""));
        assert!(!wrapper.contains("--no-tools"));
    }

    #[test]
    fn narrow_modes_keep_full_judge_order_and_do_not_write_holdout_frontier() {
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
    }

    #[test]
    fn case_files_are_appended_and_wrapper_has_required_pi_arguments() {
        let (temp, mut settings, _eval_dir) = fixture("files", FAKE);
        let artifact = settings.args.eval_dir.parent().unwrap();
        fs::write(artifact.join("context.txt"), "file sentinel").unwrap();
        fs::write(
            &settings.cases_file,
            "{\"id\":\"n1\",\"input\":\"x\",\"expect\":\"y\",\"files\":[\"context.txt\"]}\n",
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
        assert!(prompts.contains("candidate text"));
        assert!(prompts.contains("--- context.txt ---\nfile sentinel"));
    }

    #[test]
    fn exhaustion_skips_frontier_and_exit_two_aborts() {
        let exhausted = r#"#!/bin/zsh
exit 3
"#;
        let (_temp, mut settings, eval_dir) = fixture("exhausted", exhausted);
        settings.args.tier = Some("T2".to_string());
        run(settings).unwrap();
        assert!(!eval_dir.join("frontier.jsonl").exists());

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
        settings.args.tier = Some("T1".to_string());
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
        settings.args.tier = Some("T1".to_string());
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
                "#!/bin/zsh\nprint -r -- \"$1\" >> {}/preflight-log\nexit 7\n",
                temp.path.display()
            ),
        );
        let error = run(settings).unwrap_err();
        assert!(error.contains("preflight failed"));
        let preflight_log = fs::read_to_string(temp.path.join("preflight-log")).unwrap();
        assert_eq!(preflight_log.lines().count(), 1);
        assert_eq!(
            Path::new(preflight_log.trim()),
            eval_dir.join("../SKILL.md")
        );
        assert!(!temp.path.join("calls").exists());
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
        settings.args.tier = Some("T1".to_string());
        settings.repeats = 1;
        run(settings).unwrap();
        let entries = read_frontier(&eval_dir.join("frontier.jsonl")).unwrap();
        assert_eq!(entries[0].repeat_scores_nonholdout["n1"], vec![Some(4)]);
        assert_eq!(entries[0].repeat_scores_holdout["h1"], vec![Some(4)]);
    }

    #[test]
    fn legacy_frontier_entries_remain_readable() {
        let temp = test_temp("legacy-frontier");
        let path = temp.path.join("frontier.jsonl");
        fs::write(
            &path,
            "{\"candidate\":\"current\",\"runner\":\"mechanical\",\"slice\":\"nonholdout\",\"cases\":17,\"mean\":5.0,\"date\":\"2026-09-03\"}\n",
        )
        .unwrap();
        let entries = read_frontier(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].candidate_id.is_empty());
        assert!(entries[0].tier.is_empty());
        assert_eq!(entries[0].legacy["candidate"], "current");

        fs::write(&path, "{\"candidate_id\":\"truncated\"}\n").unwrap();
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
            mean_nonholdout: Some(score),
            is_accepted: false,
            ts: id.to_string(),
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
        assert_eq!(tradeoffs.len(), 21);

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
    }
}
