use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

#[test]
fn h6_changed_skill_agent_and_workflow_reach_ready_through_the_mandatory_gate() {
    for kind in [Kind::Skill, Kind::Agent, Kind::Workflow] {
        let fixture = Fixture::new(kind);
        let run = fixture.qualify("pass");

        assert_eq!(run.own_eval["status"], "passed");
        assert_eq!(run.own_eval["artifact_revision"], run.revision);
        assert_eq!(candidate_tiers(&run.report), ["t1", "t2"]);
        assert_eq!(
            artifact_report(&run.report)["boundary"]["accepted"]["tier"],
            "t2"
        );

        let before = snapshot(&fixture.artifact);
        let decision = fixture.decide(&run.run_id, kind.assignments("T2"), true);
        let artifact = artifact_report(&decision);
        assert_eq!(artifact["decision"]["decision"], "accepted");
        assert_eq!(
            artifact["decision"]["assignments"]
                .as_array()
                .unwrap()
                .len(),
            kind.destination_count()
        );

        let applied = fixture.apply(&run.run_id, true);
        let artifact = artifact_report(&applied);
        assert_eq!(artifact["publication_gate"]["status"], "ready");
        assert_eq!(
            artifact["publication_gate"]["assignments"],
            artifact["decision"]["assignments"]
        );
        fixture.assert_owned_write(&before);
    }
}

