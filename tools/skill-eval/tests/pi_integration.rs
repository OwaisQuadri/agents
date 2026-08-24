use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const OPT_IN: &str = "SKILL_EVAL_H4_REAL_PI";
const CREDENTIAL: &str = "OPENROUTER_API_KEY";
const CANDIDATE_PROVIDER: &str = "openrouter";
const CANDIDATE_MODEL: &str = "nvidia/nemotron-nano-9b-v2:free";
const JUDGE_PROVIDER: &str = "openrouter";
const JUDGE_MODEL: &str = "liquid/lfm-2.5-2.6b:free";

// TODO(AGNT-0032.T15): Prove one bounded real Pi qualification after provider capacity returns.
#[test]
#[ignore = "real Pi execution requires SKILL_EVAL_H4_REAL_PI=1 and OPENROUTER_API_KEY"]
fn ordinary_skill_trial() {
    if env::var(OPT_IN).as_deref() != Ok("1") {
        eprintln!("SKIP H4: SKILL_EVAL_H4_REAL_PI=1 is not set");
        return;
    }
    if env::var_os(CREDENTIAL).is_none() {
        eprintln!("SKIP H4: OPENROUTER_API_KEY is not set");
        return;
    }

    let root = TemporaryRoot::new();
    let home = root.path().join("home");
    let pi_directory = root.path().join("pi");
    let sessions = root.path().join("sessions");
    let cache = root.path().join("cache");
    let data = root.path().join("data");
    let temporary = root.path().join("tmp");
    let repository = root.path().join("repository");
    let runs = root.path().join("runs");
    for directory in [
        &home,
        &pi_directory,
        &sessions,
        &cache,
        &data,
        &temporary,
        &repository,
        &runs,
    ] {
        fs::create_dir_all(directory).unwrap();
    }

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/integration/skill");
    let skill = repository.join("skills/h4-real-pi");
    copy_directory(&fixture, &skill);
    let configuration = repository.join("config/model-tiers.json");
    fs::create_dir_all(configuration.parent().unwrap()).unwrap();
    fs::write(&configuration, routing_configuration()).unwrap();

    let sandbox = Sandbox {
        working_directory: &repository,
        home: &home,
        pi_directory: &pi_directory,
        sessions: &sessions,
        cache: &cache,
        data: &data,
        temporary: &temporary,
    };
    let tracked_before = tracked_state();
    let catalog = sandbox.command("pi", ["--list-models"]);
    assert!(
        catalog.status.success(),
        "isolated Pi catalog failed: {}",
        String::from_utf8_lossy(&catalog.stderr)
    );
    let catalog = String::from_utf8(catalog.stdout).unwrap();
    let is_candidate_available = catalog.lines().any(|line| {
        let mut columns = line.split_whitespace();
        columns.next() == Some(CANDIDATE_PROVIDER) && columns.next() == Some(CANDIDATE_MODEL)
    });
    let is_judge_available = catalog.lines().any(|line| {
        let mut columns = line.split_whitespace();
        columns.next() == Some(JUDGE_PROVIDER) && columns.next() == Some(JUDGE_MODEL)
    });
    if !is_candidate_available || !is_judge_available {
        eprintln!(
            "SKIP H4: required models are unavailable: {CANDIDATE_PROVIDER}/{CANDIDATE_MODEL}, {JUDGE_PROVIDER}/{JUDGE_MODEL}"
        );
        return;
    }

    let run_id_file = root.path().join("run-id");
    let output = sandbox.command(
        env!("CARGO_BIN_EXE_skill-eval"),
        [
            "qualify",
            "--skill",
            skill.to_str().unwrap(),
            "--start-tier",
            "T1",
            "--reference-tier",
            "T4",
            "--trials",
            "1",
            "--minimum-score",
            "0",
            "--noninferiority-margin",
            "10",
            "--confidence",
            "0.5",
            "--run-id-file",
            run_id_file.to_str().unwrap(),
            "--runs-root",
            runs.to_str().unwrap(),
            "--format",
            "jsonl",
        ],
    );
    assert!(
        output.status.success(),
        "real qualification failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(tracked_state(), tracked_before);

    let progress = json_lines(&output.stdout);
    assert!(!progress.is_empty());
    let run_id = fs::read_to_string(run_id_file).unwrap();
    let run_id = run_id.trim();
    let run_directory = runs.join(run_id);
    let event_path = run_directory.join("events.jsonl");
    let events = json_lines(&fs::read(&event_path).unwrap());
    let event_names = events
        .iter()
        .map(|event| event["event"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(event_names.first(), Some(&"run_started"));
    assert!(matches!(
        event_names.last(),
        Some(&"boundary_found") | Some(&"review_required")
    ));
    let trial_events = &event_names[1..event_names.len() - 1];
    assert!(!trial_events.is_empty());
    assert_eq!(trial_events.len() % 4, 0);
    for trial in trial_events.chunks_exact(4) {
        assert_eq!(
            trial,
            [
                "trial_started",
                "candidate_executed",
                "trial_completed",
                "tier_evaluated",
            ]
        );
    }

    let trial = events
        .iter()
        .find(|event| event["event"] == "trial_completed" && event["record"]["key"]["tier"] == "t1")
        .unwrap();
    let record = &trial["record"];
    let artifact = PathBuf::from(record["artifact_path"].as_str().unwrap());
    let transcript = PathBuf::from(record["transcript_path"].as_str().unwrap());
    let canonical_runs = fs::canonicalize(&runs).unwrap();
    assert!(artifact.is_file());
    assert!(transcript.is_file());
    assert!(artifact.starts_with(&canonical_runs));
    assert!(transcript.starts_with(&canonical_runs));
    assert_eq!(
        fs::read_to_string(&artifact).unwrap().trim(),
        "H4 fixture complete"
    );
    assert_eq!(
        fs::read_to_string(artifact.parent().unwrap().join("fixture/result.txt"))
            .unwrap()
            .trim(),
        "H4 disposable result"
    );

    assert_eq!(record["model"]["provider"], CANDIDATE_PROVIDER);
    assert_eq!(record["model"]["model"], CANDIDATE_MODEL);
    assert!(
        record["candidate_usage"]["tool_calls"]
            .as_u64()
            .is_some_and(|calls| calls >= 2)
    );
    assert!(token_count(&record["candidate_usage"]) > 0);
    assert!(
        record["candidate_usage"]["elapsed_milliseconds"]
            .as_u64()
            .is_some_and(|elapsed| elapsed > 0)
    );
    assert_ne!(
        (
            record["model"]["provider"].as_str(),
            record["model"]["model"].as_str(),
        ),
        (
            record["judge_model"]["provider"].as_str(),
            record["judge_model"]["model"].as_str(),
        )
    );
    assert!(token_count(&record["judge_usage"]) > 0);
    assert!(
        record["judge_usage"]["elapsed_milliseconds"]
            .as_u64()
            .is_some_and(|elapsed| elapsed > 0)
    );

    let transcript_events = json_lines(&fs::read(&transcript).unwrap());
    assert_eq!(
        transcript_events
            .iter()
            .filter(|event| event["type"] == "tool_execution_start")
            .count(),
        1
    );
    assert!(transcript_events.iter().any(|event| {
        event["type"] == "message_end" && event["message"]["role"] == "assistant"
    }));
    assert!(progress.iter().all(Value::is_object));
}

fn routing_configuration() -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "tiers": {
            "T1": route(CANDIDATE_PROVIDER, CANDIDATE_MODEL, "low"),
            "T2": route(CANDIDATE_PROVIDER, CANDIDATE_MODEL, "low"),
            "T3": route(CANDIDATE_PROVIDER, CANDIDATE_MODEL, "low"),
            "T4": route(CANDIDATE_PROVIDER, CANDIDATE_MODEL, "low"),
            "T5": route(JUDGE_PROVIDER, JUDGE_MODEL, "low")
        },
        "judge": "T5"
    }))
    .unwrap()
}

