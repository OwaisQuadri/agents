#![expect(
    dead_code,
    reason = "the integration test imports private production modules for the frontier lifecycle"
)]
#![expect(
    clippy::large_enum_variant,
    reason = "the integration test imports the frozen production model declarations"
)]

#[path = "../src/audit.rs"]
mod audit;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/pi_runner.rs"]
mod pi_runner;
#[path = "../src/ports.rs"]
mod ports;
#[path = "../src/publication.rs"]
mod publication;
#[path = "../src/service.rs"]
mod service;
#[path = "../src/statistics.rs"]
mod statistics;
#[path = "../src/store.rs"]
mod store;
#[path = "../src/t1_screen_campaign_store.rs"]
mod t1_screen_campaign_store;
#[path = "../src/t1_screen_store.rs"]
mod t1_screen_store;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use model::{
    ArtifactDefinition, ArtifactKind, ArtifactName, CandidateArtifact, CaseDefinition, CaseDrive,
    CaseId, CheckResult, ExecutionDefinition, FrontierApplyReport, FrontierBaselineLedger,
    FrontierCaseGroup, FrontierCaseReference, FrontierConfidenceMethod, FrontierEntrant,
    FrontierInspection, FrontierPlan, FrontierPolicy, FrontierRunId, FrontierRunState,
    FrontierSuite, FrontierSuiteIdentity, FrontierTierSuite, HarnessIdentity, JudgeInput,
    JudgeResult, ModelIdentity, PromptJudgeRequest, PromptJudgeResult, RunEvent, RunId,
    SkillEvalError, T1ScreenSnapshotIdentity, Tier, TierAssignment, TierDestination, Timestamp,
    TrialKey, TrialRecord, TrialSelector,
};
use pi_runner::{PiCandidateRunner, Process, ProcessOutput, ProcessRequest};
use ports::{
    ArtifactSource, CandidateRunner, Clock, FrontierProgressSink, FrontierRuntime, HarnessResolver,
    Judge, ModelResolver, QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
};
use serde_json::{Value, json};
use service::{resume_frontier, start_frontier};

const OPT_IN: &str = "SKILL_EVAL_H4_REAL_PI";
const CREDENTIAL: &str = "OPENROUTER_API_KEY";
const CANDIDATE_PROVIDER: &str = "openrouter";
const CANDIDATE_MODEL: &str = "google/gemini-3.7-flash";
const JUDGE_PROVIDER: &str = "openrouter";
const JUDGE_MODEL: &str = "openai/gpt-5.6-sol-pro";

#[test]
fn frontier_start_and_resume_reach_only_exact_first_party_repository_fake_pi() {
    for (provider, model) in [
        ("anthropic", "frontier-anthropic"),
        ("openai-codex", "frontier-codex"),
    ] {
        let fixture = FrontierFixture::new(provider, model);
        let mut runtime = fixture.runtime(FakePiOutcome::Quota);
        let mut progress = SavedProgress::default();

        let paused =
            start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress).unwrap();
        assert!(matches!(paused.status, model::FrontierRunStatus::Paused));
        assert!(paused.cells.is_empty());
        assert_eq!(paused.spent_millionths_of_dollar, 0);

        let resumed =
            resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress).unwrap();
        assert!(matches!(resumed.status, model::FrontierRunStatus::Paused));
        assert!(resumed.cells.is_empty());
        assert_eq!(runtime.process_requests.borrow().len(), 60);
        assert!(
            runtime
                .process_requests
                .borrow()
                .iter()
                .all(|request| request.timeout.is_none())
        );
        assert_eq!(runtime.trials.borrow().len(), 0);
        assert_eq!(runtime.candidate_performance.get(), 0);
        assert_guards_precede_process(&runtime.events.borrow());

        let log = fs::read_to_string(&fixture.log).unwrap();
        let candidate = format!("candidate:{provider}/{model}/low");
        let candidates = log
            .lines()
            .filter(|line| line.starts_with("candidate:"))
            .collect::<Vec<_>>();
        assert_eq!(candidates.len(), 60);
        assert!(candidates.iter().all(|line| *line == candidate));
        assert_guarded_environment(&log, &fixture.environment, 60);
        for request in runtime.process_requests.borrow().iter() {
            let working_directory = format!("cwd:{}", request.working_directory.display());
            assert!(log.lines().any(|line| line == working_directory));
        }
        assert!(
            log.lines()
                .filter(|line| line.starts_with("args:"))
                .all(|line| line.contains(&format!("--model {provider}/{model}"))
                    && line.contains("--thinking low")
                    && !line.contains("timeout"))
        );
    }
}

