use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;
use serde_json::{Value, json};

const HASH_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const HASH_PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Deserialize)]
struct Case {
    id: String,
    input: String,
    expect: String,
    source: String,
    #[serde(rename = "holdout")]
    is_holdout: bool,
    #[serde(default)]
    files: Vec<PathBuf>,
    execution: Execution,
}

#[derive(Deserialize)]
struct Execution {
    drive: FixtureDrive,
    allowed_tools: Vec<String>,
    timeout_seconds: u32,
}

#[derive(Deserialize)]
struct FixtureDrive {
    kind: String,
    source: PathBuf,
    verify_commands: Vec<VerifyCommand>,
}

#[derive(Deserialize)]
struct VerifyCommand {
    program: String,
    arguments: Vec<String>,
    working_directory: PathBuf,
}

#[derive(Clone, Deserialize)]
struct CheckSpec {
    id: String,
    required_reads: Vec<PathBuf>,
    required_writes: Vec<PathBuf>,
    maximum_tool_calls: usize,
}

struct Exam {
    root: PathBuf,
    revision: String,
    cases: Vec<Case>,
    checks: BTreeMap<String, CheckSpec>,
}

impl Exam {
    fn load(root: &Path) -> Self {
        let root = fs::canonicalize(root).unwrap();
        let cases = fs::read_to_string(root.join("evals/cases.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect::<Vec<_>>();
        let checks = serde_json::from_slice::<Vec<CheckSpec>>(
            &fs::read(root.join("evals/checks.json")).unwrap(),
        )
        .unwrap()
        .into_iter()
        .map(|check| (check.id.clone(), check))
        .collect();
        let revision = fs::read_to_string(root.join("REVISION"))
            .unwrap()
            .trim()
            .to_owned();
        Self {
            root,
            revision,
            cases,
            checks,
        }
    }

    fn current_revision(&self) -> String {
        artifact_revision(&self.root, &self.cases)
    }
}

#[derive(Clone)]
struct ToolCall {
    action: &'static str,
    path: PathBuf,
}

struct CandidateOutput {
    response: String,
    calls: Vec<ToolCall>,
}

#[derive(Default)]
struct FakeCandidate {
    launches: usize,
}

impl FakeCandidate {
    fn execute(&mut self, case: &Case, fixture: &Path) -> CandidateOutput {
        self.launches += 1;
        let mut calls = Vec::new();
        match case.id.as_str() {
            "bounded-analysis" => execute_analysis(fixture, &mut calls),
            "code-repair" => execute_repair(fixture, &mut calls),
            "instruction-following" => execute_instructions(fixture, &mut calls),
            "tool-use" => execute_tool_use(fixture, &mut calls),
            "structured-output" => execute_structured_output(fixture, &mut calls),
            unknown => panic!("unknown calibration case {unknown}"),
        }
        CandidateOutput {
            response: "Completed.".to_owned(),
            calls,
        }
    }
}

struct CheckOutcome {
    name: &'static str,
    is_passed: bool,
}

struct BlindSubmission<'a> {
    response: &'a str,
    fixture: &'a Path,
    checks: &'a [CheckOutcome],
}

#[derive(Debug)]
struct Verdict {
    score: u8,
    is_catastrophic: bool,
}

#[derive(Default)]
struct FakeJudge {
    calls: usize,
    packets: Vec<String>,
}

impl FakeJudge {
    fn grade(&mut self, submission: BlindSubmission<'_>) -> Verdict {
        self.calls += 1;
        let packet = json!({
            "response": submission.response,
            "fixture": fixture_outputs(submission.fixture),
            "checks": submission
                .checks
                .iter()
                .map(|check| json!({"name": check.name, "is_passed": check.is_passed}))
                .collect::<Vec<_>>()
        })
        .to_string();
        let is_all_passed = submission.checks.iter().all(|check| check.is_passed);
        self.packets.push(packet);
        Verdict {
            score: if is_all_passed { 10 } else { 4 },
            is_catastrophic: false,
        }
    }
}

#[test]
fn fixed_exam_executes_every_case_in_disposable_fixtures() {
    let exam = Exam::load(&exam_root());
    assert_eq!(exam.current_revision(), exam.revision);
    assert_eq!(exam.cases.len(), 5);
    assert_eq!(exam.checks.len(), exam.cases.len());
    assert_eq!(
        exam.cases
            .iter()
            .map(|case| case.id.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "bounded-analysis",
            "code-repair",
            "instruction-following",
            "structured-output",
            "tool-use",
        ])
    );