fn route(provider: &str, model: &str, thinking: &str) -> Value {
    json!({
        "pi": format!("{provider}/{model}"),
        "fallbacks": [],
        "thinking": thinking
    })
}

struct Sandbox<'a> {
    working_directory: &'a Path,
    home: &'a Path,
    pi_directory: &'a Path,
    sessions: &'a Path,
    cache: &'a Path,
    data: &'a Path,
    temporary: &'a Path,
}

impl Sandbox<'_> {
    fn command<I, S>(&self, program: &str, arguments: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Command::new(program)
            .args(arguments)
            .current_dir(self.working_directory)
            .env("HOME", self.home)
            .env("PI_CODING_AGENT_DIR", self.pi_directory)
            .env("PI_CODING_AGENT_SESSION_DIR", self.sessions)
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("XDG_CACHE_HOME", self.cache)
            .env("XDG_DATA_HOME", self.data)
            .env("TMPDIR", self.temporary)
            .env("PI_SKIP_VERSION_CHECK", "1")
            .env("PI_TELEMETRY", "0")
            .output()
            .unwrap()
    }
}

fn json_lines(bytes: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn token_count(usage: &Value) -> u64 {
    [
        "input_tokens",
        "output_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
    ]
    .iter()
    .map(|field| usage[*field].as_u64().unwrap())
    .sum()
}

fn tracked_state() -> (Vec<u8>, Vec<u8>) {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let unstaged = Command::new("git")
        .args(["-C", repository.to_str().unwrap(), "diff", "--binary", "--"])
        .output()
        .unwrap();
    let staged = Command::new("git")
        .args([
            "-C",
            repository.to_str().unwrap(),
            "diff",
            "--binary",
            "--cached",
            "--",
        ])
        .output()
        .unwrap();
    assert!(unstaged.status.success());
    assert!(staged.status.success());
    (unstaged.stdout, staged.stdout)
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().unwrap();
        assert!(!file_type.is_symlink());
        if file_type.is_dir() {
            copy_directory(&source_path, &destination_path);
        } else {
            assert!(file_type.is_file());
            fs::copy(source_path, destination_path).unwrap();
        }
    }
}

struct TemporaryRoot(PathBuf);

impl TemporaryRoot {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("skill-eval-h4-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryRoot {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