#[test]
fn frontier_openrouter_fails_before_repository_fake_pi_launch() {
    let fixture = FrontierFixture::new("openrouter", "frontier-proxy");
    let mut runtime = fixture.runtime(FakePiOutcome::Quota);
    let mut progress = SavedProgress::default();

    let error =
        start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress).unwrap_err();

    assert!(matches!(error, SkillEvalError::InvalidConfiguration(_)));
    assert!(runtime.process_requests.borrow().is_empty());
    assert!(!fixture.log.exists());
}

#[test]
fn frontier_infrastructure_retries_are_service_managed_and_bounded() {
    let fixture = FrontierFixture::new("openai-codex", "frontier-codex");
    let mut runtime = fixture.runtime(FakePiOutcome::Infrastructure);
    let mut progress = SavedProgress::default();

    let first =
        start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress).unwrap();
    assert_infrastructure_pause(&first, 30, 1);
    assert_eq!(runtime.state.as_ref(), Some(&first));
    assert_eq!(progress.states.last(), Some(&first));
    assert_eq!(runtime.process_requests.borrow().len(), 30);
    assert_eq!(runtime.trials.borrow().len(), 0);
    assert_eq!(runtime.candidate_performance.get(), 0);

    let second = resume_frontier(&first.configuration.run_id, &mut runtime, &mut progress).unwrap();
    assert_infrastructure_pause(&second, 60, 2);
    assert_eq!(runtime.state.as_ref(), Some(&second));
    assert_eq!(progress.states.last(), Some(&second));
    assert_eq!(runtime.process_requests.borrow().len(), 60);
    assert_eq!(runtime.trials.borrow().len(), 0);
    assert_eq!(runtime.candidate_performance.get(), 0);

    let saved_progress_count = progress.states.len();
    let stopped =
        resume_frontier(&second.configuration.run_id, &mut runtime, &mut progress).unwrap();
    assert_eq!(stopped, second);
    assert_infrastructure_pause(&stopped, 60, 2);
    assert_eq!(progress.states.len(), saved_progress_count);

    assert_eq!(runtime.process_requests.borrow().len(), 60);
    assert_eq!(runtime.trials.borrow().len(), 0);
    assert_eq!(runtime.candidate_performance.get(), 0);
    assert!(stopped.cells.is_empty());
    assert_eq!(stopped.spent_millionths_of_dollar, 0);
    assert_eq!(
        stopped
            .configuration
            .plan
            .policy
            .maximum_infrastructure_attempts,
        2
    );
    assert_guards_precede_process(&runtime.events.borrow());
}

#[derive(Clone, Copy)]
enum FakePiOutcome {
    Quota,
    Infrastructure,
}

impl FakePiOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Quota => "quota",
            Self::Infrastructure => "infrastructure",
        }
    }
}

struct FrontierFixture {
    _root: TemporaryRoot,
    skill: PathBuf,
    runs: PathBuf,
    fake_pi: PathBuf,
    log: PathBuf,
    environment: BTreeMap<&'static str, PathBuf>,
    provider: String,
    model: String,
}