#[test]
fn h6_unresolved_states_preserve_incumbent_bytes_and_tiers() {
    for state in [
        Unresolved::StaleOwnEval,
        Unresolved::Paused,
        Unresolved::IncompleteStaircase,
        Unresolved::AwaitingDecision,
        Unresolved::RejectedDecision,
        Unresolved::MissingWorkflowNode,
        Unresolved::WrongTier,
        Unresolved::WriteFailure,
    ] {
        let kind = if state == Unresolved::MissingWorkflowNode {
            Kind::Workflow
        } else {
            Kind::Skill
        };
        let fixture = Fixture::new(kind);
        let run = fixture.qualify(match state {
            Unresolved::Paused => "pause",
            Unresolved::IncompleteStaircase => "all-fail",
            _ => "pass",
        });
        let before = snapshot(&fixture.artifact);

        match state {
            Unresolved::StaleOwnEval => fixture.stale_own_eval(&run.run_id),
            Unresolved::RejectedDecision => {
                fixture.reject(&run.run_id);
            }
            Unresolved::MissingWorkflowNode => {
                fixture.decide(&run.run_id, &["orchestrator=T2"], false);
            }
            Unresolved::WrongTier => {
                fixture.decide(&run.run_id, kind.assignments("T3"), false);
            }
            Unresolved::WriteFailure => {
                fixture.decide(&run.run_id, kind.assignments("T2"), true);
                fixture.block_skill_staging();
            }
            Unresolved::Paused | Unresolved::IncompleteStaircase | Unresolved::AwaitingDecision => {
            }
        }

        fixture.apply(&run.run_id, false);
        assert_snapshot(&fixture.artifact, &before);
        fixture.assert_incumbent_tiers();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Skill,
    Agent,
    Workflow,
}

impl Kind {
    fn directory(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Agent => "agent",
            Self::Workflow => "workflow",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Skill => "h6-skill",
            Self::Agent => "h6-agent",
            Self::Workflow => "h6-workflow",
        }
    }

    fn assignments(self, tier: &'static str) -> &'static [&'static str] {
        match (self, tier) {
            (Self::Skill, "T2") => &["minimum=T2", "target=T2"],
            (Self::Skill, "T3") => &["minimum=T3", "target=T3"],
            (Self::Agent, "T2") => &["agent=T2"],
            (Self::Agent, "T3") => &["agent=T3"],
            (Self::Workflow, "T2") => &[
                "orchestrator=T2",
                "workflow_node:Plan=T2",
                "workflow_node:Review=T2",
            ],
            (Self::Workflow, "T3") => &[
                "orchestrator=T3",
                "workflow_node:Plan=T3",
                "workflow_node:Review=T3",
            ],
            _ => unreachable!(),
        }
    }

    fn destination_count(self) -> usize {
        self.assignments("T2").len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Unresolved {
    StaleOwnEval,
    Paused,
    IncompleteStaircase,
    AwaitingDecision,
    RejectedDecision,
    MissingWorkflowNode,
    WrongTier,
    WriteFailure,
}

struct Qualification {
    run_id: String,
    revision: String,
    own_eval: Value,
    report: Value,
}

struct Fixture {
    root: PathBuf,
    artifact: PathBuf,
    runs: PathBuf,
    bin: PathBuf,
    kind: Kind,
}

impl Fixture {
    fn new(kind: Kind) -> Self {
        let root = std::env::temp_dir().join(format!(
            "skill-eval-authoring-gate-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let source = fixture_root();
        let artifact = root.join("artifact");
        copy_tree(&source.join(kind.directory()), &artifact);
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_fake-pi-authoring-gate"), bin.join("pi")).unwrap();
        make_executable(&bin.join("pi"));
        Self {
            runs: root.join("runs"),
            root,
            artifact,
            bin,
            kind,
        }
    }

    fn qualify(&self, mode: &str) -> Qualification {
        let discovery_runs = self
            .root
            .join(format!("discovery-{}", self.kind.directory()));
        let discovery = self.command(
            vec![
                "qualify".to_owned(),
                "--artifact".to_owned(),
                path_text(&self.artifact),
                "--dry-run".to_owned(),
                "--runs-root".to_owned(),
                path_text(&discovery_runs),
                "--format".to_owned(),
                "jsonl".to_owned(),
            ],
            "pass",
        );
        assert_success(&discovery);
        let discovery = last_json(&discovery.stdout);
        let revision = discovery["discoveries"][0]["revision"]
            .as_str()
            .unwrap()
            .to_owned();
        let own_eval_path = self.artifact.join("evals/result.json");
        let own_eval = json!({
            "artifact_revision": revision,
            "case": format!("own-{}", self.kind.directory()),
            "status": "passed"
        });
        fs::write(
            &own_eval_path,
            serde_json::to_vec_pretty(&own_eval).unwrap(),
        )
        .unwrap();

        let discovery_run_id = discovery["run_id"].as_str().unwrap();
        let discovery_log = discovery_runs.join(discovery_run_id).join("events.jsonl");
        let mut started = serde_json::from_str::<Value>(
            fs::read_to_string(discovery_log)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        let run_id = format!("h6-{}", self.kind.directory());
        let change = json!({
            "artifact": self.kind.name(),
            "kind": self.kind.directory(),
            "incumbent_revision": format!("incumbent-{}", self.kind.directory()),
            "candidate_revision": revision.clone(),
            "own_eval": {
                "artifact_revision": revision.clone(),
                "path": own_eval_path
            }
        });
        started["configuration"]["run_id"] = Value::String(run_id.clone());
        started["configuration"]["mode"] = Value::String("execute".to_owned());
        started["configuration"]["change"] = change;
        started["configuration"]["policy"] = json!({
            "candidate_tiers": ["t1", "t2", "t3"],
            "reference_tier": "t4",
            "judge_tier": "t5",
            "repeats_per_case": 1,
            "minimum_score": 8,
            "noninferiority_margin": 0.0,
            "confidence_level": 0.95
        });

        let reference = evidence("reference", "t4", "accepted", 0.9, &revision);
        let failing_t1 = evidence("candidate", "t1", "failed", 0.6, &revision);
        let failing_t2 = evidence("candidate", "t2", "failed", 0.6, &revision);
        let failing_t3 = evidence("candidate", "t3", "failed", 0.6, &revision);
        let accepted_t2 = evidence("candidate", "t2", "accepted", 0.9, &revision);
        let mut events = vec![started];
        match mode {
            "pass" => {
                events.push(tier_event(self.kind.name(), reference));
                events.push(tier_event(self.kind.name(), failing_t1.clone()));
                events.push(tier_event(self.kind.name(), accepted_t2.clone()));
                events.push(json!({
                    "event": "boundary_found",
                    "at": "h6-time",
                    "artifact": self.kind.name(),
                    "boundary": {"failing": failing_t1, "accepted": accepted_t2}
                }));
            }
            "pause" => events.push(json!({
                "event": "run_paused",
                "at": "h6-time",
                "reason": {"kind": "infrastructure", "message": "synthetic pause"}
            })),
            "all-fail" => {
                events.push(tier_event(self.kind.name(), reference));
                events.push(tier_event(self.kind.name(), failing_t1));
                events.push(tier_event(self.kind.name(), failing_t2));
                events.push(tier_event(self.kind.name(), failing_t3));
                events.push(json!({
                    "event": "review_required",
                    "at": "h6-time",
                    "artifact": self.kind.name(),
                    "reason": "synthetic staircase has no supported boundary"
                }));
            }
            _ => unreachable!(),
        }
        let run_directory = self.runs.join(&run_id);
        fs::create_dir_all(&run_directory).unwrap();
        let mut event_log = events
            .iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        event_log.push('\n');
        fs::write(run_directory.join("events.jsonl"), event_log).unwrap();

        let output = self.command(
            vec![
                "report".to_owned(),
                "--run".to_owned(),
                run_id.clone(),
                "--runs-root".to_owned(),
                path_text(&self.runs),
                "--format".to_owned(),
                "jsonl".to_owned(),
            ],
            "pass",
        );
        assert_success(&output);
        Qualification {
            run_id,
            revision,
            own_eval,
            report: last_json(&output.stdout),
        }
    }

    fn decide(&self, run_id: &str, assignments: &[&str], is_success: bool) -> Value {
        let mut arguments = vec![
            "decide".to_owned(),
            "--run".to_owned(),
            run_id.to_owned(),
            "--artifact".to_owned(),
            self.kind.name().to_owned(),
            "--accept".to_owned(),
        ];
        for assignment in assignments {
            arguments.push("--assign".to_owned());
            arguments.push((*assignment).to_owned());
        }
        arguments.extend([
            "--runs-root".to_owned(),
            path_text(&self.runs),
            "--format".to_owned(),
            "jsonl".to_owned(),
        ]);
        let output = self.command(arguments, "pass");
        assert_eq!(output.status.success(), is_success, "{}", stderr(&output));
        if is_success {
            last_json(&output.stdout)
        } else {
            Value::Null
        }
    }

    fn reject(&self, run_id: &str) {
        let output = self.command(
            vec![
                "decide".to_owned(),
                "--run".to_owned(),
                run_id.to_owned(),
                "--artifact".to_owned(),
                self.kind.name().to_owned(),
                "--reject".to_owned(),
                "--reason".to_owned(),
                "owner keeps incumbent".to_owned(),
                "--runs-root".to_owned(),
                path_text(&self.runs),
                "--format".to_owned(),
                "jsonl".to_owned(),
            ],
            "pass",
        );
        assert_success(&output);
    }

    fn apply(&self, run_id: &str, is_success: bool) -> Value {
        let output = self.command(
            vec![
                "apply".to_owned(),
                "--run".to_owned(),
                run_id.to_owned(),
                "--artifact".to_owned(),
                self.kind.name().to_owned(),
                "--runs-root".to_owned(),
                path_text(&self.runs),
                "--format".to_owned(),
                "jsonl".to_owned(),
            ],
            "pass",
        );
        assert_eq!(output.status.success(), is_success, "{}", stderr(&output));
        if is_success {
            last_json(&output.stdout)
        } else {
            Value::Null
        }
    }

    fn stale_own_eval(&self, run_id: &str) {
        let log = self.runs.join(run_id).join("events.jsonl");
        let text = fs::read_to_string(&log).unwrap();
        let mut lines = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        lines[0]["configuration"]["change"]["own_eval"]["artifact_revision"] =
            Value::String("stale-own-eval".to_owned());
        let mut output = lines
            .iter()
            .map(|line| serde_json::to_string(line).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        output.push('\n');
        fs::write(log, output).unwrap();
    }

    fn block_skill_staging(&self) {
        for suffix in 0..1000_u16 {
            fs::write(
                self.artifact.join(format!(".SKILL.md.tier-write-{suffix}")),
                b"occupied",
            )
            .unwrap();
        }
    }

    fn assert_owned_write(&self, before: &BTreeMap<PathBuf, Vec<u8>>) {
        let after = snapshot(&self.artifact);
        let changed = before
            .iter()
            .filter_map(|(path, bytes)| (after.get(path) != Some(bytes)).then_some(path.clone()))
            .collect::<Vec<_>>();
        let expected = match self.kind {
            Kind::Skill => vec![PathBuf::from("SKILL.md")],
            Kind::Agent => vec![PathBuf::from("config/model-tiers.json")],
            Kind::Workflow => vec![PathBuf::from("SKILL.md"), PathBuf::from("h6.workflow.js")],
        };
        assert_eq!(changed, expected);
        match self.kind {
            Kind::Skill => {
                let text = fs::read_to_string(self.artifact.join("SKILL.md")).unwrap();
                assert!(text.contains("minimum-tier: T2"));
                assert!(text.contains("target-tier: T2"));
            }
            Kind::Agent => {
                let text =
                    fs::read_to_string(self.artifact.join("config/model-tiers.json")).unwrap();
                assert!(text.contains("\"h6-agent\": \"T2\""));
                assert!(text.contains("\"other-agent\": \"T1\""));
            }
            Kind::Workflow => {
                let definition = fs::read_to_string(self.artifact.join("SKILL.md")).unwrap();
                let workflow = fs::read_to_string(self.artifact.join("h6.workflow.js")).unwrap();
                assert!(definition.contains("minimum-tier: T2"));
                assert!(workflow.contains("title: 'Plan', tier: 'T2', keep: 'plan'"));
                assert!(workflow.contains("title: 'Review', tier: 'T2', keep: 'review'"));
            }
        }
    }

    fn assert_incumbent_tiers(&self) {
        match self.kind {
            Kind::Skill => {
                let text = fs::read_to_string(self.artifact.join("SKILL.md")).unwrap();
                assert!(text.contains("minimum-tier: T4"));
                assert!(text.contains("target-tier: T3"));
            }
            Kind::Agent => {
                let text =
                    fs::read_to_string(self.artifact.join("config/model-tiers.json")).unwrap();
                assert!(text.contains("\"h6-agent\": \"T3\""));
            }
            Kind::Workflow => {
                let definition = fs::read_to_string(self.artifact.join("SKILL.md")).unwrap();
                let workflow = fs::read_to_string(self.artifact.join("h6.workflow.js")).unwrap();
                assert!(definition.contains("minimum-tier: T4"));
                assert!(workflow.contains("title: 'Plan', model: 'sonnet'"));
                assert!(workflow.contains("title: 'Review', tier: 'T4'"));
            }
        }
    }

    fn command<I, S>(&self, arguments: I, mode: &str) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_skill-eval"))
            .args(arguments)
            .current_dir(repository_root())
            .env("PATH", path)
            .env("H6_MODE", mode)
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/authoring-gate")
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir(destination).unwrap();
    let mut entries = fs::read_dir(source)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_: &Path) {}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), files);
            } else {
                files.insert(
                    entry.path().strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn assert_snapshot(root: &Path, before: &BTreeMap<PathBuf, Vec<u8>>) {
    for (path, bytes) in before {
        assert_eq!(
            &fs::read(root.join(path)).unwrap(),
            bytes,
            "{} changed",
            path.display()
        );
    }
}

fn evidence(role: &str, tier: &str, status: &str, score: f64, revision: &str) -> Value {
    json!({
        "role": role,
        "tier": tier,
        "model": {
            "tier": tier,
            "provider": "synthetic",
            "model": format!("h6-{tier}"),
            "thinking": "fixed"
        },
        "harnesses": [{
            "runner_version": "phase-04-h6",
            "pi_version": "synthetic-pi-h6",
            "artifact_revision": revision,
            "tool_policy_digest": "synthetic-policy"
        }],
        "status": status,
        "completed_trials": 1,
        "expected_trials": 1,
        "passed_trials": usize::from(status == "accepted"),
        "score": {"lower": score, "estimate": score, "upper": score},
        "candidate_usage": usage(),
        "judge_usage": usage(),
        "total_usage": usage()
    })
}

fn tier_event(artifact: &str, evidence: Value) -> Value {
    json!({
        "event": "tier_evaluated",
        "at": "h6-time",
        "artifact": artifact,
        "evidence": evidence
    })
}

fn usage() -> Value {
    json!({
        "input_tokens": 1,
        "output_tokens": 1,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "turns": 1,
        "tool_calls": 0,
        "elapsed_milliseconds": 1,
        "cost_millionths_of_dollar": 0
    })
}

fn candidate_tiers(report: &Value) -> Vec<&str> {
    artifact_report(report)["tiers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|evidence| evidence["role"] == "candidate")
        .map(|evidence| evidence["tier"].as_str().unwrap())
        .collect()
}

fn artifact_report(report: &Value) -> &Value {
    &report["artifacts"][0]
}

fn last_json(bytes: &[u8]) -> Value {
    String::from_utf8_lossy(bytes)
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line).ok())
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(output.status.success(), "{}", stderr(output));
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