    let original = snapshot(&exam.root);
    let temporary = TemporaryRoot::new("execute");
    let mut candidate = FakeCandidate::default();
    let mut judge = FakeJudge::default();

    for case in &exam.cases {
        let trial = temporary.path().join(&case.id);
        let result = run_case(&exam, case, &trial, &mut candidate, &mut judge).unwrap();
        assert_eq!(result.score, 10, "{} did not receive full credit", case.id);
        assert!(!result.is_catastrophic);
        assert!(trial.starts_with(temporary.path()));
        assert!(!trial.starts_with(&exam.root));
    }

    assert_eq!(candidate.launches, exam.cases.len());
    assert_eq!(judge.calls, exam.cases.len());
    assert_eq!(snapshot(&exam.root), original);
    assert!(judge.packets.iter().all(|packet| {
        !packet.contains("hidden-candidate-identity") && !packet.contains("hidden-judge-identity")
    }));
}

#[test]
fn exam_is_bounded_and_contains_no_disallowed_subjects() {
    let exam = Exam::load(&exam_root());
    let forbidden = [
        "api key",
        "authorization bypass",
        "credential",
        "exploit",
        "password",
        "private key",
        "production system",
        "live system",
        "sudo",
        "/etc/",
        "http://",
        "https://",
        "openai",
        "anthropic",
        "openrouter",
        "claude",
        "codex",
        "gemini",
        "gpt-",
        "llama",
        "mistral",
    ];

    for case in &exam.cases {
        let checked_text = format!("{}\n{}", case.input, case.expect).to_ascii_lowercase();
        for term in forbidden {
            assert!(
                !checked_text.contains(term),
                "case {} contains disallowed text {term:?}",
                case.id
            );
        }
        assert_eq!(case.source, "fixed-local-exam");
        assert!(!case.is_holdout);
        assert_eq!(case.execution.drive.kind, "fixture");
        assert!(!case.execution.drive.verify_commands.is_empty());
        assert_eq!(case.execution.timeout_seconds, 90);
        assert!(
            case.execution
                .allowed_tools
                .iter()
                .all(|tool| matches!(tool.as_str(), "read" | "write" | "edit"))
        );
        let check = exam.checks.get(&case.id).unwrap();
        assert!(check.maximum_tool_calls <= 5);
        assert!(!check.required_reads.is_empty());
        assert!(!check.required_writes.is_empty());
    }
}

#[test]
fn verify_commands_accept_expected_outputs_and_reject_targeted_mutations() {
    let exam = Exam::load(&exam_root());
    let temporary = TemporaryRoot::new("verify");
    let mut candidate = FakeCandidate::default();

    for case in &exam.cases {
        let trial = temporary.path().join(&case.id);
        copy_directory(&exam.root.join(&case.execution.drive.source), &trial);
        candidate.execute(case, &trial);

        for command in &case.execution.drive.verify_commands {
            let output = run_verify_command(case, command, &trial);
            assert!(output.status.success(), "{}: {output:?}", case.id);
        }

        mutate_expected_output(&case.id, &trial);
        for command in &case.execution.drive.verify_commands {
            let output = run_verify_command(case, command, &trial);
            assert!(!output.status.success(), "{}: {output:?}", case.id);
        }
    }
}