impl FrontierFixture {
    fn new(provider: &str, model: &str) -> Self {
        let root = TemporaryRoot::new();
        let repository = root.path().join("repository");
        let runs = root.path().join("runs");
        let bin = root.path().join("bin");
        let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let skill = repository.join("skills/frontier-pi");
        for directory in [&repository, &runs, &bin] {
            fs::create_dir_all(directory).unwrap();
        }
        copy_directory(&fixture_root.join("integration/skill"), &skill);
        let fake_pi = bin.join("pi");
        fs::copy(fixture_root.join("frontier-pi/bin/pi"), &fake_pi).unwrap();
        fs::set_permissions(&fake_pi, fs::Permissions::from_mode(0o700)).unwrap();
        let environment = BTreeMap::from([
            ("HOME", root.path().join("home")),
            ("PI_CODING_AGENT_DIR", root.path().join("pi")),
            ("PI_CODING_AGENT_SESSION_DIR", root.path().join("sessions")),
            ("XDG_CONFIG_HOME", root.path().join("config")),
            ("XDG_CACHE_HOME", root.path().join("cache")),
            ("XDG_DATA_HOME", root.path().join("data")),
            ("TMPDIR", root.path().join("tmp")),
        ]);
        for directory in environment.values() {
            fs::create_dir_all(directory).unwrap();
        }
        Self {
            log: root.path().join("pi.log"),
            _root: root,
            skill,
            runs,
            fake_pi,
            environment,
            provider: provider.to_owned(),
            model: model.to_owned(),
        }
    }

    fn runtime(&self, outcome: FakePiOutcome) -> LifecycleRuntime {
        let process_requests = Rc::new(RefCell::new(Vec::new()));
        let events = Rc::new(RefCell::new(Vec::new()));
        let process = RepositoryFakeProcess {
            executable: self.fake_pi.clone(),
            log: self.log.clone(),
            environment: self.environment.clone(),
            outcome,
            requests: process_requests.clone(),
            events: events.clone(),
        };
        let artifact = frontier_artifact(&self.skill);
        LifecycleRuntime {
            plan: frontier_plan(&self.provider, &self.model),
            suite: frontier_suite(&artifact),
            artifact,
            runner: PiCandidateRunner::with_process(self.runs.clone(), process),
            state: None,
            trials: Rc::new(RefCell::new(Vec::new())),
            process_requests,
            environment: self.environment.clone(),
            events,
            candidate_performance: Cell::new(0),
        }
    }
}

struct RepositoryFakeProcess {
    executable: PathBuf,
    log: PathBuf,
    environment: BTreeMap<&'static str, PathBuf>,
    outcome: FakePiOutcome,
    requests: Rc<RefCell<Vec<ProcessRequest>>>,
    events: Rc<RefCell<Vec<&'static str>>>,
}

impl Process for RepositoryFakeProcess {
    fn run(&mut self, request: &ProcessRequest) -> io::Result<ProcessOutput> {
        self.events.borrow_mut().push("process");
        self.requests.borrow_mut().push(request.clone());
        let mut command = Command::new(&self.executable);
        command
            .args(&request.arguments)
            .current_dir(&request.working_directory)
            .env_clear()
            .env("FAKE_PI_LOG", &self.log)
            .env("FAKE_PI_OUTCOME", self.outcome.as_str())
            .env("PI_SKIP_VERSION_CHECK", "1")
            .env("PI_TELEMETRY", "0");
        for (name, value) in &self.environment {
            command.env(name, value);
        }
        let output = command.output()?;
        Ok(ProcessOutput {
            exit_code: output.status.code(),
            standard_output: output.stdout,
            standard_error: output.stderr,
            is_timed_out: false,
        })
    }
}

struct LifecycleRuntime {
    plan: FrontierPlan,
    suite: FrontierSuite,
    artifact: ArtifactDefinition,
    runner: PiCandidateRunner<RepositoryFakeProcess>,
    state: Option<FrontierRunState>,
    trials: Rc<RefCell<Vec<TrialRecord>>>,
    process_requests: Rc<RefCell<Vec<ProcessRequest>>>,
    environment: BTreeMap<&'static str, PathBuf>,
    events: Rc<RefCell<Vec<&'static str>>>,
    candidate_performance: Cell<u32>,
}

impl QualificationRuntime for LifecycleRuntime {}

impl FrontierRuntime for LifecycleRuntime {
    fn load_frontier_plan(
        &self,
        _path: &Path,
    ) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError> {
        self.events.borrow_mut().extend(["suite", "plan"]);
        Ok((self.plan.clone(), self.suite.clone()))
    }

    fn next_frontier_run_id(&mut self) -> Result<FrontierRunId, SkillEvalError> {
        Ok(FrontierRunId("frontier-repository-fake".to_owned()))
    }

