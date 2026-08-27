#[path = "../src/audit.rs"]
mod audit;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/ports.rs"]
mod ports;
#[path = "../src/publication.rs"]
mod publication;
#[path = "../src/service.rs"]
mod service;
#[path = "../src/statistics.rs"]
mod statistics;
#[path = "../src/t1_screen_campaign_store.rs"]
mod t1_screen_campaign_store;
#[path = "../src/t1_screen_store.rs"]
mod t1_screen_store;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use model::*;
use ports::*;
use service::{
    build_t1_screen_report, extend_t1_screen_cap, fail_t1_screen_route, pending_t1_screen_state,
    resume_t1_screening, start_t1_screening,
};
use sha2::{Digest, Sha256};
use t1_screen_campaign_store::T1_SCREEN_CAMPAIGN_APPROVED_TOTAL;
use t1_screen_store::{
    candidate_environment_manifest_digest, preallocate_t1_screen_children,
    t1_screen_classification_digest, validate_t1_screen_state, validate_t1_screen_transition,
};

const SNAPSHOT_BYTES: &[u8] = b"offline capability snapshot";

struct ChildIds(u64);

impl RunIdSource for ChildIds {
    fn next(&mut self) -> Result<RunId, SkillEvalError> {
        let id = self.0;
        self.0 += 1;
        Ok(RunId(format!("child-{id}")))
    }
}

#[derive(Clone)]
enum Stop {
    CandidateQuota,
    CandidateInfrastructure,
    JudgeQuota,
    JudgeInfrastructure,
}

struct FakeRuntime {
    artifact: ArtifactDefinition,
    parent: Option<T1ScreenRunState>,
    campaign: T1ScreenCampaignState,
    events: BTreeMap<RunId, Vec<RunEvent>>,
    scores: BTreeMap<(String, String), (u8, bool)>,
    stop: Option<Stop>,
    stop_at_execute_call: Option<(usize, Stop)>,
    snapshot: Vec<u8>,
    candidate_environment_manifest: Vec<CandidateEnvironmentEntry>,
    exact_model_suffix: String,
    judge_model_suffix: String,
    harness_suffix: String,
    candidate_cost: u64,
    judge_cost: u64,
    conservative_judge_cost: u64,
    execute_calls: usize,
    candidate_timeouts: Vec<Option<u32>>,
    verify_calls: usize,
    grade_calls: usize,
    exact_calls: Cell<usize>,
    judge_resolution_calls: Cell<usize>,
    parent_saves: usize,
    maximum_active_children: usize,
    is_parent_ready_before_execute: bool,
    is_campaign_ready_before_execute: bool,
    is_register_failure: bool,
    order: Rc<RefCell<Vec<&'static str>>>,
}

impl FakeRuntime {
    fn new(state: &T1ScreenRunState) -> Self {
        Self {
            artifact: state.configuration.exam.clone(),
            parent: None,
            campaign: T1ScreenCampaignState {
                campaign_id: state.configuration.campaign_id.clone(),
                created_at: now(),
                approved_judge_total_millionths_of_dollar: T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
                cap_extensions: Vec::new(),
                retirements: Vec::new(),
                aggregate_judge_spent_millionths_of_dollar: 0,
                runs: Vec::new(),
                active_run_id: None,
                owner_reason: "owner approved test campaign".to_owned(),
                status: T1ScreenCampaignStatus::Open,
            },
            events: BTreeMap::new(),
            scores: BTreeMap::new(),
            stop: None,
            stop_at_execute_call: None,
            snapshot: SNAPSHOT_BYTES.to_vec(),
            candidate_environment_manifest: state
                .configuration
                .candidate_environment
                .manifest
                .clone(),
            exact_model_suffix: String::new(),
            judge_model_suffix: String::new(),
            harness_suffix: String::new(),
            candidate_cost: 0,
            judge_cost: 1,
            conservative_judge_cost: 1,
            execute_calls: 0,
            candidate_timeouts: Vec::new(),
            verify_calls: 0,
            grade_calls: 0,
            exact_calls: Cell::new(0),
            judge_resolution_calls: Cell::new(0),
            parent_saves: 0,
            maximum_active_children: 0,
            is_parent_ready_before_execute: true,
            is_campaign_ready_before_execute: true,
            is_register_failure: false,
            order: Rc::new(RefCell::new(Vec::new())),
        }
    }

    fn calls(&self) -> (usize, usize, usize) {
        (self.execute_calls, self.verify_calls, self.grade_calls)
    }

    fn score(&mut self, thinking: &str, case: &str, score: u8, is_catastrophic: bool) {
        self.scores.insert(
            (thinking.to_owned(), case.to_owned()),
            (score, is_catastrophic),
        );
    }

    fn run_events(&self) -> impl Iterator<Item = &RunEvent> {
        self.events.values().flatten()
    }

    fn observed_campaign(&self) -> T1ScreenCampaignState {
        let mut campaign = self.campaign.clone();
        if campaign.runs.is_empty()
            && let Some(parent) = &self.parent
        {
            campaign.runs.push(campaign_entry(parent));
            campaign.aggregate_judge_spent_millionths_of_dollar =
                parent.spent_judge_millionths_of_dollar;
            match parent.status {
                T1ScreenRunStatus::Pending | T1ScreenRunStatus::Running => {
                    campaign.active_run_id = Some(parent.configuration.run_id.clone());
                    campaign.status = T1ScreenCampaignStatus::Open;
                }
                T1ScreenRunStatus::Paused => {
                    campaign.active_run_id = Some(parent.configuration.run_id.clone());
                    campaign.status = T1ScreenCampaignStatus::Paused;
                }
                T1ScreenRunStatus::AwaitingOwner => {
                    campaign.active_run_id = None;
                    campaign.status = T1ScreenCampaignStatus::AwaitingOwner;
                }
                T1ScreenRunStatus::Completed => {
                    campaign.active_run_id = None;
                    campaign.status = T1ScreenCampaignStatus::Closed;
                }
                T1ScreenRunStatus::Failed => {
                    campaign.active_run_id = None;
                    campaign.status = T1ScreenCampaignStatus::Open;
                }
            }
        }
        campaign
    }
}

impl ArtifactSource for FakeRuntime {
    fn load(&self, _root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
        Ok(self.artifact.clone())
    }
}

impl ModelResolver for FakeRuntime {
    fn candidates(&self, _tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
        unreachable!()
    }

    fn qualification_routes(&self, _tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
        unreachable!()
    }

    fn exact_candidate(&self, requested: &ModelIdentity) -> Result<ModelIdentity, SkillEvalError> {
        self.exact_calls.set(self.exact_calls.get() + 1);
        let mut model = requested.clone();
        model.model.push_str(&self.exact_model_suffix);
        Ok(model)
    }

    fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
        Ok(Tier::T5)
    }

    fn judge(
        &self,
        _judge_tier: Tier,
        _candidate: Option<&ModelIdentity>,
    ) -> Result<ModelIdentity, SkillEvalError> {
        unreachable!()
    }

    fn pool_judge(&self, _candidate: &ModelIdentity) -> Result<ModelIdentity, SkillEvalError> {
        self.judge_resolution_calls
            .set(self.judge_resolution_calls.get() + 1);
        let mut identity = judge();
        identity.model.push_str(&self.judge_model_suffix);
        Ok(identity)
    }
}

impl HarnessResolver for FakeRuntime {
    fn identity(
        &self,
        artifact: &ArtifactDefinition,
        _execution: &ExecutionDefinition,
    ) -> Result<HarnessIdentity, SkillEvalError> {
        let mut identity = harness(artifact);
        identity.runner_version.push_str(&self.harness_suffix);
        Ok(identity)
    }
}

