// TODO(AGNT-0032.T106): Prove per-model thinking selection before cross-model promotion.
#[macro_export]
macro_rules! pool_qualification_tests {
    () => {
        mod pool_qualification {
            use std::cell::RefCell;
            use std::collections::{BTreeMap, BTreeSet};
            use std::path::{Path, PathBuf};

            use $crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, CandidateArtifact, CaseDefinition,
                CaseDrive, CaseId, CheckResult, ExecutionDefinition, HarnessIdentity, JudgeInput,
                JudgeResult, ModelIdentity, PoolChildStatus, PoolEntrant, PoolPauseReason,
                PoolPlan, PoolPolicy, PoolQualifyRequest, PoolRunId, PoolRunState, PoolRunStatus,
                PromptJudgeRequest, PromptJudgeResult, RunEvent, RunId, SkillEvalError, Tier,
                TierAssignment, TierDestination, Timestamp, TrialKey, TrialRecord, TrialSelector,
                TrialUsage, TrialVerdict,
            };
            use $crate::ports::{
                ArtifactSource, CandidateRunner, Clock, HarnessResolver, Judge, ModelResolver,
                PoolPlanSource, PoolProgressSink, PoolRunIdSource, PoolRuntime, PoolStore,
                QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
            };

            use super::{resume_pool_qualification, start_pool_qualification};

            #[test]
            fn one_tier_progresses_three_to_two_with_stable_ids_and_terminal_skip() {
                let mut runtime = FakeRuntime::new();
                let mut progress = FakeProgress::default();
                let state = run_to_review(&mut runtime, vec![Tier::T2], &mut progress);

                assert_eq!(state.status, PoolRunStatus::AwaitingDecision);
                assert_eq!(state.pools.len(), 1);
                assert_eq!(state.pools[0].calibration.len(), 3);
                assert_eq!(
                    state.pools[0].promoted,
                    vec![model(Tier::T2, 0), model(Tier::T2, 1)]
                );
                assert_eq!(state.pools[0].qualification.len(), 2);
                assert_eq!(
                    state.pools[0].ranked,
                    vec![model(Tier::T2, 0), model(Tier::T2, 1)]
                );
                assert!(state.pools[0].is_complete);
                assert_eq!(
                    runtime.started,
                    vec!["child-0", "child-2", "child-4", "child-1", "child-3"]
                );
                assert_eq!(state.child_runs[5].status, PoolChildStatus::Skipped);
                assert!(runtime.is_unpromoted_skipped_before_full_call);
                assert_eq!(runtime.next_sequence, 6);
                assert!(progress.is_monotonic());
                assert!(
                    state.pools[0]
                        .qualification
                        .iter()
                        .all(|evidence| evidence.completed_trials == 3)
                );
            }

            #[test]
            fn five_tiers_complete_in_selected_order_without_running_the_control() {
                let mut runtime = FakeRuntime::new();
                let mut progress = FakeProgress::default();
                let tiers = vec![Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5];
                let state = run_to_review(&mut runtime, tiers.clone(), &mut progress);

                assert_eq!(state.status, PoolRunStatus::AwaitingDecision);
                assert_eq!(state.pools.len(), 5);
                assert!(state.pools.iter().all(|pool| pool.is_complete));
                assert_eq!(
                    state.pools.iter().map(|pool| pool.tier).collect::<Vec<_>>(),
                    tiers
                );
                assert!(
                    runtime
                        .executed_models
                        .iter()
                        .all(|model| *model != runtime.plan.control)
                );
                assert_eq!(runtime.started.len(), 25);
                assert_eq!(runtime.next_sequence, 30);
                assert!(progress.is_monotonic());
            }

            #[test]
            fn a_failed_finalist_stops_for_review_without_backfill() {
                let mut runtime = FakeRuntime::new();
                runtime.failed_full_runs.insert("child-3".to_owned());
                let state =
                    run_to_review(&mut runtime, vec![Tier::T2], &mut FakeProgress::default());

                assert_eq!(state.status, PoolRunStatus::AwaitingDecision);
                assert_eq!(state.pools[0].qualification.len(), 2);
                assert!(!state.pools[0].is_complete);
                assert!(state.pools[0].ranked.is_empty());
                assert_eq!(state.child_runs[5].status, PoolChildStatus::Skipped);
                assert_eq!(runtime.started.last().map(String::as_str), Some("child-3"));
            }

            #[test]
            fn fewer_than_two_calibration_passers_retain_evidence_for_owner_review() {
                let mut runtime = FakeRuntime::new();
                runtime.low_models.push(model(Tier::T2, 1));
                runtime.low_models.push(model(Tier::T2, 2));
                let state =
                    run_to_review(&mut runtime, vec![Tier::T2], &mut FakeProgress::default());

                assert_eq!(state.status, PoolRunStatus::AwaitingDecision);
                assert_eq!(state.pools[0].calibration.len(), 3);
                assert_eq!(state.pools[0].promoted, vec![model(Tier::T2, 0)]);
                assert!(state.pools[0].qualification.is_empty());
                assert_eq!(runtime.started, vec!["child-0", "child-2", "child-4"]);
            }

            #[test]
            fn spending_limit_pauses_before_the_next_child_call() {
                let mut runtime = FakeRuntime::new();
                runtime.plan.policy.spending_limit_millionths_of_dollar = 1;
                let mut progress = FakeProgress::default();
                let first =
                    start_pool_qualification(request(vec![Tier::T2]), &mut runtime, &mut progress)
                        .unwrap();
                assert_eq!(first.spent_millionths_of_dollar, 1);
                let calls = runtime.executed_models.len();

                let paused = resume_pool_qualification(
                    &first.configuration.run_id,
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                assert_eq!(paused.status, PoolRunStatus::Paused);
                assert!(matches!(
                    paused.pause,
                    Some(PoolPauseReason::SpendingLimit {
                        spent_millionths_of_dollar: 1,
                        limit_millionths_of_dollar: 1
                    })
                ));
                assert_eq!(runtime.executed_models.len(), calls);
            }

            #[test]
            fn quota_resume_reuses_the_preallocated_child_and_keeps_progress() {
                let mut runtime = FakeRuntime::new();
                runtime.quota_grades = 1;
                let mut progress = FakeProgress::default();
                let paused =
                    start_pool_qualification(request(vec![Tier::T2]), &mut runtime, &mut progress)
                        .unwrap();
                assert_eq!(paused.status, PoolRunStatus::Paused);
                let next_sequence = runtime.next_sequence;

                let resumed = resume_pool_qualification(
                    &paused.configuration.run_id,
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                assert_eq!(resumed.child_runs[0].status, PoolChildStatus::Completed);
                assert_eq!(runtime.next_sequence, next_sequence);
                assert_eq!(runtime.started, vec!["child-0"]);
                assert_eq!(resumed.pools[0].calibration.len(), 1);
            }

            fn run_to_review(
                runtime: &mut FakeRuntime,
                tiers: Vec<Tier>,
                progress: &mut FakeProgress,
            ) -> PoolRunState {
                let mut state =
                    start_pool_qualification(request(tiers), runtime, progress).unwrap();
                while state.status == PoolRunStatus::Running {
                    state =
                        resume_pool_qualification(&state.configuration.run_id, runtime, progress)
                            .unwrap();
                }
                state
            }

            fn request(selected_tiers: Vec<Tier>) -> PoolQualifyRequest {
                PoolQualifyRequest {
                    plan_path: PathBuf::from("pool.json"),
                    artifact_roots: vec![PathBuf::from("exam")],
                    selected_tiers,
                    is_dry_run: false,
                }
            }

            struct FakeRuntime {
                artifact: ArtifactDefinition,
                plan: PoolPlan,
                state: RefCell<Option<PoolRunState>>,
                runs: BTreeMap<RunId, Vec<RunEvent>>,
                next_sequence: usize,
                started: Vec<String>,
                executed_models: Vec<ModelIdentity>,
                low_models: Vec<ModelIdentity>,
                failed_full_runs: BTreeSet<String>,
                quota_grades: usize,
                is_unpromoted_skipped_before_full_call: bool,
            }

            impl FakeRuntime {
                fn new() -> Self {
                    Self {
                        artifact: artifact(),
                        plan: plan(),
                        state: RefCell::new(None),
                        runs: BTreeMap::new(),
                        next_sequence: 0,
                        started: Vec::new(),
                        executed_models: Vec::new(),
                        low_models: Vec::new(),
                        failed_full_runs: BTreeSet::new(),
                        quota_grades: 0,
                        is_unpromoted_skipped_before_full_call: false,
                    }
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

                fn exact_candidate(
                    &self,
                    requested: &ModelIdentity,
                ) -> Result<ModelIdentity, SkillEvalError> {
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
                    unreachable!()
                }

                fn pool_judge(
                    &self,
                    _candidate: &ModelIdentity,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    Ok(judge())
                }
            }

            impl HarnessResolver for FakeRuntime {
                fn identity(
                    &self,
                    artifact: &ArtifactDefinition,
                    _execution: &ExecutionDefinition,
                ) -> Result<HarnessIdentity, SkillEvalError> {
                    Ok(HarnessIdentity {
                        runner_version: "runner-1".to_owned(),
                        pi_version: "pi-1".to_owned(),
                        artifact_revision: artifact.revision.clone(),
                        tool_policy_digest: "tools-1".to_owned(),
                    })
                }
            }

            impl RunIdSource for FakeRuntime {
                fn next(&mut self) -> Result<RunId, SkillEvalError> {
                    let run_id = RunId(format!("child-{}", self.next_sequence));
                    self.next_sequence += 1;
                    Ok(run_id)
                }
            }

            impl PoolRunIdSource for FakeRuntime {
                fn next_pool(&mut self) -> Result<PoolRunId, SkillEvalError> {
                    Ok(PoolRunId("pool-1".to_owned()))
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
                ) -> Result<CandidateArtifact, SkillEvalError> {
                    self.executed_models.push(model.clone());
                    if run_id.0.ends_with('1') || run_id.0.ends_with('3') {
                        self.is_unpromoted_skipped_before_full_call |=
                            self.state.borrow().as_ref().is_some_and(|state| {
                                state.child_runs.iter().any(|child| {
                                    child.tier == key.tier
                                        && child.entrant_index == 2
                                        && child.status == PoolChildStatus::Skipped
                                })
                            });
                    }
                    let cost = model
                        .model
                        .rsplit('-')
                        .next()
                        .and_then(|value| value.parse::<u64>().ok())
                        .unwrap_or(0)
                        + 1;
                    Ok(CandidateArtifact {
                        key: key.clone(),
                        model: model.clone(),
                        harness: harness.clone(),
                        artifact_path: PathBuf::from(&run_id.0).join("artifact.txt"),
                        transcript_path: PathBuf::from(&run_id.0).join("transcript.jsonl"),
                        usage: usage(cost),
                    })
                }
            }

            impl Verifier for FakeRuntime {
                fn verify(
                    &mut self,
                    _case: &CaseDefinition,
                    _candidate: &CandidateArtifact,
                ) -> Result<Vec<CheckResult>, SkillEvalError> {
                    Ok(Vec::new())
                }
            }

            impl Judge for FakeRuntime {
                fn grade(
                    &mut self,
                    model: &ModelIdentity,
                    input: &JudgeInput,
                ) -> Result<JudgeResult, SkillEvalError> {
                    if self.quota_grades > 0 {
                        self.quota_grades -= 1;
                        return Err(SkillEvalError::Quota {
                            model: model.clone(),
                            reset_at: Some(now()),
                        });
                    }
                    let run_id = input
                        .candidate
                        .artifact_path
                        .components()
                        .next()
                        .and_then(|part| part.as_os_str().to_str())
                        .unwrap_or_default();
                    let is_low = self.low_models.contains(&input.candidate.model)
                        || self.failed_full_runs.contains(run_id);
                    Ok(JudgeResult {
                        verdict: TrialVerdict {
                            score: if is_low { 2 } else { 9 },
                            is_catastrophic: false,
                            failure_mode: None,
                            checks: Vec::new(),
                        },
                        model: model.clone(),
                        usage: usage(0),
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
                fn append(
                    &mut self,
                    run_id: &RunId,
                    event: &RunEvent,
                ) -> Result<(), SkillEvalError> {
                    if matches!(event, RunEvent::RunStarted { .. }) {
                        self.started.push(run_id.0.clone());
                    }
                    self.runs
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
                    for event in self.runs.get(run_id).ok_or_else(|| {
                        SkillEvalError::NotFound(format!("missing run {}", run_id.0))
                    })? {
                        visitor(event.clone())?;
                    }
                    Ok(())
                }

                fn find_trial(
                    &self,
                    _selector: &TrialSelector,
                ) -> Result<TrialRecord, SkillEvalError> {
                    unreachable!()
                }
            }

            impl PoolPlanSource for FakeRuntime {
                fn load_pool_plan(&self, _path: &Path) -> Result<PoolPlan, SkillEvalError> {
                    Ok(self.plan.clone())
                }

                fn validate_pool_plan_freshness(
                    &self,
                    _plan: &PoolPlan,
                    _now: &Timestamp,
                ) -> Result<(), SkillEvalError> {
                    Ok(())
                }
            }

            impl PoolStore for FakeRuntime {
                fn create_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
                    *self.state.borrow_mut() = Some(state.clone());
                    Ok(())
                }

                fn load_pool(&self, _run_id: &PoolRunId) -> Result<PoolRunState, SkillEvalError> {
                    self.state
                        .borrow()
                        .clone()
                        .ok_or_else(|| SkillEvalError::NotFound("missing pool".to_owned()))
                }

                fn save_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
                    *self.state.borrow_mut() = Some(state.clone());
                    Ok(())
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

            impl QualificationRuntime for FakeRuntime {}
            impl PoolRuntime for FakeRuntime {}

            #[derive(Default)]
            struct FakeProgress {
                states: Vec<PoolRunState>,
            }

            impl FakeProgress {
                fn is_monotonic(&self) -> bool {
                    self.states.windows(2).all(|states| {
                        states[1].spent_millionths_of_dollar >= states[0].spent_millionths_of_dollar
                            && states[1].pools.len() >= states[0].pools.len()
                    })
                }
            }

            impl PoolProgressSink for FakeProgress {
                fn emit_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
                    self.states.push(state.clone());
                    Ok(())
                }
            }

            fn plan() -> PoolPlan {
                let entrants = [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5]
                    .into_iter()
                    .map(|tier| {
                        (
                            tier,
                            (0..3)
                                .map(|index| PoolEntrant {
                                    model: model(tier, index),
                                    catalog_observed_at: now(),
                                })
                                .collect(),
                        )
                    })
                    .collect();
                PoolPlan {
                    entrants,
                    control: ModelIdentity {
                        tier: Tier::T1,
                        provider: "free".to_owned(),
                        model: "control".to_owned(),
                        thinking: "low".to_owned(),
                    },
                    policy: PoolPolicy {
                        calibration_repeats_per_case: 1,
                        qualification_repeats_per_case: 3,
                        promotion_count: 2,
                        minimum_score: 7,
                        minimum_reliability_basis_points: 10_000,
                        maximum_catalog_age_seconds: 3_600,
                        spending_limit_millionths_of_dollar: 1_000_000,
                        is_provider_limit_enforced: true,
                    },
                }
            }

            fn artifact() -> ArtifactDefinition {
                ArtifactDefinition {
                    name: ArtifactName("calibration".to_owned()),
                    kind: ArtifactKind::Skill,
                    root: PathBuf::from("exam"),
                    revision: "revision-1".to_owned(),
                    required_destinations: vec![TierDestination::SkillMinimum],
                    current_tiers: Vec::new(),
                    cases: vec![CaseDefinition {
                        id: CaseId("case-1".to_owned()),
                        input: "input".to_owned(),
                        expect: "expect".to_owned(),
                        source: "fixture".to_owned(),
                        is_holdout: false,
                        support_files: Vec::new(),
                        execution: ExecutionDefinition {
                            drive: CaseDrive::Response,
                            allowed_tools: Vec::new(),
                            timeout_seconds: 10,
                        },
                    }],
                }
            }

            fn model(tier: Tier, index: usize) -> ModelIdentity {
                ModelIdentity {
                    tier,
                    provider: "provider".to_owned(),
                    model: format!("model-{tier:?}-{index}"),
                    thinking: "medium".to_owned(),
                }
            }

            fn judge() -> ModelIdentity {
                ModelIdentity {
                    tier: Tier::T5,
                    provider: "judge".to_owned(),
                    model: "judge-model".to_owned(),
                    thinking: "high".to_owned(),
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
                    elapsed_milliseconds: cost,
                    cost_millionths_of_dollar: cost,
                }
            }

            fn now() -> Timestamp {
                Timestamp("2026-08-25T12:00:00-0400".to_owned())
            }
        }
    };
}