    fn create_frontier(&mut self, state: &FrontierRunState) -> Result<(), SkillEvalError> {
        if self.state.replace(state.clone()).is_some() {
            return Err(SkillEvalError::InvalidConfiguration(
                "frontier already exists".to_owned(),
            ));
        }
        Ok(())
    }

    fn load_frontier(&self, run_id: &FrontierRunId) -> Result<FrontierRunState, SkillEvalError> {
        self.state
            .as_ref()
            .filter(|state| state.configuration.run_id == *run_id)
            .cloned()
            .ok_or_else(|| SkillEvalError::NotFound("frontier".to_owned()))
    }

    fn save_frontier(&mut self, state: &FrontierRunState) -> Result<(), SkillEvalError> {
        self.state = Some(state.clone());
        Ok(())
    }

    fn save_frontier_trial(
        &mut self,
        _run_id: &FrontierRunId,
        trial: &TrialRecord,
    ) -> Result<(), SkillEvalError> {
        self.trials.borrow_mut().push(trial.clone());
        Ok(())
    }

    fn load_frontier_trials(
        &self,
        _run_id: &FrontierRunId,
    ) -> Result<Vec<TrialRecord>, SkillEvalError> {
        Ok(self.trials.borrow().clone())
    }

    fn inspect_frontier(
        &self,
        selector: &model::FrontierTrialSelector,
    ) -> Result<FrontierInspection, SkillEvalError> {
        self.trials
            .borrow()
            .iter()
            .find(|trial| {
                trial.model.provider == selector.provider
                    && trial.model.model == selector.model
                    && trial.model.tier == selector.tier
                    && trial.model.thinking == selector.thinking
                    && trial.key.artifact == selector.artifact
                    && trial.key.case == selector.case
                    && trial.key.attempt == selector.attempt
            })
            .cloned()
            .map(|trial| FrontierInspection::Trial { trial })
            .ok_or_else(|| SkillEvalError::NotFound("frontier trial".to_owned()))
    }

    fn load_frontier_baselines(
        &self,
        _path: &Path,
    ) -> Result<FrontierBaselineLedger, SkillEvalError> {
        panic!("frontier execution loaded baselines")
    }

    fn accept_frontier_baseline(
        &mut self,
        _state: &FrontierRunState,
        _path: &Path,
        _ledger: &FrontierBaselineLedger,
    ) -> Result<(), SkillEvalError> {
        panic!("frontier execution accepted a baseline")
    }

    fn apply_frontier_routes(
        &mut self,
        _state: &FrontierRunState,
    ) -> Result<FrontierApplyReport, SkillEvalError> {
        panic!("frontier execution applied routes")
    }
}

impl ArtifactSource for LifecycleRuntime {
    fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
        self.events.borrow_mut().push("source");
        if root != self.artifact.root {
            return Err(SkillEvalError::NotFound(root.display().to_string()));
        }
        Ok(self.artifact.clone())
    }
}

impl ModelResolver for LifecycleRuntime {
    fn candidates(&self, _tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
        Ok(Vec::new())
    }

    fn qualification_routes(&self, _tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
        Ok(Vec::new())
    }

    fn exact_candidate(&self, requested: &ModelIdentity) -> Result<ModelIdentity, SkillEvalError> {
        self.events.borrow_mut().push("catalog");
        assert_eq!(
            self.environment.keys().copied().collect::<Vec<_>>(),
            [
                "HOME",
                "PI_CODING_AGENT_DIR",
                "PI_CODING_AGENT_SESSION_DIR",
                "TMPDIR",
                "XDG_CACHE_HOME",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
            ]
        );
        assert!(self.environment.values().all(|path| path.is_dir()));
        self.events.borrow_mut().push("environment");
        Ok(requested.clone())
    }

    fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
        Ok(Tier::T5)
    }

    fn judge(
        &self,
        _judge_tier: Tier,
        _candidate: Option<&ModelIdentity>,
    ) -> Result<ModelIdentity, SkillEvalError> {
        Ok(frontier_judge())
    }
}