impl RunIdSource for FakeRuntime {
    fn next(&mut self) -> Result<RunId, SkillEvalError> {
        unreachable!()
    }
}

impl CandidateRunner for FakeRuntime {
    fn execute(
        &mut self,
        run_id: &RunId,
        key: &TrialKey,
        _artifact: &ArtifactDefinition,
        _case: &CaseDefinition,
        model: &ModelIdentity,
        harness: &HarnessIdentity,
        candidate_timeout_seconds: Option<u32>,
    ) -> Result<CandidateArtifact, SkillEvalError> {
        self.execute_calls += 1;
        self.candidate_timeouts.push(candidate_timeout_seconds);
        if self
            .stop_at_execute_call
            .as_ref()
            .is_some_and(|(call, _)| *call == self.execute_calls)
        {
            self.stop = self.stop_at_execute_call.take().map(|(_, stop)| stop);
        }
        self.is_parent_ready_before_execute &= self.parent.as_ref().is_some_and(|state| {
            state.status == T1ScreenRunStatus::Running
                && state.child_runs.iter().any(|child| {
                    child.run_id == *run_id && child.status == T1ScreenChildStatus::Running
                })
        });
        self.is_campaign_ready_before_execute &= self.parent.as_ref().is_some_and(|state| {
            self.campaign.active_run_id.as_ref() == Some(&state.configuration.run_id)
                && self.campaign.runs.iter().any(|entry| {
                    entry.run_id == state.configuration.run_id
                        && entry.observed_status == T1ScreenRunStatus::Running
                })
        });
        match self.stop.take() {
            Some(Stop::CandidateQuota) => {
                return Err(SkillEvalError::Quota {
                    model: model.clone(),
                    reset_at: Some(now()),
                });
            }
            Some(Stop::CandidateInfrastructure) => {
                return Err(SkillEvalError::Process {
                    program: "offline-fake".to_owned(),
                    exit_code: Some(75),
                    standard_error: "stopped".to_owned(),
                });
            }
            Some(other) => self.stop = Some(other),
            None => {}
        }
        Ok(CandidateArtifact {
            key: key.clone(),
            model: model.clone(),
            harness: harness.clone(),
            artifact_path: PathBuf::from(&run_id.0).join("artifact"),
            transcript_path: PathBuf::from(&run_id.0).join("transcript"),
            usage: usage(self.candidate_cost),
        })
    }
}

impl Verifier for FakeRuntime {
    fn verify(
        &mut self,
        _case: &CaseDefinition,
        _candidate: &CandidateArtifact,
    ) -> Result<Vec<CheckResult>, SkillEvalError> {
        self.verify_calls += 1;
        Ok(Vec::new())
    }
}

impl Judge for FakeRuntime {
    fn grade(
        &mut self,
        model: &ModelIdentity,
        input: &JudgeInput,
    ) -> Result<JudgeResult, SkillEvalError> {
        self.grade_calls += 1;
        match self.stop.take() {
            Some(Stop::JudgeQuota) => {
                return Err(SkillEvalError::Quota {
                    model: model.clone(),
                    reset_at: Some(now()),
                });
            }
            Some(Stop::JudgeInfrastructure) => {
                return Err(SkillEvalError::Verification("judge stopped".to_owned()));
            }
            Some(other) => self.stop = Some(other),
            None => {}
        }
        let (score, is_catastrophic) = self
            .scores
            .get(&(
                input.candidate.model.thinking.clone(),
                input.candidate.key.case.0.clone(),
            ))
            .copied()
            .unwrap_or((10, false));
        Ok(JudgeResult {
            verdict: TrialVerdict {
                score,
                is_catastrophic,
                failure_mode: None,
                checks: Vec::new(),
            },
            model: model.clone(),
            usage: usage(self.judge_cost),
        })
    }

    fn grade_prompt(
        &mut self,
        _model: &ModelIdentity,
        _request: &PromptJudgeRequest,
    ) -> Result<PromptJudgeResult, SkillEvalError> {
        unreachable!()
    }
}

impl RunStore for FakeRuntime {
    fn append(&mut self, run_id: &RunId, event: &RunEvent) -> Result<(), SkillEvalError> {
        self.events
            .entry(run_id.clone())
            .or_default()
            .push(event.clone());
        Ok(())
    }

    fn replay(
        &self,
        run_id: &RunId,
        visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
    ) -> Result<(), SkillEvalError> {
        let events = self
            .events
            .get(run_id)
            .ok_or_else(|| SkillEvalError::NotFound(run_id.0.clone()))?;
        for event in events {
            visitor(event.clone())?;
        }
        Ok(())
    }

    fn find_trial(&self, _selector: &TrialSelector) -> Result<TrialRecord, SkillEvalError> {
        unreachable!()
    }
}

impl Clock for FakeRuntime {
    fn now(&self) -> Timestamp {
        now()
    }
}

impl TierWriter for FakeRuntime {
    fn write(
        &mut self,
        _artifact: &ArtifactDefinition,
        _assignments: &[TierAssignment],
    ) -> Result<(), SkillEvalError> {
        unreachable!()
    }
}

impl T1ScreenStore for FakeRuntime {
    fn create_t1_screen(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
        validate_t1_screen_state(state)?;
        if self.parent.is_some() {
            return Err(SkillEvalError::InvalidConfiguration(
                "duplicate parent".to_owned(),
            ));
        }
        self.maximum_active_children = state
            .child_runs
            .iter()
            .filter(|child| child.status == T1ScreenChildStatus::Running)
            .count();
        self.parent = Some(state.clone());
        self.order.borrow_mut().push("save");
        Ok(())
    }

    fn load_t1_screen(&self, _run_id: &T1ScreenRunId) -> Result<T1ScreenRunState, SkillEvalError> {
        self.parent
            .clone()
            .ok_or_else(|| SkillEvalError::NotFound("parent".to_owned()))
    }

    fn save_t1_screen(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
        validate_t1_screen_state(state)?;
        validate_t1_screen_transition(self.parent.as_ref().unwrap(), state)?;
        self.maximum_active_children = self.maximum_active_children.max(
            state
                .child_runs
                .iter()
                .filter(|child| child.status == T1ScreenChildStatus::Running)
                .count(),
        );
        self.parent = Some(state.clone());
        self.parent_saves += 1;
        self.order.borrow_mut().push("save");
        Ok(())
    }

    fn load_t1_screen_campaign(
        &self,
        _campaign_id: &T1ScreenCampaignId,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        Ok(self.observed_campaign())
    }

    fn reconcile_t1_screen_campaign(
        &mut self,
        _campaign_id: &T1ScreenCampaignId,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        self.campaign = self.observed_campaign();
        if let Some(parent) = &self.parent
            && self.campaign.active_run_id.as_ref() == Some(&parent.configuration.run_id)
            && let Some(entry) = self
                .campaign
                .runs
                .iter_mut()
                .find(|entry| entry.run_id == parent.configuration.run_id)
        {
            entry.observed_status = parent.status;
            entry.judge_spend_millionths_of_dollar = parent.spent_judge_millionths_of_dollar;
            self.campaign.aggregate_judge_spent_millionths_of_dollar = self
                .campaign
                .runs
                .iter()
                .map(|run| run.judge_spend_millionths_of_dollar)
                .sum();
        }
        Ok(self.campaign.clone())
    }

    fn pause_t1_screen_campaign_for_budget(
        &mut self,
        _campaign_id: &T1ScreenCampaignId,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        self.campaign.status = T1ScreenCampaignStatus::Paused;
        Ok(self.campaign.clone())
    }

