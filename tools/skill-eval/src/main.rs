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

const USAGE: &str = "usage: skill-eval --eval-dir <artifact/evals> [--holdout] [--tier Tn] [--accept-if-winning] [candidate]";

#[derive(Debug)]
struct Args {
    eval_dir: PathBuf,
    is_holdout_only: bool,
    tier: Option<String>,
    candidate: Option<PathBuf>,
    is_accept_if_winning: bool,
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

#[derive(Debug)]
struct Settings {
    args: Args,
    repeats: usize,
    cases_file: PathBuf,
    tiers_file: PathBuf,
    tier_dispatch_bin: PathBuf,
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

#[derive(Clone, Debug)]
struct SliceResult {
    scores: Vec<Option<f64>>,
    repeats: BTreeMap<String, Vec<Option<u8>>>,
    models: BTreeSet<String>,
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

fn parse_args(raw: &[OsString]) -> Result<Args, String> {
    let mut eval_dir = None;
    let mut is_holdout_only = false;
    let mut tier = None;
    let mut candidate = None;
    let mut is_accept_if_winning = false;
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
    Ok(Args {
        eval_dir: eval_dir.ok_or_else(|| format!("--eval-dir is required\n{USAGE}"))?,
        is_holdout_only,
        tier,
        candidate,
        is_accept_if_winning,
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
) -> Result<SliceResult, String> {
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
            if actual.kind != DispatchKind::Success {
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
                is_accepted: false,
                ts: ts.clone(),
                comparison_id: String::new(),
                incumbent_id: String::new(),
                incumbent_model_ran: Vec::new(),
                incumbent_scores_nonholdout: Vec::new(),
                incumbent_scores_holdout: Vec::new(),
                incumbent_repeat_scores_nonholdout: BTreeMap::new(),
                incumbent_repeat_scores_holdout: BTreeMap::new(),
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
fn evaluate_paired_arm(
    context: &EvalContext<'_>,
    tiers: &[String],
    nonholdout: &[&Case],
    holdout: &[&Case],
) -> Result<PairedArm, String> {
    let mut results = BTreeMap::new();
    for (index, tier) in tiers.iter().enumerate() {
        let judge_tier = tiers.get(index + 1).unwrap_or(tier);
        results.insert(
            tier.clone(),
            TierResult {
                nonholdout: run_slice(context, tier, judge_tier, nonholdout, "nonholdout")?,
                holdout: run_slice(context, tier, judge_tier, holdout, "holdout")?,
            },
        );
    }
    Ok(PairedArm {
        text: context.candidate.to_string(),
        id: candidate_id(context.candidate),
        repeats: context.settings.repeats,
        results,
    })
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
    if nonholdout.is_empty() || holdout.is_empty() {
        return Err("cases require non-holdout and holdout slices".to_string());
    }
    let tiers = load_tiers(&settings.tiers_file)?;
    if settings.args.is_accept_if_winning && is_floor_declared {
        for tier in &tiers {
            set_minimum_tier(&submitted, tier)?;
        }
    }
    let rubric = fs::read_to_string(eval_dir.join("rubric.md"))
        .map_err(|error| format!("cannot read rubric.md: {error}"))?;
    let temp = TempDir::create(&env::temp_dir(), "skill-eval")?;
    let wrapper = write_wrapper(&temp, &settings.auth_extension)?;
    let artifact_dir = eval_dir
        .parent()
        .ok_or_else(|| format!("{} has no parent", eval_dir.display()))?;
    let incumbent_context = EvalContext {
        settings: &settings,
        wrapper: &wrapper,
        temp: &temp,
        candidate: &incumbent,
        rubric: &rubric,
        eval_dir: &eval_dir,
        artifact_dir,
    };
    let candidate_context = EvalContext {
        settings: &settings,
        wrapper: &wrapper,
        temp: &temp,
        candidate: &candidate,
        rubric: &rubric,
        eval_dir: &eval_dir,
        artifact_dir,
    };
    let incumbent = evaluate_paired_arm(&incumbent_context, &tiers, &nonholdout, &holdout)?;
    let candidate = evaluate_paired_arm(&candidate_context, &tiers, &nonholdout, &holdout)?;
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
        Ok(2)
    } else if settings.args.is_accept_if_winning && !selection.is_accepted {
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
        let extension = temp.path.join("auth.ts");
        fs::write(&extension, "extension").unwrap();
        let dispatch = temp.path.join("tier-dispatch");
        write_executable(&dispatch, fake);
        let args = Args {
            eval_dir: eval_dir.clone(),
            is_holdout_only: false,
            tier: None,
            candidate: None,
            is_accept_if_winning: false,
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
}