impl HarnessResolver for LifecycleRuntime {
    fn identity(
        &self,
        artifact: &ArtifactDefinition,
        _execution: &ExecutionDefinition,
    ) -> Result<HarnessIdentity, SkillEvalError> {
        self.events.borrow_mut().push("harness");
        Ok(HarnessIdentity {
            runner_version: "repository-fake-runner".to_owned(),
            pi_version: "synthetic-frontier-pi-1".to_owned(),
            artifact_revision: artifact.revision.clone(),
            tool_policy_digest: "repository-fake-tools".to_owned(),
        })
    }
}

impl CandidateRunner for LifecycleRuntime {
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
        assert!(run_id.0.starts_with("frontier-"));
        assert!(matches!(
            model.provider.as_str(),
            "anthropic" | "openai-codex"
        ));
        assert_eq!(candidate_timeout_seconds, None);
        let state = self.state.as_ref().unwrap();
        assert_eq!(state.spent_millionths_of_dollar, 0);
        assert_eq!(
            state
                .configuration
                .plan
                .policy
                .maximum_infrastructure_attempts,
            2
        );
        self.events.borrow_mut().extend(["spend", "authority"]);
        let result = self.runner.execute(
            run_id,
            key,
            artifact,
            case,
            model,
            harness,
            candidate_timeout_seconds,
        );
        if result.is_ok() {
            self.candidate_performance
                .set(self.candidate_performance.get() + 1);
        }
        result
    }
}

impl Clock for LifecycleRuntime {
    fn now(&self) -> Timestamp {
        Timestamp("2030-01-01T00:00:00+0000".to_owned())
    }
}

impl RunIdSource for LifecycleRuntime {
    fn next(&mut self) -> Result<RunId, SkillEvalError> {
        Ok(RunId("unused".to_owned()))
    }
}

impl Verifier for LifecycleRuntime {
    fn verify(
        &mut self,
        _case: &CaseDefinition,
        _candidate: &CandidateArtifact,
    ) -> Result<Vec<CheckResult>, SkillEvalError> {
        panic!("quota candidate reached verification")
    }
}

impl Judge for LifecycleRuntime {
    fn grade(
        &mut self,
        _model: &ModelIdentity,
        _input: &JudgeInput,
    ) -> Result<JudgeResult, SkillEvalError> {
        panic!("quota candidate reached judging")
    }

    fn grade_prompt(
        &mut self,
        _model: &ModelIdentity,
        _request: &PromptJudgeRequest,
    ) -> Result<PromptJudgeResult, SkillEvalError> {
        panic!("frontier execution graded a prompt")
    }
}

impl RunStore for LifecycleRuntime {
    fn append(&mut self, _run_id: &RunId, _event: &RunEvent) -> Result<(), SkillEvalError> {
        panic!("frontier execution appended an ordinary event")
    }

    fn replay(
        &self,
        _run_id: &RunId,
        _visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
    ) -> Result<(), SkillEvalError> {
        panic!("frontier execution replayed an ordinary run")
    }

    fn find_trial(&self, _selector: &TrialSelector) -> Result<TrialRecord, SkillEvalError> {
        panic!("frontier execution searched an ordinary run")
    }
}

impl TierWriter for LifecycleRuntime {
    fn write(
        &mut self,
        _artifact: &ArtifactDefinition,
        _assignments: &[TierAssignment],
    ) -> Result<(), SkillEvalError> {
        panic!("frontier execution wrote tiers")
    }
}

#[derive(Default)]
struct SavedProgress {
    states: Vec<FrontierRunState>,
}

impl FrontierProgressSink for SavedProgress {
    fn emit_frontier(&mut self, state: &FrontierRunState) -> Result<(), SkillEvalError> {
        self.states.push(state.clone());
        Ok(())
    }
}