    fn register_t1_screen_campaign_run(
        &mut self,
        state: &T1ScreenRunState,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        if self.is_register_failure {
            return Err(SkillEvalError::InvalidConfiguration(
                "injected campaign registration failure".to_owned(),
            ));
        }
        if self.campaign.active_run_id.is_some()
            && self.campaign.active_run_id.as_ref() != Some(&state.configuration.run_id)
        {
            return Err(SkillEvalError::InvalidConfiguration(
                "campaign already active".to_owned(),
            ));
        }
        if self.campaign.runs.is_empty() {
            self.campaign.runs.push(campaign_entry(state));
        }
        self.campaign.active_run_id = Some(state.configuration.run_id.clone());
        self.order.borrow_mut().push("save");
        Ok(self.campaign.clone())
    }

    fn reconcile_t1_screen_campaign_run(
        &mut self,
        state: &T1ScreenRunState,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        let entry = self
            .campaign
            .runs
            .iter_mut()
            .find(|entry| entry.run_id == state.configuration.run_id)
            .unwrap();
        entry.observed_status = state.status;
        entry.judge_spend_millionths_of_dollar = state.spent_judge_millionths_of_dollar;
        entry.candidate_cost_millionths_of_dollar = state.candidate_usage.cost_millionths_of_dollar;
        self.campaign.aggregate_judge_spent_millionths_of_dollar = self
            .campaign
            .runs
            .iter()
            .map(|entry| entry.judge_spend_millionths_of_dollar)
            .sum();
        match state.status {
            T1ScreenRunStatus::Pending | T1ScreenRunStatus::Running => {
                self.campaign.active_run_id = Some(state.configuration.run_id.clone());
                self.campaign.status = T1ScreenCampaignStatus::Open;
            }
            T1ScreenRunStatus::Paused => {
                self.campaign.status = T1ScreenCampaignStatus::Paused;
            }
            T1ScreenRunStatus::AwaitingOwner => {
                self.campaign.active_run_id = None;
                self.campaign.status = T1ScreenCampaignStatus::AwaitingOwner;
            }
            T1ScreenRunStatus::Completed => {
                self.campaign.active_run_id = None;
                self.campaign.status = T1ScreenCampaignStatus::Closed;
            }
            T1ScreenRunStatus::Failed => {
                self.campaign.active_run_id = None;
                self.campaign.status = T1ScreenCampaignStatus::Open;
            }
        }
        self.order.borrow_mut().push("save");
        Ok(self.campaign.clone())
    }
}

impl QualificationRuntime for FakeRuntime {}

impl T1ScreenRuntime for FakeRuntime {
    fn capability_snapshot_bytes(&self, _path: &Path) -> Result<Vec<u8>, SkillEvalError> {
        Ok(self.snapshot.clone())
    }

    fn candidate_environment_manifest(
        &self,
    ) -> Result<Vec<CandidateEnvironmentEntry>, SkillEvalError> {
        Ok(self.candidate_environment_manifest.clone())
    }

    fn judge_cost_upper_bound(
        &self,
        _model: &ModelIdentity,
        _input: &JudgeInput,
    ) -> Result<u64, SkillEvalError> {
        Ok(self.judge_cost)
    }

    fn conservative_next_judge_cost_upper_bound(
        &self,
        _model: &ModelIdentity,
    ) -> Result<u64, SkillEvalError> {
        Ok(self.conservative_judge_cost)
    }
}

struct Progress {
    states: Vec<T1ScreenRunState>,
    is_persisted_first: bool,
    order: Rc<RefCell<Vec<&'static str>>>,
}

impl Progress {
    fn new(runtime: &FakeRuntime) -> Self {
        Self {
            states: Vec::new(),
            is_persisted_first: true,
            order: runtime.order.clone(),
        }
    }
}

impl T1ScreenProgressSink for Progress {
    fn emit_t1_screen(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
        self.is_persisted_first &= self.order.borrow().last() == Some(&"save");
        self.order.borrow_mut().push("progress");
        self.states.push(state.clone());
        Ok(())
    }
}

struct ReportingChildStore {
    order: Rc<RefCell<Vec<&'static str>>>,
    is_persisted_before_replay: Cell<bool>,
}

struct StoredChildEvents {
    events: BTreeMap<RunId, Vec<RunEvent>>,
}

impl RunStore for StoredChildEvents {
    fn append(&mut self, _run_id: &RunId, _event: &RunEvent) -> Result<(), SkillEvalError> {
        unreachable!()
    }

    fn replay(
        &self,
        run_id: &RunId,
        visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
    ) -> Result<(), SkillEvalError> {
        for event in self
            .events
            .get(run_id)
            .ok_or_else(|| SkillEvalError::NotFound(run_id.0.clone()))?
        {
            visitor(event.clone())?;
        }
        Ok(())
    }

    fn find_trial(&self, _selector: &TrialSelector) -> Result<TrialRecord, SkillEvalError> {
        unreachable!()
    }
}

impl RunStore for ReportingChildStore {
    fn append(&mut self, _run_id: &RunId, _event: &RunEvent) -> Result<(), SkillEvalError> {
        unreachable!()
    }

    fn replay(
        &self,
        run_id: &RunId,
        _visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
    ) -> Result<(), SkillEvalError> {
        self.is_persisted_before_replay
            .set(self.order.borrow().last() == Some(&"save"));
        Err(SkillEvalError::NotFound(run_id.0.clone()))
    }

    fn find_trial(&self, _selector: &TrialSelector) -> Result<TrialRecord, SkillEvalError> {
        unreachable!()
    }
}

fn state(levels: &[(&str, &[&str])], cap: u64) -> T1ScreenRunState {
    let eligible = levels
        .iter()
        .map(|(model, thinking)| T1ScreenEligibleRow {
            provider: "provider".to_owned(),
            model: (*model).to_owned(),
            supported_pi_thinking_levels: thinking
                .iter()
                .map(|level| (*level).to_owned())
                .collect(),
            is_preview: false,
        })
        .collect::<Vec<_>>();
    let artifact = artifact();
    let mut ids = ChildIds(0);
    let children = preallocate_t1_screen_children(&eligible, &mut ids).unwrap();
    let models = eligible
        .iter()
        .map(|row| T1ScreenModelState {
            provider: row.provider.clone(),
            model: row.model.clone(),
            attempts: Vec::new(),
            outcome: None,
        })
        .collect();
    let slots = u64::try_from(children.len()).unwrap();
    T1ScreenRunState {
        configuration: T1ScreenRunConfiguration {
            run_id: T1ScreenRunId("screen".to_owned()),
            campaign_id: T1ScreenCampaignId("campaign".to_owned()),
            created_at: now(),
            capability_snapshot: T1ScreenSnapshotIdentity {
                path: PathBuf::from("/offline/capabilities.json"),
                sha256: hex_digest(SNAPSHOT_BYTES),
                version: 1,
                observed_at_unix_seconds: 1,
                pi_version: "pi-1".to_owned(),
            },
            classification_sha256: t1_screen_classification_digest(&eligible, &[]).unwrap(),
            eligible,
            excluded: Vec::new(),
            exam: artifact.clone(),
            judge: judge(),
            candidate_environment: candidate_environment(vec![harness(&artifact); 5]),
            policy: T1ScreenPolicy {
                minimum_score: 8,
                calibration_minimum_reliability_basis_points: 8_000,
                maximum_catastrophic_trials: 0,
                repeats_per_case: 1,
                candidate_timeout_seconds: None,
            },
            is_complete_thinking_coverage: true,
            candidate_calls: T1ScreenCallRange {
                minimum: slots * 5,
                maximum: slots * 5,
            },
            judge_calls: T1ScreenCallRange {
                minimum: slots * 5,
                maximum: slots * 5,
            },
            candidate_price: T1ScreenCandidatePrice {
                input_per_million_tokens: 0,
                output_per_million_tokens: 0,
            },
            owner_approved_judge_cap_millionths_of_dollar: cap,
            provider_enforced_judge_cap_millionths_of_dollar: cap,
        },
        cap_extensions: Vec::new(),
        route_failures: Vec::new(),
        status: T1ScreenRunStatus::Pending,
        child_runs: children,
        models,
        candidate_usage: zero_usage(),
        judge_usage: zero_usage(),
        spent_judge_millionths_of_dollar: 0,
        pause: None,
    }
}