#[test]
fn frozen_revision_drift_blocks_launch_and_judging() {
    let source = exam_root();
    let temporary = TemporaryRoot::new("revision");
    let copied_exam = temporary.path().join("exam");
    copy_directory(&source, &copied_exam);
    let exam = Exam::load(&copied_exam);
    let frozen_revision = exam.revision.clone();
    assert_eq!(exam.current_revision(), frozen_revision);

    fs::write(
        copied_exam.join("fixtures/bounded-analysis/observations.csv"),
        "unit,samples,errors\ncedar,20,3\nbirch,8,4\nmaple,40,11\nspruce,50,5\nash,10,2\n",
    )
    .unwrap();
    let changed_exam = Exam::load(&copied_exam);
    let mut candidate = FakeCandidate::default();
    let mut judge = FakeJudge::default();
    let trial = temporary.path().join("blocked-trial");

    let drift = run_case(
        &changed_exam,
        &changed_exam.cases[0],
        &trial,
        &mut candidate,
        &mut judge,
    )
    .unwrap_err();

    assert_eq!(drift.expected, frozen_revision);
    assert_eq!(drift.actual, changed_exam.current_revision());
    assert_ne!(drift.expected, drift.actual);
    assert_eq!(candidate.launches, 0);
    assert_eq!(judge.calls, 0);
    assert!(!trial.exists());
}

#[derive(Debug)]
struct RevisionDrift {
    expected: String,
    actual: String,
}

fn run_case(
    exam: &Exam,
    case: &Case,
    trial: &Path,
    candidate: &mut FakeCandidate,
    judge: &mut FakeJudge,
) -> Result<Verdict, RevisionDrift> {
    let actual = exam.current_revision();
    if actual != exam.revision {
        return Err(RevisionDrift {
            expected: exam.revision.clone(),
            actual,
        });
    }

    let source = exam.root.join(&case.execution.drive.source);
    copy_directory(&source, trial);
    let initial_files = snapshot(trial);
    let output = candidate.execute(case, trial);
    let check = exam.checks.get(&case.id).unwrap();
    let checks = deterministic_checks(case, check, trial, &initial_files, &output);
    assert!(checks.iter().all(|check| check.is_passed), "{}", case.id);
    Ok(judge.grade(BlindSubmission {
        response: &output.response,
        fixture: trial,
        checks: &checks,
    }))
}

fn deterministic_checks(
    case: &Case,
    spec: &CheckSpec,
    fixture: &Path,
    initial_files: &BTreeMap<PathBuf, Vec<u8>>,
    output: &CandidateOutput,
) -> Vec<CheckOutcome> {
    let reads = output
        .calls
        .iter()
        .filter(|call| call.action == "read")
        .map(|call| call.path.clone())
        .collect::<BTreeSet<_>>();
    let writes = output
        .calls
        .iter()
        .filter(|call| call.action != "read")
        .map(|call| call.path.clone())
        .collect::<BTreeSet<_>>();
    let required_reads = spec.required_reads.iter().cloned().collect::<BTreeSet<_>>();
    let required_writes = spec
        .required_writes
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let current_files = snapshot(fixture);
    let changed_files = changed_files(initial_files, &current_files);

    vec![
        CheckOutcome {
            name: "bounded declared tools",
            is_passed: reads == required_reads
                && writes == required_writes
                && output.calls.len() <= spec.maximum_tool_calls,
        },
        CheckOutcome {
            name: "requested file scope",
            is_passed: changed_files == required_writes,
        },
        CheckOutcome {
            name: "case result",
            is_passed: case
                .execution
                .drive
                .verify_commands
                .iter()
                .all(|command| run_verify_command(case, command, fixture).status.success()),
        },
        CheckOutcome {
            name: "fixture drive",
            is_passed: case.execution.drive.kind == "fixture" && fixture.is_dir(),
        },
    ]
}

fn run_verify_command(case: &Case, command: &VerifyCommand, fixture: &Path) -> Output {
    assert_eq!(command.working_directory, case.execution.drive.source);
    Command::new(&command.program)
        .args(&command.arguments)
        .current_dir(fixture)
        .output()
        .unwrap()
}