fn frontier_artifact(root: &Path) -> ArtifactDefinition {
    ArtifactDefinition {
        name: ArtifactName("frontier-pi".to_owned()),
        kind: ArtifactKind::Skill,
        root: root.to_path_buf(),
        revision: "repository-fake-revision".to_owned(),
        required_destinations: vec![TierDestination::SkillMinimum],
        current_tiers: Vec::new(),
        cases: [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5]
            .into_iter()
            .flat_map(|tier| (0..30).map(move |index| (tier, index)))
            .map(|(tier, index)| CaseDefinition {
                id: CaseId(format!("{tier:?}-frontier-case-{index:02}")),
                input: "Answer the repository fake prompt.".to_owned(),
                expect: "not reached because quota pauses".to_owned(),
                source: "repository fake".to_owned(),
                is_holdout: true,
                support_files: Vec::new(),
                execution: ExecutionDefinition {
                    drive: CaseDrive::Response,
                    allowed_tools: Vec::new(),
                    timeout_seconds: 30,
                },
            })
            .collect(),
    }
}

fn frontier_suite(artifact: &ArtifactDefinition) -> FrontierSuite {
    FrontierSuite {
        version: 1,
        tiers: [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5]
            .into_iter()
            .enumerate()
            .map(|(tier_index, tier)| {
                (
                    tier,
                    FrontierTierSuite {
                        group_weights_basis_points: BTreeMap::from([
                            (FrontierCaseGroup::Normal, 2_500),
                            (FrontierCaseGroup::Edge, 2_500),
                            (FrontierCaseGroup::Adversarial, 2_500),
                            (FrontierCaseGroup::Critical, 2_500),
                        ]),
                        cases: artifact
                            .cases
                            .iter()
                            .skip(tier_index * 30)
                            .take(30)
                            .enumerate()
                            .map(|(index, case)| FrontierCaseReference {
                                artifact_path: artifact.root.clone(),
                                artifact_revision: artifact.revision.clone(),
                                case: case.id.clone(),
                                group: match index % 4 {
                                    0 => FrontierCaseGroup::Normal,
                                    1 => FrontierCaseGroup::Edge,
                                    2 => FrontierCaseGroup::Adversarial,
                                    _ => FrontierCaseGroup::Critical,
                                },
                                is_confirmation: true,
                            })
                            .collect(),
                    },
                )
            })
            .collect(),
    }
}

fn frontier_plan(provider: &str, model: &str) -> FrontierPlan {
    FrontierPlan {
        version: 1,
        suite: FrontierSuiteIdentity {
            path: PathBuf::from("frontier-suite.json"),
            sha256: "a".repeat(64),
            version: 1,
        },
        capabilities: T1ScreenSnapshotIdentity {
            path: PathBuf::from("frontier-capabilities.json"),
            sha256: "b".repeat(64),
            version: 1,
            observed_at_unix_seconds: 1_893_456_000,
            pi_version: "synthetic-frontier-pi-1".to_owned(),
        },
        entrants: vec![FrontierEntrant {
            provider: provider.to_owned(),
            model: model.to_owned(),
            entry_tier: Tier::T5,
            thinking_levels: vec!["low".to_owned()],
            catalog_observed_at: Timestamp("2030-01-01T00:00:00+0000".to_owned()),
        }],
        judge: frontier_judge(),
        policy: FrontierPolicy {
            screening_trials_per_case: 1,
            confirmation_trials_per_case: 3,
            maximum_trials_per_case: 5,
            minimum_trial_score: 7,
            minimum_weighted_pass_basis_points: 8_500,
            minimum_lower_bound_basis_points: 8_000,
            confidence_level_basis_points: 9_500,
            confidence_method: FrontierConfidenceMethod::StratifiedBootstrap,
            confidence_resamples: 10,
            maximum_infrastructure_attempts: 2,
            maximum_catalog_age_seconds: 3_600,
            active_pool_size: 5,
            maximum_trial_cost_millionths_of_dollar: 10,
            spending_limit_millionths_of_dollar: 15_000,
            is_provider_limit_enforced: true,
            is_first_party_only: true,
        },
    }
}

fn frontier_judge() -> ModelIdentity {
    ModelIdentity {
        tier: Tier::T5,
        provider: "anthropic".to_owned(),
        model: "frontier-judge".to_owned(),
        thinking: "medium".to_owned(),
    }
}