fn campaign_entry(state: &T1ScreenRunState) -> T1ScreenCampaignRunEntry {
    T1ScreenCampaignRunEntry {
        run_id: state.configuration.run_id.clone(),
        canonical_state_path: PathBuf::from("/offline/t1-screening/screen/state.json"),
        state_file_sha256: "a".repeat(64),
        created_at: state.configuration.created_at.clone(),
        observed_status: state.status,
        judge_spend_millionths_of_dollar: state.spent_judge_millionths_of_dollar,
        candidate_cost_millionths_of_dollar: state.candidate_usage.cost_millionths_of_dollar,
        is_resumable: true,
        superseded_reason: None,
    }
}

fn artifact() -> ArtifactDefinition {
    ArtifactDefinition {
        name: ArtifactName("exam".to_owned()),
        kind: ArtifactKind::Skill,
        root: PathBuf::from("/offline/exam"),
        revision: "exam-r1".to_owned(),
        required_destinations: vec![TierDestination::SkillMinimum],
        current_tiers: Vec::new(),
        cases: (0..5)
            .map(|index| CaseDefinition {
                id: CaseId(format!("case-{index}")),
                input: "input".to_owned(),
                expect: "expect".to_owned(),
                source: "offline".to_owned(),
                is_holdout: false,
                support_files: Vec::new(),
                execution: ExecutionDefinition {
                    drive: CaseDrive::Response,
                    allowed_tools: Vec::new(),
                    timeout_seconds: 10,
                },
            })
            .collect(),
    }
}

fn judge() -> ModelIdentity {
    ModelIdentity {
        tier: Tier::T5,
        provider: "judge".to_owned(),
        model: "fixed-judge".to_owned(),
        thinking: "high".to_owned(),
    }
}

fn candidate_environment(harnesses: Vec<HarnessIdentity>) -> T1ScreenCandidateEnvironment {
    let manifest = vec![CandidateEnvironmentEntry {
        key: "pi-agent/settings.json".to_owned(),
        sha256: "b".repeat(64),
    }];
    let digest = candidate_environment_manifest_digest(&manifest).unwrap();
    T1ScreenCandidateEnvironment {
        harnesses,
        manifest,
        digest,
    }
}

fn harness(artifact: &ArtifactDefinition) -> HarnessIdentity {
    HarnessIdentity {
        runner_version: "runner-1".to_owned(),
        pi_version: "pi-1".to_owned(),
        artifact_revision: artifact.revision.clone(),
        tool_policy_digest: "tools-1".to_owned(),
    }
}

fn usage(cost: u64) -> TrialUsage {
    TrialUsage {
        input_tokens: 1,
        output_tokens: 1,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        turns: 1,
        tool_calls: 0,
        elapsed_milliseconds: 1,
        cost_millionths_of_dollar: cost,
    }
}

fn zero_usage() -> TrialUsage {
    TrialUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        turns: 0,
        tool_calls: 0,
        elapsed_milliseconds: 0,
        cost_millionths_of_dollar: 0,
    }
}

