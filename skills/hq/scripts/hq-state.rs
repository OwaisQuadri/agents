use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::UNIX_EPOCH;

const STUCK_SECONDS: i64 = 2 * 60 * 60;
const SEED_LABELS: &[&str] = &["com.owaisquadri.ollama", "homebrew.mxcl.postgresql@18"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    pane_id: String,
    cwd: Option<String>,
    name: Option<String>,
    transcript: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Workspace {
    repo: String,
    workspace: String,
    path: String,
    branch: Option<String>,
    head_sha: Option<String>,
    last_touched_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    id: String,
    state: Option<String>,
    detail: Option<String>,
    state_mtime: Option<String>,
    timeline_mtime: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LaunchdJob {
    label: String,
    is_running: bool,
    pid: Option<i64>,
    last_exit: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    at: String,
    sessions: Vec<Session>,
    workspaces: Vec<Workspace>,
    jobs: Vec<Job>,
    launchd: Vec<LaunchdJob>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Change {
    at: String,
    kind: String,
    subject: String,
    before: Value,
    after: Value,
    detail: String,
}

#[derive(Deserialize, Serialize)]
struct Classification {
    routine: Vec<Change>,
    anomalies: Vec<Change>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EvidenceInput {
    One(String),
    Many(Vec<String>),
}

fn deserialize_evidence<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match EvidenceInput::deserialize(deserializer)? {
        EvidenceInput::One(value) => vec![value],
        EvidenceInput::Many(values) => values,
    })
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Gate {
    id: String,
    created_at: String,
    source: String,
    kind: String,
    subject: String,
    summary: String,
    #[serde(deserialize_with = "deserialize_evidence")]
    evidence: Vec<String>,
    urgency: String,
    is_resolved: bool,
    resolved_at: Option<String>,
    resolution: Option<String>,
}

#[derive(Deserialize)]
struct TriageOutput {
    digest: String,
    #[serde(default)]
    gates: Vec<Gate>,
    #[serde(default)]
    notify: Option<String>,
}

#[derive(Deserialize)]
struct HerdrEnvelope {
    result: HerdrResult,
}

#[derive(Deserialize)]
struct HerdrResult {
    snapshot: HerdrSnapshot,
}

#[derive(Deserialize)]
struct HerdrSnapshot {
    #[serde(default)]
    panes: Vec<HerdrPane>,
    #[serde(default)]
    workspaces: Vec<HerdrWorkspace>,
}

#[derive(Deserialize)]
struct HerdrPane {
    pane_id: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent_session: Option<HerdrAgentSession>,
    #[serde(default)]
    terminal_title_stripped: Option<String>,
}

#[derive(Deserialize)]
struct HerdrAgentSession {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    value: Option<String>,
}

#[derive(Deserialize)]
struct HerdrWorkspace {
    label: String,
    #[serde(default)]
    worktree: Option<HerdrWorktree>,
}

#[derive(Deserialize)]
struct HerdrWorktree {
    checkout_path: String,
    #[serde(default)]
    repo_name: Option<String>,
}

fn state_dir() -> PathBuf {
    env::var_os("HQ_STATE").map_or_else(|| home_dir().join(".pi/agent/state/hq"), PathBuf::from)
}

fn home_dir() -> PathBuf {
    env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

fn command_output(command: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(command).args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn now_iso() -> String {
    command_output("/bin/date", &["+%Y-%m-%dT%H:%M:%S%z"]).unwrap_or_default()
}

fn mtime_iso(path: &Path) -> Option<String> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let seconds = modified
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs()
        .to_string();
    command_output("/bin/date", &["-r", &seconds, "+%Y-%m-%dT%H:%M:%S%z"])
}

fn git_line(path: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new("/usr/bin/git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn herdr_snapshot() -> Option<HerdrSnapshot> {
    let herdr = env::var("HERDR_BIN").unwrap_or_else(|_| "herdr".to_string());
    let output = Command::new(herdr)
        .args(["api", "snapshot"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice::<HerdrEnvelope>(&output.stdout)
        .ok()
        .map(|envelope| envelope.result.snapshot)
}

fn probe_sessions(snapshot: &HerdrSnapshot) -> Vec<Session> {
    let mut sessions = snapshot
        .panes
        .iter()
        .filter_map(|pane| {
            let session = pane.agent_session.as_ref()?;
            if session.agent.as_deref() != Some("pi") {
                return None;
            }
            Some(Session {
                pane_id: pane.pane_id.clone(),
                cwd: pane.cwd.clone(),
                name: pane.terminal_title_stripped.clone(),
                transcript: session.value.clone()?,
            })
        })
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| left.pane_id.cmp(&right.pane_id));
    sessions
}

fn probe_workspaces(snapshot: &HerdrSnapshot) -> Vec<Workspace> {
    let mut workspaces = snapshot
        .workspaces
        .iter()
        .filter_map(|workspace| {
            let worktree = workspace.worktree.as_ref()?;
            Some(Workspace {
                repo: worktree
                    .repo_name
                    .clone()
                    .unwrap_or_else(|| workspace.label.clone()),
                workspace: workspace.label.clone(),
                path: worktree.checkout_path.clone(),
                branch: git_line(
                    &worktree.checkout_path,
                    &["rev-parse", "--abbrev-ref", "HEAD"],
                ),
                head_sha: git_line(&worktree.checkout_path, &["rev-parse", "--short", "HEAD"]),
                last_touched_at: mtime_iso(Path::new(&worktree.checkout_path)),
            })
        })
        .collect::<Vec<_>>();
    workspaces.sort_by(|left, right| left.path.cmp(&right.path));
    workspaces
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

const SCHEDULED_JOB_SOURCES: &[(&str, &[&str])] = &[
    ("gepa-due", &["gepa-due", "trigger.log"]),
    ("scheduled-ideation", &["scheduled-ideation", "trigger.log"]),
    ("worktree-hygiene", &["worktree-hygiene", "hygiene.log"]),
];

fn scheduled_job(root: &Path, id: &str, source: &[&str]) -> Job {
    let path = source
        .iter()
        .fold(root.to_path_buf(), |path, segment| path.join(segment));
    let is_recorded = path.exists();
    Job {
        id: id.to_string(),
        state: Some(
            if is_recorded {
                "recorded"
            } else {
                "unavailable"
            }
            .to_string(),
        ),
        detail: Some(path.display().to_string()),
        state_mtime: mtime_iso(&path),
        timeline_mtime: None,
    }
}

fn probe_jobs(root: &Path, own_state: &Path) -> Vec<Job> {
    let mut jobs = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().into_owned();
            if entry.path() == own_state
                || SCHEDULED_JOB_SOURCES
                    .iter()
                    .any(|(scheduled_id, _)| id == *scheduled_id)
            {
                continue;
            }
            let state_path = entry.path().join("state.json");
            let Some(value) = read_json(&state_path) else {
                continue;
            };
            let timeline_path = entry.path().join("timeline.jsonl");
            jobs.push(Job {
                id,
                state: value
                    .get("state")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                detail: value
                    .get("detail")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                state_mtime: mtime_iso(&state_path),
                timeline_mtime: mtime_iso(&timeline_path),
            });
        }
    }
    jobs.extend(
        SCHEDULED_JOB_SOURCES
            .iter()
            .map(|(id, source)| scheduled_job(root, id, source)),
    );
    jobs.sort_by(|left, right| left.id.cmp(&right.id));
    jobs
}

fn probe_launchd(state: &Path) -> Vec<LaunchdJob> {
    let labels = fs::read_to_string(state.join("watched-jobs.txt")).unwrap_or_default();
    labels
        .lines()
        .filter(|label| !label.is_empty())
        .map(|label| {
            let output = Command::new("/bin/launchctl")
                .args(["list", label])
                .output()
                .ok();
            let text = output
                .as_ref()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                .unwrap_or_default();
            let pid = launchd_integer(&text, "PID");
            LaunchdJob {
                label: label.to_string(),
                is_running: pid.is_some(),
                pid,
                last_exit: launchd_integer(&text, "LastExitStatus"),
            }
        })
        .collect()
}

fn launchd_integer(text: &str, key: &str) -> Option<i64> {
    text.lines().find_map(|line| {
        let (_, value) = line.split_once(key)?;
        let (_, value) = value.split_once('=')?;
        value.trim().parse().ok()
    })
}

fn build_snapshot() -> Result<Snapshot, String> {
    let snapshot = herdr_snapshot().ok_or_else(|| "herdr api snapshot failed".to_string())?;
    Ok(Snapshot {
        at: now_iso(),
        sessions: probe_sessions(&snapshot),
        workspaces: probe_workspaces(&snapshot),
        jobs: probe_jobs(&home_dir().join(".pi/agent/state"), &state_dir()),
        launchd: probe_launchd(&state_dir()),
    })
}

fn change(
    at: &str,
    kind: &str,
    subject: String,
    before: Value,
    after: Value,
    detail: &str,
) -> Change {
    Change {
        at: at.to_string(),
        kind: kind.to_string(),
        subject,
        before,
        after,
        detail: detail.to_string(),
    }
}

fn parse_iso_epoch(text: &str) -> Option<i64> {
    let year = text.get(0..4)?.parse::<i64>().ok()?;
    let month = text.get(5..7)?.parse::<i64>().ok()?;
    let day = text.get(8..10)?.parse::<i64>().ok()?;
    let hour = text.get(11..13)?.parse::<i64>().ok()?;
    let minute = text.get(14..16)?.parse::<i64>().ok()?;
    let second = text.get(17..19)?.parse::<i64>().ok()?;
    let sign = match text.get(19..20)? {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let offset_hour = text.get(20..22)?.parse::<i64>().ok()?;
    let offset_minute = text.get(22..24)?.parse::<i64>().ok()?;
    let adjusted_year = if month <= 2 { year - 1 } else { year };
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146097 + day_of_era - 719468;
    Some(
        days * 86_400 + hour * 3_600 + minute * 60 + second
            - sign * (offset_hour * 3_600 + offset_minute * 60),
    )
}

fn is_stuck(job: &Job, snapshot_at: &str) -> bool {
    let (Some(timeline), Some(now)) = (job.timeline_mtime.as_deref(), parse_iso_epoch(snapshot_at))
    else {
        return false;
    };
    job.state.as_deref() == Some("running")
        && parse_iso_epoch(timeline).is_some_and(|then| now - then > STUCK_SECONDS)
}

fn classify(previous: &Snapshot, current: &Snapshot) -> Classification {
    let mut routine = Vec::new();
    let mut anomalies = Vec::new();
    let previous_sessions = previous
        .sessions
        .iter()
        .map(|entry| (&entry.pane_id, entry))
        .collect::<BTreeMap<_, _>>();
    let current_sessions = current
        .sessions
        .iter()
        .map(|entry| (&entry.pane_id, entry))
        .collect::<BTreeMap<_, _>>();
    for (pane_id, session) in &current_sessions {
        if !previous_sessions.contains_key(pane_id) {
            routine.push(change(
                &current.at,
                "session_started",
                session.name.clone().unwrap_or_else(|| (*pane_id).clone()),
                Value::Null,
                json_string(session.cwd.as_deref()),
                "Pi session started",
            ));
        }
    }
    for (pane_id, session) in &previous_sessions {
        if !current_sessions.contains_key(pane_id) {
            routine.push(change(
                &current.at,
                "session_ended",
                session.name.clone().unwrap_or_else(|| (*pane_id).clone()),
                json_string(session.cwd.as_deref()),
                Value::Null,
                "Pi session ended",
            ));
        }
    }

    let previous_workspaces = previous
        .workspaces
        .iter()
        .map(|entry| (&entry.path, entry))
        .collect::<BTreeMap<_, _>>();
    for workspace in &current.workspaces {
        let old = previous_workspaces.get(&workspace.path);
        let subject = format!("{}/{}", workspace.repo, workspace.workspace);
        match old {
            None => routine.push(change(
                &current.at,
                "workspace_updated",
                subject,
                Value::Null,
                json_string(workspace.head_sha.as_deref()),
                "new workspace",
            )),
            Some(old) if old.head_sha != workspace.head_sha => routine.push(change(
                &current.at,
                "workspace_updated",
                subject,
                json_string(old.head_sha.as_deref()),
                json_string(workspace.head_sha.as_deref()),
                &format!(
                    "head moved on {}",
                    workspace.branch.as_deref().unwrap_or("unknown")
                ),
            )),
            Some(old) if old.last_touched_at != workspace.last_touched_at => routine.push(change(
                &current.at,
                "workspace_updated",
                subject,
                json_string(old.last_touched_at.as_deref()),
                json_string(workspace.last_touched_at.as_deref()),
                "workspace touched",
            )),
            _ => {}
        }
    }
    for workspace in &previous.workspaces {
        if !current
            .workspaces
            .iter()
            .any(|entry| entry.path == workspace.path)
        {
            routine.push(change(
                &current.at,
                "workspace_updated",
                format!("{}/{}", workspace.repo, workspace.workspace),
                json_string(workspace.head_sha.as_deref()),
                Value::Null,
                "workspace removed",
            ));
        }
    }

    let previous_jobs = previous
        .jobs
        .iter()
        .map(|entry| (&entry.id, entry))
        .collect::<BTreeMap<_, _>>();
    for job in &current.jobs {
        let old = previous_jobs.get(&job.id);
        let old_state = old.and_then(|entry| entry.state.as_deref());
        if old_state != job.state.as_deref() {
            let entry = change(
                &current.at,
                "job_state_changed",
                job.id.clone(),
                json_string(old_state),
                json_string(job.state.as_deref()),
                job.detail.as_deref().unwrap_or(""),
            );
            if job
                .state
                .as_deref()
                .is_some_and(|state| state.contains("failed") || state.contains("error"))
            {
                anomalies.push(entry);
            } else {
                routine.push(entry);
            }
        }
        let was_stuck = old.is_some_and(|entry| is_stuck(entry, &previous.at));
        if is_stuck(job, &current.at) && !was_stuck {
            anomalies.push(change(
                &current.at,
                "job_stuck",
                job.id.clone(),
                old.and_then(|entry| entry.timeline_mtime.as_deref())
                    .map_or(Value::Null, Value::from),
                json_string(job.timeline_mtime.as_deref()),
                "running with no timeline progress for over 2h",
            ));
        }
    }

    let previous_launchd = previous
        .launchd
        .iter()
        .map(|entry| (&entry.label, entry))
        .collect::<BTreeMap<_, _>>();
    for launchd in &current.launchd {
        let Some(old) = previous_launchd.get(&launchd.label) else {
            continue;
        };
        if old.is_running && !launchd.is_running {
            anomalies.push(change(
                &current.at,
                "launchd_down",
                launchd.label.clone(),
                old.pid.map_or(Value::Null, Value::from),
                Value::Null,
                "launchd job no longer running",
            ));
        } else if matches!(old.last_exit, Some(0) | None)
            && !matches!(launchd.last_exit, Some(0) | None)
        {
            anomalies.push(change(
                &current.at,
                "launchd_flapped",
                launchd.label.clone(),
                old.last_exit.map_or(Value::Null, Value::from),
                launchd.last_exit.map_or(Value::Null, Value::from),
                "launchd job exited nonzero",
            ));
        } else if !old.is_running && launchd.is_running {
            routine.push(change(
                &current.at,
                "job_state_changed",
                launchd.label.clone(),
                Value::from("down"),
                Value::from("running"),
                "launchd job back up",
            ));
        }
    }
    Classification { routine, anomalies }
}

fn json_string(value: Option<&str>) -> Value {
    value.map_or(Value::Null, Value::from)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn update_registry(state: &Path, sessions: &[Session]) -> Result<(), String> {
    let path = state.join("registry.json");
    let mut agents = read_json(&path)
        .and_then(|value| value.get("agents").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            let pane_id = entry.get("paneId").and_then(Value::as_str)?.to_string();
            Some((pane_id, entry))
        })
        .collect::<BTreeMap<_, _>>();
    let stamp = now_iso();
    for session in sessions {
        let mut entry = serde_json::Map::new();
        entry.insert(
            "name".into(),
            Value::from(
                session
                    .name
                    .clone()
                    .unwrap_or_else(|| session.pane_id.clone()),
            ),
        );
        entry.insert("cwd".into(), json_string(session.cwd.as_deref()));
        entry.insert("paneId".into(), Value::from(session.pane_id.clone()));
        entry.insert("isLive".into(), Value::Bool(true));
        entry.insert("transcript".into(), Value::from(session.transcript.clone()));
        entry.insert("lastSeenAt".into(), Value::from(stamp.clone()));
        agents.insert(session.pane_id.clone(), Value::Object(entry));
    }
    let live = sessions
        .iter()
        .map(|session| session.pane_id.as_str())
        .collect::<Vec<_>>();
    for (pane, entry) in &mut agents {
        if !live.contains(&pane.as_str()) {
            entry["isLive"] = Value::Bool(false);
        }
    }
    let agents = agents.into_values().collect::<Vec<_>>();
    write_json(
        &path,
        &serde_json::json!({ "updatedAt": stamp, "agents": agents }),
    )
}

fn update_state(state: &Path, key: &str, value: Value) -> Result<(), String> {
    let path = state.join("state.json");
    let mut value_state = read_json(&path).unwrap_or_else(|| Value::Object(Default::default()));
    let Some(object) = value_state.as_object_mut() else {
        return Err(format!("{} must contain a JSON object", path.display()));
    };
    object.insert(key.to_string(), value);
    write_json(&path, &value_state)
}

fn scan() -> Result<(), String> {
    let state = state_dir();
    fs::create_dir_all(state.join("snapshots")).map_err(|error| error.to_string())?;
    fs::create_dir_all(state.join("gates/resolved")).map_err(|error| error.to_string())?;
    let labels = state.join("watched-jobs.txt");
    if !labels.exists() {
        fs::write(&labels, format!("{}\n", SEED_LABELS.join("\n")))
            .map_err(|error| error.to_string())?;
    }
    let snapshot = build_snapshot()?;
    update_registry(&state, &snapshot.sessions)?;
    let latest = state.join("snapshots/latest.json");
    let previous_path = state.join("snapshots/previous.json");
    let previous = if latest.exists() {
        let value = fs::read(&latest).map_err(|error| error.to_string())?;
        let previous =
            serde_json::from_slice::<Snapshot>(&value).map_err(|error| error.to_string())?;
        fs::rename(&latest, &previous_path).map_err(|error| error.to_string())?;
        Some(previous)
    } else {
        None
    };
    write_json(&latest, &snapshot)?;
    update_state(&state, "lastScanAt", Value::from(snapshot.at.clone()))?;
    let Some(previous) = previous else {
        return Ok(());
    };
    let result = classify(&previous, &snapshot);
    if !result.routine.is_empty() {
        let mut lines = String::new();
        for entry in result.routine {
            lines.push_str(&serde_json::to_string(&entry).map_err(|error| error.to_string())?);
            lines.push('\n');
        }
        append(&state.join("activity.jsonl"), &lines)?;
    }
    if !result.anomalies.is_empty() {
        write_json(
            &state.join("delta.json"),
            &serde_json::json!({ "at": snapshot.at, "changes": result.anomalies }),
        )?;
        println!("delta");
    }
    Ok(())
}

fn append(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(text.as_bytes())
        .map_err(|error| error.to_string())
}

fn triage_due() -> Result<bool, String> {
    let state = state_dir();
    let delta = state.join("delta.json");
    let delta_mtime = fs::metadata(delta)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| error.to_string())?;
    let last_triage = read_json(&state.join("state.json")).and_then(|value| {
        value
            .get("lastTriageAt")
            .and_then(Value::as_str)
            .and_then(parse_iso_epoch)
    });
    let delta_epoch = delta_mtime
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs() as i64;
    Ok(last_triage.is_none_or(|last| delta_epoch > last))
}

fn mark_triaged() -> Result<(), String> {
    update_state(&state_dir(), "lastTriageAt", Value::from(now_iso()))
}

fn is_safe_gate_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || character == '-'
                || character == '_'
                || character == '.'
        })
}

fn parse_triage_output(output: &[u8]) -> Result<TriageOutput, String> {
    let text = std::str::from_utf8(output)
        .map_err(|error| format!("triage output must be UTF-8: {error}"))?;
    let (marker, start) = ["```json\n", "```\n"]
        .into_iter()
        .filter_map(|marker| text.find(marker).map(|start| (marker, start)))
        .min_by_key(|(_, start)| *start)
        .ok_or_else(|| "triage output must contain one fenced JSON object".to_string())?;
    let body_start = start + marker.len();
    let body_end = text[body_start..]
        .find("\n```")
        .map(|end| body_start + end)
        .ok_or_else(|| "triage output must contain one fenced JSON object".to_string())?;
    serde_json::from_str(&text[body_start..body_end])
        .map_err(|error| format!("triage output must be JSON: {error}"))
}

fn apply_triage(path: &str) -> Result<(), String> {
    let output = fs::read(path).map_err(|error| error.to_string())?;
    let triage = parse_triage_output(&output)?;
    let state = state_dir();
    for gate in &triage.gates {
        if !is_safe_gate_id(&gate.id) {
            return Err(format!("invalid gate id: {}", gate.id));
        }
        if gate.is_resolved {
            return Err(format!("new gate {} cannot be resolved", gate.id));
        }
        write_json(&state.join("gates").join(format!("{}.json", gate.id)), gate)?;
    }
    fs::write(state.join("digest.md"), triage.digest).map_err(|error| error.to_string())?;
    if let Some(notification) = triage.notify {
        println!("NOTIFY:{notification}");
    }
    Ok(())
}

fn usage() {
    eprintln!(
        "usage: hq-state [--classify <previous.json|-> <current.json> | --triage-due | --apply-triage <output.json> | --mark-triaged]"
    );
}

fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let result = match arguments.as_slice() {
        [] => scan(),
        [flag, previous, current] if flag == "--classify" => {
            let current = fs::read(current)
                .map_err(|error| error.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<Snapshot>(&bytes).map_err(|error| error.to_string())
                });
            match current {
                Ok(_current) if previous == "-" => Ok(()),
                Ok(current) => fs::read(previous)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        serde_json::from_slice::<Snapshot>(&bytes)
                            .map_err(|error| error.to_string())
                    })
                    .and_then(|previous| {
                        let result = classify(&previous, &current);
                        if result.routine.is_empty() && result.anomalies.is_empty() {
                            Ok(())
                        } else {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&result)
                                    .map_err(|error| error.to_string())?
                            );
                            Ok(())
                        }
                    }),
                Err(error) => Err(error),
            }
        }
        [flag] if flag == "--triage-due" => triage_due().map(|is_due| {
            if is_due {
                println!("due");
            }
        }),
        [flag, output] if flag == "--apply-triage" => apply_triage(output),
        [flag] if flag == "--mark-triaged" => mark_triaged(),
        _ => {
            usage();
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hq-state: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(at: &str, sessions: Vec<Session>) -> Snapshot {
        Snapshot {
            at: at.into(),
            sessions,
            workspaces: Vec::new(),
            jobs: Vec::new(),
            launchd: Vec::new(),
        }
    }

    #[test]
    fn classifies_an_ended_pi_pane_without_a_resume_identifier() {
        let previous = snapshot(
            "2026-08-04T09:00:00-0400",
            vec![Session {
                pane_id: "w1:p2".into(),
                cwd: Some("/tmp/project".into()),
                name: Some("project".into()),
                transcript: "/tmp/session.jsonl".into(),
            }],
        );
        let current = snapshot("2026-08-04T09:30:00-0400", Vec::new());
        let result = classify(&previous, &current);
        assert_eq!(result.routine.len(), 1);
        assert_eq!(result.routine[0].kind, "session_ended");
        assert!(result.anomalies.is_empty());
    }

    #[test]
    fn finds_only_pi_transcripts_advertised_by_herdr() {
        let snapshot = HerdrSnapshot {
            panes: vec![
                HerdrPane {
                    pane_id: "w1:p1".into(),
                    cwd: None,
                    agent_session: Some(HerdrAgentSession {
                        agent: Some("pi".into()),
                        value: Some("/tmp/pi.jsonl".into()),
                    }),
                    terminal_title_stripped: None,
                },
                HerdrPane {
                    pane_id: "w1:p2".into(),
                    cwd: None,
                    agent_session: Some(HerdrAgentSession {
                        agent: Some("other".into()),
                        value: Some("/tmp/other.jsonl".into()),
                    }),
                    terminal_title_stripped: None,
                },
            ],
            workspaces: Vec::new(),
        };
        let sessions = probe_sessions(&snapshot);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].transcript, "/tmp/pi.jsonl");
    }

    fn test_dir(name: &str) -> PathBuf {
        let path = env::temp_dir().join(format!("hq-state-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn excludes_its_own_state_directory_from_jobs() {
        let root = test_dir("own-state");
        let own_state = root.join("hq");
        fs::create_dir_all(&own_state).unwrap();
        fs::write(own_state.join("state.json"), r#"{"state":"running"}"#).unwrap();

        let jobs = probe_jobs(&root, &own_state);

        assert!(!jobs.iter().any(|job| job.id == "hq"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn observes_the_three_persisted_scheduled_job_sources() {
        let root = test_dir("scheduled-sources");
        for (_, source) in SCHEDULED_JOB_SOURCES {
            let path = source
                .iter()
                .fold(root.clone(), |path, segment| path.join(segment));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "recorded").unwrap();
        }

        let jobs = probe_jobs(&root, &root.join("hq"));

        for (id, _) in SCHEDULED_JOB_SOURCES {
            let job = jobs.iter().find(|job| job.id == *id).unwrap();
            assert_eq!(job.state.as_deref(), Some("recorded"));
            assert!(job.state_mtime.is_some());
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_a_fenced_json_triage_response_with_model_prose() {
        let output = br#"Here is the result:
```json
{"digest":"digest","gates":[{"id":"com.example.job","createdAt":"now","source":"heartbeat","kind":"launchd_down","subject":"job","summary":"down","evidence":"pid changed","urgency":"normal","isResolved":false,"resolvedAt":null,"resolution":null}],"notify":null}
```
Done."#;

        let triage = parse_triage_output(output).unwrap();
        assert_eq!(triage.digest, "digest");
        assert_eq!(triage.gates[0].evidence, ["pid changed"]);
        assert!(is_safe_gate_id(&triage.gates[0].id));
        assert!(parse_triage_output(br#"{"digest":"digest"}"#).is_err());
        assert!(parse_triage_output(b"```json\nnot json\n```").is_err());
    }
}