fn mutate_expected_output(case_id: &str, fixture: &Path) {
    let (path, content) = match case_id {
        "bounded-analysis" => ("answer.json", r#"{"highest":[]}"#),
        "code-repair" => (
            "widget.rs",
            "pub fn bounded_add(left: i32, right: i32, limit: i32) -> i32 {\n    (left - right).min(limit)\n}\n",
        ),
        "instruction-following" => ("answer.txt", "RIDGE-41-violet\n"),
        "tool-use" => ("selected.txt", "fern\n"),
        "structured-output" => ("summary.json", r#"{"accepted":3,"rejected":2,"total":4}"#),
        unknown => panic!("unknown calibration case {unknown}"),
    };
    fs::write(fixture.join(path), content).unwrap();
}

fn execute_analysis(fixture: &Path, calls: &mut Vec<ToolCall>) {
    let input = read_fixture(fixture, "observations.csv", calls);
    let mut ranked = input
        .lines()
        .skip(1)
        .filter_map(|line| {
            let columns = line.split(',').collect::<Vec<_>>();
            let samples = columns[1].parse::<u32>().unwrap();
            let errors = columns[2].parse::<u32>().unwrap();
            (samples >= 10).then(|| (columns[0], f64::from(errors) / f64::from(samples)))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.partial_cmp(&left.1).unwrap());
    let highest = ranked
        .into_iter()
        .take(2)
        .map(|(unit, rate)| json!({"unit": unit, "rate": format!("{rate:.2}")}))
        .collect::<Vec<_>>();
    write_fixture(
        fixture,
        "answer.json",
        serde_json::to_string(&json!({"highest": highest})).unwrap(),
        "write",
        calls,
    );
}

fn execute_repair(fixture: &Path, calls: &mut Vec<ToolCall>) {
    let input = read_fixture(fixture, "widget.rs", calls);
    let repaired = input.replace("left - right", "left + right");
    assert_ne!(input, repaired);
    write_fixture(fixture, "widget.rs", repaired, "edit", calls);
}

fn execute_instructions(fixture: &Path, calls: &mut Vec<ToolCall>) {
    let input = read_fixture(fixture, "rules.txt", calls);
    let tokens = input
        .lines()
        .next()
        .unwrap()
        .strip_prefix("tokens: ")
        .unwrap()
        .split_whitespace()
        .collect::<Vec<_>>();
    let answer = format!("{}-{}-{}\n", tokens[1].to_uppercase(), tokens[2], tokens[0]);
    write_fixture(fixture, "answer.txt", answer, "write", calls);
}

fn execute_tool_use(fixture: &Path, calls: &mut Vec<ToolCall>) {
    let mut selected = (String::new(), 0_u32);
    for path in ["inbox/a.txt", "inbox/b.txt", "inbox/c.txt"] {
        let record = read_fixture(fixture, path, calls);
        let fields = record
            .lines()
            .map(|line| line.split_once('=').unwrap())
            .collect::<BTreeMap<_, _>>();
        let count = fields["count"].parse::<u32>().unwrap();
        if fields["state"] == "ready" && count > selected.1 {
            selected = (fields["identifier"].to_owned(), count);
        }
    }
    write_fixture(
        fixture,
        "selected.txt",
        format!("{}\n", selected.0),
        "write",
        calls,
    );
}

fn execute_structured_output(fixture: &Path, calls: &mut Vec<ToolCall>) {
    let input = read_fixture(fixture, "events.json", calls);
    let events: Vec<Value> = serde_json::from_str(&input).unwrap();
    let accepted = events
        .iter()
        .filter(|event| event["result"] == "accepted")
        .count();
    let rejected = events.len() - accepted;
    write_fixture(
        fixture,
        "summary.json",
        json!({"accepted": accepted, "rejected": rejected, "total": events.len()}).to_string(),
        "write",
        calls,
    );
}

fn read_fixture(fixture: &Path, relative: &str, calls: &mut Vec<ToolCall>) -> String {
    let path = PathBuf::from(relative);
    calls.push(ToolCall {
        action: "read",
        path: path.clone(),
    });
    fs::read_to_string(fixture.join(path)).unwrap()
}

fn write_fixture(
    fixture: &Path,
    relative: &str,
    content: String,
    action: &'static str,
    calls: &mut Vec<ToolCall>,
) {
    let path = PathBuf::from(relative);
    fs::write(fixture.join(&path), content).unwrap();
    calls.push(ToolCall { action, path });
}

fn fixture_outputs(fixture: &Path) -> BTreeMap<String, String> {
    snapshot(fixture)
        .into_iter()
        .map(|(path, bytes)| {
            (
                path.to_string_lossy().into_owned(),
                String::from_utf8_lossy(&bytes).into_owned(),
            )
        })
        .collect()
}

fn changed_files(
    before: &BTreeMap<PathBuf, Vec<u8>>,
    after: &BTreeMap<PathBuf, Vec<u8>>,
) -> BTreeSet<PathBuf> {
    before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect()
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    collect_files(root, root, &mut files);
    files
}

fn collect_files(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
    let mut entries = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_type = entry.file_type().unwrap();
        assert!(!file_type.is_symlink());
        if file_type.is_dir() {
            collect_files(root, &entry.path(), files);
        } else {
            assert!(file_type.is_file());
            files.insert(
                entry.path().strip_prefix(root).unwrap().to_owned(),
                fs::read(entry.path()).unwrap(),
            );
        }
    }
}

fn artifact_revision(root: &Path, cases: &[Case]) -> String {
    let root = fs::canonicalize(root).unwrap();
    let mut hasher = RevisionHasher::new();
    hash_revision_path(&mut hasher, &root, &root.join("SKILL.md"), "definition");
    hasher.add(b"required destinations");
    hasher.add(b"skill minimum");
    hasher.add(b"workflow node routing");
    hash_revision_path(&mut hasher, &root, &root.join("evals/cases.jsonl"), "cases");
    for case in cases {
        for support in &case.files {
            hash_revision_path(&mut hasher, &root, &root.join(support), "support");
        }
        hash_revision_path(
            &mut hasher,
            &root,
            &root.join(&case.execution.drive.source),
            "fixture",
        );
    }
    hasher.finish()
}

fn hash_revision_path(hasher: &mut RevisionHasher, root: &Path, path: &Path, role: &str) {
    let canonical = fs::canonicalize(path).unwrap();
    let relative = canonical.strip_prefix(root).unwrap();
    hasher.add(role.as_bytes());
    hash_revision_entry(hasher, root, &canonical, relative);
}

fn hash_revision_entry(hasher: &mut RevisionHasher, root: &Path, path: &Path, relative: &Path) {
    assert!(path.starts_with(root));
    let metadata = fs::symlink_metadata(path).unwrap();
    assert!(!metadata.file_type().is_symlink());
    hasher.add(relative.as_os_str().as_encoded_bytes());
    if metadata.is_file() {
        hasher.add(b"file");
        hasher.add(&fs::read(path).unwrap());
        return;
    }
    assert!(metadata.is_dir());
    hasher.add(b"directory");
    let mut children = fs::read_dir(path)
        .unwrap()
        .map(|entry| fs::canonicalize(entry.unwrap().path()).unwrap())
        .collect::<Vec<_>>();
    children.sort();
    for child in children {
        let name = child.file_name().unwrap();
        hash_revision_entry(hasher, root, &child, &relative.join(name));
    }
}

struct RevisionHasher {
    state: u64,
}

impl RevisionHasher {
    fn new() -> Self {
        Self { state: HASH_OFFSET }
    }

    fn add(&mut self, bytes: &[u8]) {
        for byte in (bytes.len() as u64).to_le_bytes().iter().chain(bytes) {
            self.state ^= u64::from(*byte);
            self.state = self.state.wrapping_mul(HASH_PRIME);
        }
    }

    fn finish(self) -> String {
        format!("fnv1a64:{:016x}", self.state)
    }
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    let mut entries = fs::read_dir(source)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_type = entry.file_type().unwrap();
        assert!(!file_type.is_symlink());
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_directory(&entry.path(), &target);
        } else {
            assert!(file_type.is_file());
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn exam_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/model-calibration")
}

static NEXT_TEMPORARY_ROOT: AtomicU64 = AtomicU64::new(0);

struct TemporaryRoot(PathBuf);

impl TemporaryRoot {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEMPORARY_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skill-eval-model-calibration-{label}-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).unwrap();
        }
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