fn assert_infrastructure_pause(
    state: &FrontierRunState,
    expected_events: usize,
    maximum_attempt: u8,
) {
    assert!(matches!(state.status, model::FrontierRunStatus::Paused));
    assert!(matches!(
        state.pause,
        Some(model::PoolPauseReason::Infrastructure { .. })
    ));
    assert_eq!(state.infrastructure_events.len(), expected_events);
    let events_per_attempt = expected_events / usize::from(maximum_attempt);
    for attempt in 1..=maximum_attempt {
        assert_eq!(
            state
                .infrastructure_events
                .iter()
                .filter(|event| event.infrastructure_attempt == attempt)
                .count(),
            events_per_attempt
        );
    }
    assert!(state.cells.is_empty());
    assert_eq!(state.spent_millionths_of_dollar, 0);
}

fn assert_guards_precede_process(events: &[&str]) {
    let first_process = events.iter().position(|event| *event == "process").unwrap();
    let required = [
        "suite",
        "plan",
        "catalog",
        "environment",
        "source",
        "harness",
        "spend",
        "authority",
    ];
    let positions = required
        .iter()
        .map(|guard| {
            let position = events.iter().position(|event| event == guard).unwrap();
            assert!(
                position < first_process,
                "{guard} guard ran after Process::run"
            );
            position
        })
        .collect::<Vec<_>>();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
}

fn assert_guarded_environment(
    log: &str,
    expected: &BTreeMap<&str, PathBuf>,
    expected_processes: usize,
) {
    for (name, value) in expected {
        let line = format!("env:{name}={}", value.display());
        assert_eq!(
            log.lines().filter(|entry| *entry == line).count(),
            expected_processes
        );
    }
    for (name, value) in [("PI_SKIP_VERSION_CHECK", "1"), ("PI_TELEMETRY", "0")] {
        let line = format!("env:{name}={value}");
        assert_eq!(
            log.lines().filter(|entry| *entry == line).count(),
            expected_processes
        );
    }
    assert!(!log.contains("API_KEY"));
    assert!(!log.contains("TOKEN"));
}

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
    assert!(artifact.is_dir());
    assert!(transcript.is_file());
    assert!(artifact.starts_with(&canonical_runs));
    assert!(transcript.starts_with(&canonical_runs));
    assert_eq!(
        fs::read_to_string(artifact.parent().unwrap().join("response.txt"))
            .unwrap()
            .trim(),
        "H4 fixture complete"
    );
    assert_eq!(
        fs::read_to_string(artifact.join("result.txt"))
            .unwrap()
            .trim(),
        "H4 disposable result"
    );
    let judge_evidence = artifact.parent().unwrap().join("judge-evidence");
    let attempts = fs::read_dir(&judge_evidence)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 1);
    let judge_packet = &attempts[0];
    assert!(judge_packet.join("artifact/result.txt").is_file());
    assert!(judge_packet.join("response.txt").is_file());
    assert!(judge_packet.join("rubric.md").is_file());
    assert!(judge_packet.join("checks.json").is_file());
    assert!(judge_packet.join("locked-read.ts").is_file());
    assert!(judge_packet.join("judge-transcript.jsonl").is_file());
    let sanitized = fs::read_to_string(judge_packet.join("transcript.jsonl")).unwrap();
    assert!(!sanitized.contains(CANDIDATE_PROVIDER));
    assert!(!sanitized.contains(CANDIDATE_MODEL));

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
    assert_eq!(record["judge_model"]["provider"], JUDGE_PROVIDER);
    assert_eq!(record["judge_model"]["model"], JUDGE_MODEL);
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
        record["judge_usage"]["tool_calls"]
            .as_u64()
            .is_some_and(|calls| calls > 0)
    );
    assert!(
        record["judge_usage"]["elapsed_milliseconds"]
            .as_u64()
            .is_some_and(|elapsed| elapsed > 0)
    );

    let transcript_events = json_lines(&fs::read(&transcript).unwrap());
    let transcript_tool_calls = transcript_events
        .iter()
        .filter(|event| event["type"] == "tool_execution_start")
        .count();
    assert_eq!(
        u64::try_from(transcript_tool_calls).unwrap(),
        record["candidate_usage"]["tool_calls"].as_u64().unwrap()
    );
    assert!(transcript_tool_calls >= 2);
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
            "T5": route(JUDGE_PROVIDER, JUDGE_MODEL, "medium")
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