fn now() -> Timestamp {
    Timestamp("2026-08-26T00:00:00-0400".to_owned())
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn start(
    initial: T1ScreenRunState,
    runtime: &mut FakeRuntime,
) -> Result<T1ScreenRunState, SkillEvalError> {
    let mut progress = Progress::new(runtime);
    let result = start_t1_screening(initial, runtime, &mut progress);
    assert!(progress.is_persisted_first);
    result
}

fn resume(
    initial: &T1ScreenRunState,
    runtime: &mut FakeRuntime,
) -> Result<T1ScreenRunState, SkillEvalError> {
    let mut progress = Progress::new(runtime);
    let result = resume_t1_screening(initial, runtime, &mut progress);
    assert!(progress.is_persisted_first);
    result
}

#[test]
fn t1_candidate_timeout_passes_the_exact_frozen_policy_value() {
    let mut initial = state(&[("a", &["off"])], 100);
    initial.configuration.policy.candidate_timeout_seconds = Some(41);
    let mut runtime = FakeRuntime::new(&initial);

    start(initial, &mut runtime).unwrap();

    assert_eq!(runtime.candidate_timeouts, vec![Some(41); 5]);
}

#[test]
fn thinking_pass_runs_every_level_before_selecting_the_lowest() {
    let initial = state(&[("a", &["off", "high", "max"])], 100);
    let mut runtime = FakeRuntime::new(&initial);
    let result = start(initial.clone(), &mut runtime).unwrap();

    assert_eq!(result.status, T1ScreenRunStatus::AwaitingOwner);
    assert_eq!(runtime.calls(), (15, 15, 15));
    assert!(
        result
            .child_runs
            .iter()
            .all(|child| child.status == T1ScreenChildStatus::Completed)
    );
    assert_eq!(result.models[0].attempts.len(), 3);
    assert!(matches!(
        result.models[0].outcome,
        Some(T1ScreenModelOutcome::Selected { ref model }) if model.thinking == "off"
    ));
    assert_eq!(result.candidate_usage.cost_millionths_of_dollar, 0);
    assert!(runtime.is_parent_ready_before_execute);
    assert!(runtime.is_campaign_ready_before_execute);
    assert_eq!(runtime.maximum_active_children, 1);
    let serialized = serde_json::to_value(&result).unwrap();
    for absent in ["recommendation", "decision", "publication", "tier_write"] {
        assert!(serialized.get(absent).is_none());
    }
    assert!(
        runtime
            .run_events()
            .any(|event| matches!(event, RunEvent::CandidateExecuted { .. }))
    );
    assert!(runtime.run_events().all(|event| {
        match event {
            RunEvent::TrialStarted { models, .. } => models.len() == 1,
            RunEvent::TrialCompleted { record, .. } => result
                .child_runs
                .iter()
                .any(|child| child.model == record.model),
            _ => true,
        }
    }));

    let calls = runtime.calls();
    assert_eq!(resume(&initial, &mut runtime).unwrap(), result);
    assert_eq!(runtime.calls(), calls);
}

#[test]
fn resume_complete_level_screen() {
    let initial = state(
        &[
            ("a", &["off", "medium", "high"]),
            ("b", &["minimal", "high"]),
        ],
        100,
    );
    let child_ids = initial
        .child_runs
        .iter()
        .map(|child| child.run_id.clone())
        .collect::<Vec<_>>();
    let mut runtime = FakeRuntime::new(&initial);
    runtime.score("off", "case-0", 10, false);
    runtime.score("medium", "case-0", 10, false);
    runtime.stop_at_execute_call = Some((11, Stop::CandidateInfrastructure));

    let paused = start(initial.clone(), &mut runtime).unwrap();

    assert_eq!(paused.status, T1ScreenRunStatus::Paused);
    assert_eq!(paused.models[0].attempts.len(), 2);
    assert!(paused.models[0].outcome.is_none());
    assert_eq!(paused.child_runs[2].status, T1ScreenChildStatus::Paused);
    assert_eq!(paused.spent_judge_millionths_of_dollar, 10);
    assert_eq!(runtime.calls(), (11, 10, 10));

    let complete = resume(&initial, &mut runtime).unwrap();

    assert_eq!(complete.status, T1ScreenRunStatus::AwaitingOwner);
    assert_eq!(runtime.calls(), (26, 25, 25));
    assert_eq!(complete.spent_judge_millionths_of_dollar, 25);
    assert_eq!(complete.candidate_usage.cost_millionths_of_dollar, 0);
    assert_eq!(complete.judge_usage.cost_millionths_of_dollar, 25);
    assert_eq!(complete.models[0].attempts.len(), 3);
    assert_eq!(complete.models[1].attempts.len(), 2);
    assert!(matches!(
        complete.models[0].outcome,
        Some(T1ScreenModelOutcome::Selected { ref model }) if model.thinking == "off"
    ));
    assert!(matches!(
        complete.models[1].outcome,
        Some(T1ScreenModelOutcome::Selected { ref model }) if model.thinking == "minimal"
    ));
    assert_eq!(
        complete
            .child_runs
            .iter()
            .map(|child| child.run_id.clone())
            .collect::<Vec<_>>(),
        child_ids
    );
    assert!(
        complete
            .child_runs
            .iter()
            .all(|child| child.status == T1ScreenChildStatus::Completed)
    );
    assert_eq!(runtime.candidate_timeouts, vec![None; 26]);
    let calls = runtime.calls();
    assert_eq!(resume(&initial, &mut runtime).unwrap(), complete);
    assert_eq!(runtime.calls(), calls);
}

#[test]
fn failed_campaign_registration_leaves_pending_parent_and_makes_no_candidate_or_judge_call() {
    let initial = state(&[("a", &["off"])], 100);
    let mut runtime = FakeRuntime::new(&initial);
    runtime.is_register_failure = true;

    let error = start(initial.clone(), &mut runtime).unwrap_err();
    assert!(
        matches!(error, SkillEvalError::InvalidConfiguration(message) if message.contains("registration"))
    );
    assert_eq!(runtime.calls(), (0, 0, 0));
    assert_eq!(
        runtime.parent.as_ref().unwrap().status,
        T1ScreenRunStatus::Pending
    );
    assert!(runtime.campaign.runs.is_empty());

    runtime.is_register_failure = false;
    let completed = start(initial, &mut runtime).unwrap();
    assert_eq!(completed.status, T1ScreenRunStatus::AwaitingOwner);
    assert_eq!(runtime.campaign.runs.len(), 1);
    assert_eq!(runtime.calls(), (5, 5, 5));
}

#[test]
fn crash_reconciliation_catches_campaign_up_before_resume_projection() {
    let initial = state(&[("a", &["off"]), ("b", &["off"])], 1);
    let mut runtime = FakeRuntime::new(&initial);
    let paused = start(initial.clone(), &mut runtime).unwrap();
    assert_eq!(paused.spent_judge_millionths_of_dollar, 1);
    assert_eq!(
        runtime.campaign.aggregate_judge_spent_millionths_of_dollar,
        1
    );

    runtime.campaign.runs[0].judge_spend_millionths_of_dollar = 0;
    runtime.campaign.aggregate_judge_spent_millionths_of_dollar = 0;
    let calls = runtime.calls();
    let resumed = resume(&initial, &mut runtime).unwrap();

    assert_eq!(resumed.status, T1ScreenRunStatus::Paused);
    assert_eq!(runtime.calls(), calls);
    assert_eq!(
        runtime.campaign.aggregate_judge_spent_millionths_of_dollar,
        1
    );
    assert_eq!(runtime.campaign.runs[0].judge_spend_millionths_of_dollar, 1);
}

#[test]
fn imported_spend_blocks_every_restart_when_remaining_is_below_conservative_bound() {
    let initial = state(&[("a", &["off"])], 20_000_000);
    let mut runtime = FakeRuntime::new(&initial);
    runtime.conservative_judge_cost = 9_450_000;
    let mut historical = campaign_entry(&initial);
    historical.run_id = T1ScreenRunId("historical".to_owned());
    historical.created_at = Timestamp("2026-08-25T00:00:00-0400".to_owned());
    historical.observed_status = T1ScreenRunStatus::Paused;
    historical.judge_spend_millionths_of_dollar = 13_672_958;
    historical.is_resumable = false;
    historical.superseded_reason = Some("legacy environment".to_owned());
    runtime.campaign.runs.push(historical);
    runtime.campaign.aggregate_judge_spent_millionths_of_dollar = 13_672_958;

    let first_error = start(initial.clone(), &mut runtime).unwrap_err();
    assert!(
        matches!(first_error, SkillEvalError::InvalidConfiguration(message)
            if message.contains("6327042") && message.contains("9450000"))
    );
    assert_eq!(runtime.campaign.status, T1ScreenCampaignStatus::Paused);

    let mut restart = initial;
    restart.configuration.run_id = T1ScreenRunId("screen-restart".to_owned());
    let restart_error = start(restart, &mut runtime).unwrap_err();
    assert!(
        matches!(restart_error, SkillEvalError::InvalidConfiguration(message)
            if message.contains("not open"))
    );
    assert_eq!(runtime.calls(), (0, 0, 0));
    assert!(runtime.parent.is_none());
    assert_eq!(
        runtime.campaign.aggregate_judge_spent_millionths_of_dollar,
        13_672_958
    );
}

#[test]
fn thinking_levels_complete_in_sparse_order_and_can_exhaust() {
    let initial = state(
        &[("a", &["off", "high", "max"]), ("b", &["minimal", "max"])],
        100,
    );
    let mut runtime = FakeRuntime::new(&initial);
    for case in 0..5 {
        runtime.score("off", &format!("case-{case}"), 1, false);
        runtime.score("minimal", &format!("case-{case}"), 1, false);
        runtime.score("max", &format!("case-{case}"), 1, false);
    }
    let result = start(initial, &mut runtime).unwrap();

    assert_eq!(runtime.calls(), (25, 25, 25));
    assert!(matches!(
        result.models[0].outcome,
        Some(T1ScreenModelOutcome::Selected { ref model }) if model.thinking == "high"
    ));
    assert_eq!(result.models[0].attempts.len(), 3);
    assert_eq!(result.child_runs[2].status, T1ScreenChildStatus::Completed);
    assert_eq!(
        result.models[1].outcome,
        Some(T1ScreenModelOutcome::Exhausted)
    );
    assert_eq!(result.child_runs[4].status, T1ScreenChildStatus::Exhausted);
}

#[test]
fn eighty_percent_boundary_passes_but_one_catastrophic_trial_fails() {
    let initial = state(&[("a", &["off"]), ("b", &["off", "high"])], 100);
    let mut runtime = FakeRuntime::new(&initial);
    runtime.score("off", "case-4", 0, false);
    let result = start(initial, &mut runtime).unwrap();
    assert!(matches!(
        result.models[0].outcome,
        Some(T1ScreenModelOutcome::Selected { .. })
    ));

    let initial = state(&[("a", &["off", "high"])], 100);
    let mut runtime = FakeRuntime::new(&initial);
    runtime.score("off", "case-0", 10, true);
    let result = start(initial, &mut runtime).unwrap();
    assert_eq!(result.models[0].attempts.len(), 2);
    assert!(result.models[0].attempts[0].evidence.catastrophic_trials > 0);
}

#[test]
fn candidate_checkpoint_survives_judge_pause_and_resume_without_duplicate_candidate() {
    for stop in [Stop::JudgeQuota, Stop::JudgeInfrastructure] {
        let initial = state(&[("a", &["off"])], 100);
        let mut runtime = FakeRuntime::new(&initial);
        runtime.stop = Some(stop);
        let paused = start(initial.clone(), &mut runtime).unwrap();
        assert_eq!(paused.status, T1ScreenRunStatus::Paused);
        assert_eq!(runtime.calls(), (1, 1, 1));
        assert!(
            runtime
                .run_events()
                .any(|event| matches!(event, RunEvent::CandidateExecuted { .. }))
        );

        let complete = resume(&initial, &mut runtime).unwrap();
        assert_eq!(complete.status, T1ScreenRunStatus::AwaitingOwner);
        assert_eq!(runtime.calls(), (5, 6, 6));
        let calls = runtime.calls();
        resume(&initial, &mut runtime).unwrap();
        assert_eq!(runtime.calls(), calls);
    }
}

#[test]
fn candidate_quota_and_infrastructure_pause_keep_the_same_child_id() {
    for stop in [Stop::CandidateQuota, Stop::CandidateInfrastructure] {
        let initial = state(&[("a", &["off"])], 100);
        let child_id = initial.child_runs[0].run_id.clone();
        let mut runtime = FakeRuntime::new(&initial);
        runtime.stop = Some(stop);
        let paused = start(initial.clone(), &mut runtime).unwrap();
        assert_eq!(paused.child_runs[0].run_id, child_id);
        assert_eq!(paused.child_runs[0].status, T1ScreenChildStatus::Paused);
        assert_eq!(runtime.calls(), (1, 0, 0));
        let complete = resume(&initial, &mut runtime).unwrap();
        assert_eq!(complete.child_runs[0].run_id, child_id);
        assert_eq!(runtime.calls(), (6, 5, 5));
    }
}

#[test]
fn exact_judge_cap_boundary_pauses_before_the_next_judge_call() {
    let initial = state(&[("a", &["off"]), ("b", &["off"])], 5);
    let mut runtime = FakeRuntime::new(&initial);
    let paused = start(initial.clone(), &mut runtime).unwrap();
    assert_eq!(paused.status, T1ScreenRunStatus::Paused);
    assert!(matches!(
        paused.pause,
        Some(T1ScreenPauseReason::JudgeCap {
            spent_millionths_of_dollar: 5,
            ..
        })
    ));
    assert_eq!(runtime.calls(), (6, 6, 5));
    assert_eq!(paused.spent_judge_millionths_of_dollar, 5);
    assert_eq!(paused.candidate_usage.cost_millionths_of_dollar, 0);

    let calls = runtime.calls();
    let paused_again = resume(&initial, &mut runtime).unwrap();
    assert_eq!(paused_again.status, T1ScreenRunStatus::Paused);
    assert_eq!(runtime.calls(), calls);
}

#[test]
fn route_failure_advances_exact_child_and_resume_starts_next_model() {
    let initial = state(
        &[("a", &["off", "high"]), ("b", &["off"])],
        T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
    );
    let failed_child = initial.child_runs[0].clone();
    let next_model_child = initial.child_runs[2].run_id.clone();
    let mut runtime = FakeRuntime::new(&initial);
    runtime.stop = Some(Stop::CandidateInfrastructure);
    let paused = start(initial, &mut runtime).unwrap();
    assert_eq!(paused.status, T1ScreenRunStatus::Paused);
    assert_eq!(paused.child_runs[0].status, T1ScreenChildStatus::Paused);
    let calls = runtime.calls();
    let child_store = StoredChildEvents {
        events: runtime.events.clone(),
    };
    let request = T1ScreenRouteFailureRequest {
        run_id: paused.configuration.run_id.clone(),
        child_run_id: failed_child.run_id.clone(),
        owner_reason: "Owner accepted the exact route failure".to_owned(),
    };

    let report =
        fail_t1_screen_route(&request, &mut runtime, &child_store, &route_failure_clock()).unwrap();

    assert_eq!(runtime.calls(), calls);
    assert_eq!(report.route_failures.len(), 1);
    assert_eq!(report.route_failures[0].child_run_id, failed_child.run_id);
    assert_eq!(report.route_failures[0].model, failed_child.model);
    assert_eq!(report.status, T1ScreenRunStatus::Running);
    assert_eq!(report.campaign_status, T1ScreenCampaignStatus::Open);
    assert_eq!(
        report.campaign_aggregate_judge_spent_millionths_of_dollar,
        paused.spent_judge_millionths_of_dollar
    );
    let saved = runtime.parent.as_ref().unwrap();
    assert_eq!(saved.child_runs[0].status, T1ScreenChildStatus::Failed);
    assert_eq!(saved.child_runs[1].status, T1ScreenChildStatus::Skipped);
    assert!(matches!(
        saved.models[0].outcome,
        Some(T1ScreenModelOutcome::InfrastructureFailed { .. })
    ));

    let pending = pending_t1_screen_state(saved).unwrap();
    let complete = resume(&pending, &mut runtime).unwrap();
    assert_eq!(complete.status, T1ScreenRunStatus::AwaitingOwner);
    assert_eq!(complete.child_runs[0].status, T1ScreenChildStatus::Failed);
    assert_eq!(complete.child_runs[1].status, T1ScreenChildStatus::Skipped);
    assert_eq!(complete.child_runs[2].run_id, next_model_child);
    assert_eq!(runtime.calls().0, calls.0 + 5);
}

#[test]
fn route_failure_after_a_scored_pass_stays_distinct_and_skips_only_stronger_levels() {
    let initial = state(
        &[("a", &["off", "high", "max"])],
        T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
    );
    let mut runtime = FakeRuntime::new(&initial);
    runtime.stop_at_execute_call = Some((6, Stop::CandidateInfrastructure));
    let paused = start(initial, &mut runtime).unwrap();
    assert_eq!(paused.models[0].attempts.len(), 1);
    assert!(paused.models[0].attempts[0].evidence.is_passing);
    assert_eq!(paused.child_runs[1].status, T1ScreenChildStatus::Paused);
    let child_store = StoredChildEvents {
        events: runtime.events.clone(),
    };
    let request = T1ScreenRouteFailureRequest {
        run_id: paused.configuration.run_id.clone(),
        child_run_id: paused.child_runs[1].run_id.clone(),
        owner_reason: "Owner accepted the stronger exact route failure".to_owned(),
    };

    let report =
        fail_t1_screen_route(&request, &mut runtime, &child_store, &route_failure_clock()).unwrap();

    let saved = runtime.parent.as_ref().unwrap();
    assert_eq!(report.route_failures.len(), 1);
    assert_eq!(saved.models[0].attempts.len(), 1);
    assert!(saved.models[0].attempts[0].evidence.is_passing);
    assert_eq!(saved.child_runs[0].status, T1ScreenChildStatus::Completed);
    assert_eq!(saved.child_runs[1].status, T1ScreenChildStatus::Failed);
    assert_eq!(saved.child_runs[2].status, T1ScreenChildStatus::Skipped);
    assert!(matches!(
        saved.models[0].outcome,
        Some(T1ScreenModelOutcome::InfrastructureFailed { ref model, .. })
            if model.thinking == "high"
    ));
}

#[test]
fn route_failure_preserves_exact_campaign_spend() {
    const APPROVED_TOTAL: u64 = 66_038_087;
    const AGGREGATE_SPEND: u64 = 22_047_006;

    let initial = state(&[("a", &["off"])], APPROVED_TOTAL);
    let mut runtime = FakeRuntime::new(&initial);
    runtime.campaign.approved_judge_total_millionths_of_dollar = APPROVED_TOTAL;
    runtime.stop = Some(Stop::CandidateInfrastructure);
    let paused = start(initial, &mut runtime).unwrap();
    let parent = runtime.parent.as_mut().unwrap();
    parent.judge_usage.cost_millionths_of_dollar = AGGREGATE_SPEND;
    parent.spent_judge_millionths_of_dollar = AGGREGATE_SPEND;
    let campaign_entry = runtime
        .campaign
        .runs
        .iter_mut()
        .find(|entry| entry.run_id == paused.configuration.run_id)
        .unwrap();
    campaign_entry.judge_spend_millionths_of_dollar = AGGREGATE_SPEND;
    runtime.campaign.aggregate_judge_spent_millionths_of_dollar = AGGREGATE_SPEND;
    let child_store = StoredChildEvents {
        events: runtime.events.clone(),
    };
    let request = T1ScreenRouteFailureRequest {
        run_id: paused.configuration.run_id,
        child_run_id: paused.child_runs[0].run_id.clone(),
        owner_reason: "Owner accepted the exact route failure".to_owned(),
    };
    let calls = runtime.calls();

    let report =
        fail_t1_screen_route(&request, &mut runtime, &child_store, &route_failure_clock()).unwrap();

    assert_eq!(runtime.calls(), calls);
    assert_eq!(
        report.campaign_approved_judge_total_millionths_of_dollar,
        APPROVED_TOTAL
    );
    assert_eq!(
        report.campaign_aggregate_judge_spent_millionths_of_dollar,
        AGGREGATE_SPEND
    );
    assert_eq!(
        report.campaign_remaining_judge_millionths_of_dollar,
        APPROVED_TOTAL - AGGREGATE_SPEND
    );
    assert_eq!(report.spent_judge_millionths_of_dollar, AGGREGATE_SPEND);
}

#[test]
fn route_failure_rejects_wrong_pause_child_reason_and_repeated_route() {
    let initial = state(&[("a", &["off"])], T1_SCREEN_CAMPAIGN_APPROVED_TOTAL);
    let mut runtime = FakeRuntime::new(&initial);
    runtime.stop = Some(Stop::CandidateInfrastructure);
    let paused = start(initial, &mut runtime).unwrap();
    let child_store = StoredChildEvents {
        events: runtime.events.clone(),
    };
    let mut request = T1ScreenRouteFailureRequest {
        run_id: paused.configuration.run_id.clone(),
        child_run_id: paused.child_runs[0].run_id.clone(),
        owner_reason: "   ".to_owned(),
    };
    assert!(
        fail_t1_screen_route(&request, &mut runtime, &child_store, &route_failure_clock()).is_err()
    );
    request.owner_reason = "approved".to_owned();
    request.child_run_id = RunId("wrong-child".to_owned());
    assert!(
        fail_t1_screen_route(&request, &mut runtime, &child_store, &route_failure_clock()).is_err()
    );
    request.child_run_id = paused.child_runs[0].run_id.clone();
    assert!(
        fail_t1_screen_route(
            &request,
            &mut runtime,
            &child_store,
            &stale_route_failure_clock()
        )
        .is_err()
    );
    fail_t1_screen_route(&request, &mut runtime, &child_store, &route_failure_clock()).unwrap();
    assert!(
        fail_t1_screen_route(&request, &mut runtime, &child_store, &route_failure_clock()).is_err()
    );
    assert_eq!(runtime.calls(), (1, 0, 0));
}

#[test]
fn cap_extension_service_persists_before_report_without_runtime_or_publication_work() {
    let mut paused = state(&[("a", &["off"])], 15_000_000);
    paused.status = T1ScreenRunStatus::Paused;
    paused.child_runs[0].status = T1ScreenChildStatus::Paused;
    paused.judge_usage = usage(5_811_172);
    paused.spent_judge_millionths_of_dollar = 5_811_172;
    paused.pause = Some(T1ScreenPauseReason::JudgeCap {
        spent_millionths_of_dollar: 5_811_172,
        owner_approved_millionths_of_dollar: 15_000_000,
        provider_enforced_millionths_of_dollar: 15_000_000,
    });
    let before = paused.clone();
    let mut parent = FakeRuntime::new(&paused);
    parent.parent = Some(paused);
    let child_store = ReportingChildStore {
        order: parent.order.clone(),
        is_persisted_before_replay: Cell::new(false),
    };
    let request = T1ScreenCapExtensionRequest {
        run_id: T1ScreenRunId("screen".to_owned()),
        new_owner_cap_millionths_of_dollar: 20_000_000,
        new_provider_cap_millionths_of_dollar: 20_000_000,
        owner_reason: "Owner approved the remaining judge work".to_owned(),
    };

    let report =
        extend_t1_screen_cap(&request, &mut parent, &child_store, &parent_clock()).unwrap();

    assert!(child_store.is_persisted_before_replay.get());
    assert_eq!(parent.calls(), (0, 0, 0));
    assert_eq!(parent.parent_saves, 1);
    let saved = parent.parent.as_ref().unwrap();
    assert_eq!(saved.configuration, before.configuration);
    assert_eq!(saved.child_runs, before.child_runs);
    assert_eq!(saved.models, before.models);
    assert_eq!(saved.candidate_usage, before.candidate_usage);
    assert_eq!(saved.judge_usage, before.judge_usage);
    assert_eq!(saved.spent_judge_millionths_of_dollar, 5_811_172);
    assert_eq!(saved.pause, before.pause);
    assert_eq!(
        report.owner_approved_judge_cap_millionths_of_dollar,
        15_000_000
    );
    assert_eq!(
        report.provider_enforced_judge_cap_millionths_of_dollar,
        15_000_000
    );
    assert_eq!(
        report.effective_owner_approved_judge_cap_millionths_of_dollar,
        20_000_000
    );
    assert_eq!(
        report.effective_provider_enforced_judge_cap_millionths_of_dollar,
        20_000_000
    );
    assert_eq!(report.cap_extensions.len(), 1);
    assert!(report.is_owner_approval_required);
    assert!(report.ranking.is_none());
    let value = serde_json::to_value(report).unwrap();
    for absent in ["recommendation", "decision", "publication", "tier_write"] {
        assert!(value.get(absent).is_none());
    }
}

#[test]
fn invalid_cap_extension_makes_no_save_or_runtime_call() {
    let initial = state(&[("a", &["off"])], 100);
    let mut parent = FakeRuntime::new(&initial);
    parent.parent = Some(initial);
    let child_store = ReportingChildStore {
        order: parent.order.clone(),
        is_persisted_before_replay: Cell::new(false),
    };
    for (owner, provider, reason) in [
        (200, 200, "approved"),
        (100, 100, "approved"),
        (200, 201, "approved"),
        (200, 200, "   "),
    ] {
        let request = T1ScreenCapExtensionRequest {
            run_id: T1ScreenRunId("screen".to_owned()),
            new_owner_cap_millionths_of_dollar: owner,
            new_provider_cap_millionths_of_dollar: provider,
            owner_reason: reason.to_owned(),
        };
        let calls = parent.calls();
        assert!(
            extend_t1_screen_cap(&request, &mut parent, &child_store, &parent_clock()).is_err()
        );
        assert_eq!(parent.calls(), calls);
        assert_eq!(parent.parent_saves, 0);
        assert!(!child_store.is_persisted_before_replay.get());
    }
}

#[test]
fn resume_after_extension_reuses_child_and_only_finishes_missing_judges() {
    let initial = state(&[("a", &["off"]), ("b", &["off"])], 5);
    let active_child = initial.child_runs[1].run_id.clone();
    let mut runtime = FakeRuntime::new(&initial);
    let paused = start(initial, &mut runtime).unwrap();
    assert_eq!(runtime.calls(), (6, 6, 5));
    assert_eq!(paused.child_runs[1].run_id, active_child);

    let mut extended = paused.clone();
    extended.cap_extensions.push(T1ScreenCapExtension {
        timestamp: Timestamp("2026-08-26T01:00:00-0400".to_owned()),
        previous_owner_cap_millionths_of_dollar: 5,
        new_owner_cap_millionths_of_dollar: 10,
        previous_provider_cap_millionths_of_dollar: 5,
        new_provider_cap_millionths_of_dollar: 10,
        owner_reason: "approved".to_owned(),
    });
    runtime.save_t1_screen(&extended).unwrap();
    let calls_before_resume = runtime.calls();
    let pending = pending_t1_screen_state(&extended).unwrap();
    let mut progress = Progress::new(&runtime);
    let complete = resume_t1_screening(&pending, &mut runtime, &mut progress).unwrap();

    assert_eq!(complete.status, T1ScreenRunStatus::AwaitingOwner);
    assert_eq!(complete.child_runs[1].run_id, active_child);
    assert_eq!(calls_before_resume, (6, 6, 5));
    assert_eq!(runtime.calls(), (10, 11, 10));
    assert_eq!(
        complete
            .models
            .iter()
            .map(|model| model.attempts.len())
            .sum::<usize>(),
        2
    );
}

fn stale_route_failure_clock() -> impl Clock {
    struct Fixed;
    impl Clock for Fixed {
        fn now(&self) -> Timestamp {
            now()
        }
    }
    Fixed
}

fn route_failure_clock() -> impl Clock {
    struct Fixed;
    impl Clock for Fixed {
        fn now(&self) -> Timestamp {
            Timestamp("2026-08-26T01:00:00-0400".to_owned())
        }
    }
    Fixed
}

fn parent_clock() -> impl Clock {
    struct Fixed;
    impl Clock for Fixed {
        fn now(&self) -> Timestamp {
            Timestamp("2026-08-26T01:00:00-0400".to_owned())
        }
    }
    Fixed
}

#[test]
fn every_frozen_drift_rejects_before_candidate_and_judge_calls() {
    let initial = state(&[("a", &["off", "high"])], 100);
    let mut created = FakeRuntime::new(&initial);
    created.stop = Some(Stop::CandidateQuota);
    start(initial.clone(), &mut created).unwrap();

    for drift in [
        "manifest",
        "snapshot",
        "exam",
        "environment",
        "judge",
        "model",
        "configuration",
        "child",
    ] {
        let mut runtime = FakeRuntime::new(&initial);
        runtime.parent = created.parent.clone();
        runtime.events.clone_from(&created.events);
        match drift {
            "manifest" => runtime.candidate_environment_manifest[0].sha256 = "c".repeat(64),
            "snapshot" => runtime.snapshot.push(0),
            "exam" => runtime.artifact.revision.push('x'),
            "environment" => runtime.harness_suffix.push('x'),
            "judge" => runtime.judge_model_suffix.push('x'),
            "model" => runtime.exact_model_suffix.push('x'),
            "configuration" => {
                runtime
                    .parent
                    .as_mut()
                    .unwrap()
                    .configuration
                    .owner_approved_judge_cap_millionths_of_dollar += 1
            }
            "child" => runtime.parent.as_mut().unwrap().child_runs[0]
                .run_id
                .0
                .push('x'),
            _ => unreachable!(),
        }
        let calls = runtime.calls();
        let error = resume(&initial, &mut runtime).unwrap_err();
        if drift == "manifest" {
            assert!(matches!(
                error,
                SkillEvalError::InvalidConfiguration(message)
                    if message.ends_with(
                        "candidate environment drift: changed pi-agent/settings.json"
                    )
            ));
            assert_eq!(runtime.exact_calls.get(), 0);
            assert_eq!(runtime.judge_resolution_calls.get(), 0);
        }
        assert_eq!(runtime.calls(), calls, "{drift}");
    }
}

#[test]
fn complete_report_ranks_three_by_candidate_metrics_and_keeps_all_case_evidence() {
    let initial = state(
        &[
            ("a", &["off"]),
            ("b", &["off"]),
            ("c", &["off"]),
            ("d", &["off"]),
        ],
        1_000,
    );
    let mut runtime = FakeRuntime::new(&initial);
    start(initial, &mut runtime).unwrap();
    let parent = runtime.parent.as_mut().unwrap();
    for (index, model) in parent.models.iter_mut().enumerate() {
        let evidence = &mut model.attempts[0].evidence;
        evidence.candidate_usage.elapsed_milliseconds = [10, 10, 5, 10][index];
        evidence.failed_trials = [1, 0, 0, 0][index];
        evidence.judge_usage.elapsed_milliseconds = [1, 2, 999, 3][index];
        evidence.total_usage.elapsed_milliseconds = evidence.candidate_usage.elapsed_milliseconds
            + evidence.judge_usage.elapsed_milliseconds;
    }
    parent.candidate_usage.elapsed_milliseconds = 35;
    parent.judge_usage.elapsed_milliseconds = 1_005;
    let calls = runtime.calls();
    let saves = runtime.parent_saves;

    let report =
        build_t1_screen_report(&T1ScreenRunId("screen".to_owned()), &runtime, &runtime).unwrap();

    assert_eq!(runtime.calls(), calls);
    assert_eq!(runtime.parent_saves, saves);
    assert_eq!(report.total_inventory_count, 4);
    assert_eq!(report.candidate_environment_manifest_entry_count, 1);
    assert_eq!(
        report.candidate_environment_manifest_digest,
        report.candidate_environment.digest
    );
    assert!(report.models.iter().all(|model| {
        model.attempts.len() == 1
            && model.attempts[0].cases.len() == 5
            && model.attempts[0]
                .cases
                .iter()
                .all(|case| case.candidate.is_some() && case.trial.is_some())
    }));
    let ranking = report.ranking.unwrap();
    assert_eq!(ranking.recommendation_shortage_count, 0);
    assert_eq!(ranking.recommendations.len(), 3);
    assert_eq!(ranking.alternates.len(), 1);
    assert_eq!(
        ranking
            .recommendations
            .iter()
            .map(|route| route.model.model.as_str())
            .collect::<Vec<_>>(),
        ["c", "b", "d"]
    );
    assert_eq!(ranking.alternates[0].model.model, "a");
}

#[test]
fn report_has_no_ranking_until_terminal_and_shortage_has_no_recommendation() {
    let initial = state(&[("a", &["off"]), ("b", &["off"])], 100);
    let mut pending_runtime = FakeRuntime::new(&initial);
    pending_runtime.parent = Some(initial.clone());
    let pending = build_t1_screen_report(
        &T1ScreenRunId("screen".to_owned()),
        &pending_runtime,
        &pending_runtime,
    )
    .unwrap();
    assert!(pending.ranking.is_none());

    let mut runtime = FakeRuntime::new(&initial);
    start(initial, &mut runtime).unwrap();
    let complete =
        build_t1_screen_report(&T1ScreenRunId("screen".to_owned()), &runtime, &runtime).unwrap();
    let ranking = complete.ranking.unwrap();
    assert_eq!(ranking.recommendation_shortage_count, 1);
    assert!(ranking.recommendations.is_empty());
    assert_eq!(ranking.alternates.len(), 2);
}

#[test]
fn nonzero_candidate_cost_and_campaign_bound_fail_closed() {
    let initial = state(&[("a", &["off"])], 100);
    let mut runtime = FakeRuntime::new(&initial);
    runtime.candidate_cost = 1;
    let error = start(initial, &mut runtime).unwrap_err();
    assert!(
        matches!(error, SkillEvalError::InvalidConfiguration(message) if message.contains("cost"))
    );
    assert_eq!(runtime.grade_calls, 0);

    let initial = state(&[("a", &["off"])], u64::MAX);
    let mut runtime = FakeRuntime::new(&initial);
    runtime.judge_cost = u64::MAX;
    runtime.conservative_judge_cost = 1;
    runtime.stop = Some(Stop::JudgeQuota);
    let paused = start(initial.clone(), &mut runtime).unwrap();
    assert_eq!(paused.status, T1ScreenRunStatus::Paused);
    runtime.stop = None;
    let paused_again = resume(&initial, &mut runtime).unwrap();
    assert_eq!(paused_again.status, T1ScreenRunStatus::Paused);
    assert!(matches!(
        paused_again.pause,
        Some(T1ScreenPauseReason::JudgeCap { .. })
    ));
    assert_eq!(runtime.grade_calls, 0);
}
