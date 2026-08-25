#[macro_export]
macro_rules! pool_start_tests {
    () => {
        mod pool_start {
            use std::cell::{Cell, RefCell};
            use std::collections::BTreeMap;
            use std::path::{Path, PathBuf};
            use std::rc::Rc;

            use $crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, ArtifactStatus, CandidateArtifact,
                CaseDefinition, CaseDrive, CaseId, CheckResult, ExecutionDefinition,
                HarnessIdentity, JudgeInput, JudgeResult, ModelIdentity, PoolChildStatus,
                PoolEntrant, PoolPlan, PoolPolicy, PoolQualifyRequest, PoolRunId, PoolRunState,
                PoolRunStatus, PromptJudgeRequest, PromptJudgeResult, QualificationPolicy,
                QualificationPurpose, QualifyRequest, RunEvent, RunId, RunStatus,
                SkillEvalError, Tier, TierAssignment, TierDestination, Timestamp, TrialKey,
                TrialRecord, TrialSelector, TrialUsage, TrialVerdict,
            };
            use $crate::ports::{
                ArtifactSource, CandidateRunner, Clock, HarnessResolver, Judge, ModelResolver,
                PoolPlanSource, PoolProgressSink, PoolRunIdSource, PoolRuntime, PoolStore,
                ProgressSink, QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
            };

            use super::{
                build_report, start_pool_qualification, start_qualification,
                start_qualification_with_run_id,
            };

            #[test]
            fn dry_run_persists_all_selected_child_ids_before_progress_without_model_calls() {
                let mut runtime = FakeRuntime::new();
                let persisted = runtime.persisted.clone();
                let mut progress = FakePoolProgress::new(persisted);

                let state = start_pool_qualification(
                    pool_request(vec![Tier::T2, Tier::T5], true),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                assert_eq!(state.status, PoolRunStatus::Pending);
                assert_eq!(state.selected_tiers, vec![Tier::T2, Tier::T5]);
                assert_eq!(state.configuration.artifacts.len(), 1);
                assert_eq!(state.configuration.artifacts[0].revision, "exam-revision");
                assert_eq!(state.configuration.artifacts[0].root, PathBuf::from("exam"));
                assert_eq!(state.configuration.artifacts[0].cases.len(), 2);
                assert_eq!(
                    state.configuration.artifacts[0]
                        .cases
                        .iter()
                        .filter(|case| !case.is_holdout)
                        .count(),
                    1
                );
                assert_eq!(state.child_runs.len(), 12);
                assert!(state
                    .child_runs
                    .iter()
                    .all(|child| child.status == PoolChildStatus::Pending));
                let mut ids = state
                    .child_runs
                    .iter()
                    .map(|child| child.run_id.clone())
                    .collect::<Vec<_>>();
                ids.sort();
                ids.dedup();
                assert_eq!(ids.len(), 12);
                assert_eq!(runtime.model_calls, 0);
                assert_eq!(runtime.exact_calls.get(), 0);
                assert!(progress.is_persisted_before_emit);
                assert_eq!(progress.states, vec![state]);
                assert_eq!(
                    runtime.operations.borrow()[..3],
                    ["plan", "clock", "freshness"]
                );
            }

            #[test]
            fn live_start_runs_only_first_exact_calibration_child_to_terminal_report() {
                let mut runtime = FakeRuntime::new();
                let persisted = runtime.persisted.clone();
                let mut progress = FakePoolProgress::new(persisted);

                let state = start_pool_qualification(
                    pool_request(vec![Tier::T2], false),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                assert_eq!(state.status, PoolRunStatus::Running);
                assert_eq!(state.child_runs.len(), 6);
                assert_eq!(state.child_runs[0].status, PoolChildStatus::Completed);
                assert!(state.child_runs[1..]
                    .iter()
                    .all(|child| child.status == PoolChildStatus::Pending));
                assert_eq!(state.spent_millionths_of_dollar, 7);
                assert_eq!(runtime.model_calls, 2);
                assert!(runtime.is_preallocated_at_first_model_call);
                assert!(progress.is_persisted_before_emit);

                let child_id = state.child_runs[0].run_id.clone();
                let report = build_report(&child_id, &runtime).unwrap();
                assert_eq!(report.status, RunStatus::Completed);
                assert_eq!(report.artifacts[0].status, ArtifactStatus::PoolCompleted);
                assert!(report.artifacts[0].reference.is_none());
                assert!(report.artifacts[0].tiers.is_empty());
                assert!(report.artifacts[0].boundary.is_none());
                assert!(report.artifacts[0].decision.is_none());

                let events = &runtime.runs[&child_id];
                let routes = events
                    .iter()
                    .filter_map(|event| match event {
                        RunEvent::TrialStarted { models, .. } => Some(models),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(routes.len(), 1);
                assert_eq!(routes[0], &vec![pool_model(Tier::T2, 0)]);
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| matches!(event, RunEvent::TrialCompleted { .. }))
                        .count(),
                    1
                );
                assert!(matches!(events.last(), Some(RunEvent::PoolChildCompleted { .. })));
                assert!(!events.iter().any(|event| matches!(
                    event,
                    RunEvent::TierEvaluated { .. }
                        | RunEvent::BoundaryFound { .. }
                        | RunEvent::DecisionRecorded { .. }
                )));
                assert_eq!(runtime.execute_models, vec![pool_model(Tier::T2, 0)]);
                assert_eq!(runtime.started_run_ids, vec![child_id]);
            }

            #[test]
            fn t5_exact_child_uses_a_distinct_t5_pool_judge() {
                let mut runtime = FakeRuntime::new();
                let candidate = pool_model(Tier::T5, 0);
                let mut progress = FakeProgress;

                let report = start_qualification_with_run_id(
                    RunId("t5-child".to_owned()),
                    Some(candidate.clone()),
                    child_request(Tier::T5),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                assert_eq!(report.status, RunStatus::Completed);
                let completed = runtime.runs[&report.run_id]
                    .iter()
                    .find_map(|event| match event {
                        RunEvent::TrialCompleted { record, .. } => Some(record),
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(completed.model, candidate);
                assert_eq!(completed.judge_model.tier, Tier::T5);
                assert_ne!(
                    (completed.model.provider.as_str(), completed.model.model.as_str()),
                    (
                        completed.judge_model.provider.as_str(),
                        completed.judge_model.model.as_str()
                    )
                );
            }

            #[test]
            fn ordinary_start_keeps_configured_reference_and_fallback_route() {
                let mut runtime = FakeRuntime::new();
                let mut progress = FakeProgress;
                let request = QualifyRequest {
                    artifact_roots: vec![PathBuf::from("exam")],
                    change: None,
                    policy: QualificationPolicy {
                        purpose: QualificationPurpose::Artifact,
                        candidate_tiers: vec![Tier::T1],
                        reference_tier: Tier::T4,
                        judge_tier: Tier::T5,
                        repeats_per_case: 1,
                        minimum_score: 7,
                        noninferiority_margin: 0.1,
                        confidence_level: 0.95,
                    },
                    is_dry_run: false,
                };

                let report = start_qualification(request, &mut runtime, &mut progress).unwrap();

                assert_eq!(report.status, RunStatus::AwaitingDecision);
                assert_eq!(runtime.exact_calls.get(), 0);
                assert!(runtime.candidate_calls.get() >= 2);
                assert!(runtime.runs[&report.run_id].iter().any(|event| matches!(
                    event,
                    RunEvent::TrialStarted { models, .. } if models.len() == 2
                )));
            }

            #[test]
            fn invalid_exact_inputs_and_stale_or_uncapped_plans_fail_before_launch() {
                let mut runtime = FakeRuntime::new();
                let mut progress = FakeProgress;
                let artifact_request = QualifyRequest {
                    artifact_roots: vec![PathBuf::from("exam")],
                    change: None,
                    policy: QualificationPolicy {
                        purpose: QualificationPurpose::Artifact,
                        candidate_tiers: vec![Tier::T1],
                        reference_tier: Tier::T4,
                        judge_tier: Tier::T5,
                        repeats_per_case: 1,
                        minimum_score: 7,
                        noninferiority_margin: 0.1,
                        confidence_level: 0.95,
                    },
                    is_dry_run: false,
                };
                assert!(start_qualification_with_run_id(
                    RunId("bad-artifact".to_owned()),
                    Some(pool_model(Tier::T1, 0)),
                    artifact_request,
                    &mut runtime,
                    &mut progress,
                )
                .is_err());
                assert!(start_qualification_with_run_id(
                    RunId("missing".to_owned()),
                    None,
                    child_request(Tier::T2),
                    &mut runtime,
                    &mut progress,
                )
                .is_err());
                assert!(start_qualification_with_run_id(
                    RunId("mismatch".to_owned()),
                    Some(pool_model(Tier::T3, 0)),
                    child_request(Tier::T2),
                    &mut runtime,
                    &mut progress,
                )
                .is_err());
                assert_eq!(runtime.model_calls, 0);
                assert!(runtime.runs.is_empty());

                let mut stale = FakeRuntime::new();
                stale.freshness_error = true;
                let persisted = stale.persisted.clone();
                let mut pool_progress = FakePoolProgress::new(persisted);
                assert!(start_pool_qualification(
                    pool_request(vec![Tier::T2], false),
                    &mut stale,
                    &mut pool_progress,
                )
                .is_err());
                assert!(stale.pool_states.is_empty());
                assert_eq!(stale.model_calls, 0);

                let mut uncapped = FakeRuntime::new();
                uncapped.plan.policy.spending_limit_millionths_of_dollar = 0;
                let persisted = uncapped.persisted.clone();
                let mut pool_progress = FakePoolProgress::new(persisted);
                assert!(start_pool_qualification(
                    pool_request(vec![Tier::T2], false),
                    &mut uncapped,
                    &mut pool_progress,
                )
                .is_err());
                assert!(uncapped.pool_states.is_empty());
                assert_eq!(uncapped.model_calls, 0);
            }

            #[test]
            fn quota_pauses_child_then_pool_through_saved_legal_states() {
                let mut runtime = FakeRuntime::new();
                runtime.is_quota = true;
                let persisted = runtime.persisted.clone();
                let mut progress = FakePoolProgress::new(persisted);

                let state = start_pool_qualification(
                    pool_request(vec![Tier::T2], false),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                assert_eq!(state.status, PoolRunStatus::Paused);
                assert_eq!(state.child_runs[0].status, PoolChildStatus::Paused);
                assert!(state.pause.is_some());
                let statuses = runtime
                    .pool_states
                    .iter()
                    .map(|state| (state.status, state.child_runs[0].status))
                    .collect::<Vec<_>>();
                assert_eq!(
                    statuses,
                    vec![
                        (PoolRunStatus::Pending, PoolChildStatus::Pending),
                        (PoolRunStatus::Running, PoolChildStatus::Pending),
                        (PoolRunStatus::Running, PoolChildStatus::Running),
                        (PoolRunStatus::Running, PoolChildStatus::Paused),
                        (PoolRunStatus::Paused, PoolChildStatus::Paused),
                    ]
                );
                assert!(progress.is_persisted_before_emit);
            }

            fn pool_request(selected_tiers: Vec<Tier>, is_dry_run: bool) -> PoolQualifyRequest {
                PoolQualifyRequest {
                    plan_path: PathBuf::from("plan.json"),
                    artifact_roots: vec![PathBuf::from("exam")],
                    selected_tiers,
                    is_dry_run,
                }
            }

            fn child_request(tier: Tier) -> QualifyRequest {
                QualifyRequest {
                    artifact_roots: vec![PathBuf::from("exam")],
                    change: None,
                    policy: QualificationPolicy {
                        purpose: QualificationPurpose::ModelPool,
                        candidate_tiers: vec![tier],
                        reference_tier: if tier == Tier::T1 { Tier::T2 } else { Tier::T1 },
                        judge_tier: Tier::T5,
                        repeats_per_case: 1,
                        minimum_score: 7,
                        noninferiority_margin: 0.0,
                        confidence_level: 0.95,
                    },
                    is_dry_run: false,
                }
            }

            fn pool_model(tier: Tier, index: usize) -> ModelIdentity {
                ModelIdentity {
                    tier,
                    provider: "pool".to_owned(),
                    model: format!("model-{tier:?}-{index}"),
                    thinking: "medium".to_owned(),
                }
            }

            fn route_model(tier: Tier, index: usize) -> ModelIdentity {
                ModelIdentity {
                    tier,
                    provider: "configured".to_owned(),
                    model: format!("route-{tier:?}-{index}"),
                    thinking: "medium".to_owned(),
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

            struct FakeRuntime {
                plan: PoolPlan,
                runs: BTreeMap<RunId, Vec<RunEvent>>,
                pool_states: Vec<PoolRunState>,
                persisted: Rc<RefCell<Option<PoolRunState>>>,
                operations: RefCell<Vec<&'static str>>,
                next_run_id: u32,
                exact_calls: Cell<usize>,
                candidate_calls: Cell<usize>,
                model_calls: usize,
                execute_models: Vec<ModelIdentity>,
                started_run_ids: Vec<RunId>,
                is_preallocated_at_first_model_call: bool,
                freshness_error: bool,
                is_quota: bool,
            }

            impl FakeRuntime {
                fn new() -> Self {
                    let entrants = [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5]
                        .into_iter()
                        .map(|tier| {
                            (
                                tier,
                                (0..3)
                                    // TODO(AGNT-0032.T103): Add neutral one-level thinking lists to start fixtures.
                                    .map(|index| PoolEntrant {
                                        model: pool_model(tier, index),
                                        catalog_observed_at: Timestamp(
                                            "2026-08-24T12:00:00-0400".to_owned(),
                                        ),
                                    })
                                    .collect(),
                            )
                        })
                        .collect();
                    Self {
                        plan: PoolPlan {
                            entrants,
                            control: ModelIdentity {
                                tier: Tier::T1,
                                provider: "control".to_owned(),
                                model: "free".to_owned(),
                                thinking: "off".to_owned(),
                            },
                            policy: PoolPolicy {
                                calibration_repeats_per_case: 1,
                                qualification_repeats_per_case: 3,
                                promotion_count: 2,
                                minimum_score: 7,
                                minimum_reliability_basis_points: 9_000,
                                maximum_catalog_age_seconds: 3_600,
                                spending_limit_millionths_of_dollar: 10_000_000,
                                is_provider_limit_enforced: true,
                            },
                        },
                        runs: BTreeMap::new(),
                        pool_states: Vec::new(),
                        persisted: Rc::new(RefCell::new(None)),
                        operations: RefCell::new(Vec::new()),
                        next_run_id: 0,
                        exact_calls: Cell::new(0),
                        candidate_calls: Cell::new(0),
                        model_calls: 0,
                        execute_models: Vec::new(),
                        started_run_ids: Vec::new(),
                        is_preallocated_at_first_model_call: false,
                        freshness_error: false,
                        is_quota: false,
                    }
                }
            }

            impl ArtifactSource for FakeRuntime {
                fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
                    Ok(ArtifactDefinition {
                        name: ArtifactName("calibration".to_owned()),
                        kind: ArtifactKind::Skill,
                        root: root.to_path_buf(),
                        revision: "exam-revision".to_owned(),
                        required_destinations: vec![TierDestination::SkillMinimum],
                        current_tiers: Vec::new(),
                        cases: vec![
                            CaseDefinition {
                                id: CaseId("fixed".to_owned()),
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
                            },
                            CaseDefinition {
                                id: CaseId("holdout".to_owned()),
                                input: "hidden".to_owned(),
                                expect: "hidden".to_owned(),
                                source: "fixture".to_owned(),
                                is_holdout: true,
                                support_files: Vec::new(),
                                execution: ExecutionDefinition {
                                    drive: CaseDrive::Response,
                                    allowed_tools: Vec::new(),
                                    timeout_seconds: 10,
                                },
                            },
                        ],
                    })
                }
            }

            impl ModelResolver for FakeRuntime {
                fn candidates(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    self.candidate_calls.set(self.candidate_calls.get() + 1);
                    Ok(vec![route_model(tier, 0), route_model(tier, 1)])
                }

                fn exact_candidate(
                    &self,
                    requested: &ModelIdentity,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    self.exact_calls.set(self.exact_calls.get() + 1);
                    Ok(requested.clone())
                }

                fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
                    Ok(Tier::T5)
                }

                fn judge(
                    &self,
                    judge_tier: Tier,
                    _candidate: Option<&ModelIdentity>,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    Ok(route_model(judge_tier, 0))
                }

                fn pool_judge(
                    &self,
                    candidate: &ModelIdentity,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    Ok(ModelIdentity {
                        tier: Tier::T5,
                        provider: "judge".to_owned(),
                        model: format!("external-for-{}", candidate.model),
                        thinking: "high".to_owned(),
                    })
                }
            }

            impl HarnessResolver for FakeRuntime {
                fn identity(
                    &self,
                    artifact: &ArtifactDefinition,
                    _execution: &ExecutionDefinition,
                ) -> Result<HarnessIdentity, SkillEvalError> {
                    Ok(HarnessIdentity {
                        runner_version: "1".to_owned(),
                        pi_version: "1".to_owned(),
                        artifact_revision: artifact.revision.clone(),
                        tool_policy_digest: "fixed".to_owned(),
                    })
                }
            }

            impl RunIdSource for FakeRuntime {
                fn next(&mut self) -> Result<RunId, SkillEvalError> {
                    self.next_run_id += 1;
                    Ok(RunId(format!("child-{}", self.next_run_id)))
                }
            }

            impl PoolRunIdSource for FakeRuntime {
                fn next_pool(&mut self) -> Result<PoolRunId, SkillEvalError> {
                    self.operations.borrow_mut().push("pool-id");
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
                    self.model_calls += 1;
                    self.execute_models.push(model.clone());
                    self.started_run_ids.push(run_id.clone());
                    self.is_preallocated_at_first_model_call = self
                        .persisted
                        .borrow()
                        .as_ref()
                        .is_some_and(|state| state.child_runs.len() == state.selected_tiers.len() * 6);
                    if self.is_quota {
                        return Err(SkillEvalError::Quota {
                            model: model.clone(),
                            reset_at: Some(Timestamp("2026-08-24T13:00:00-0400".to_owned())),
                        });
                    }
                    Ok(CandidateArtifact {
                        key: key.clone(),
                        model: model.clone(),
                        harness: harness.clone(),
                        artifact_path: PathBuf::from("artifact.txt"),
                        transcript_path: PathBuf::from("transcript.jsonl"),
                        usage: usage(3),
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
                    self.model_calls += 1;
                    Ok(JudgeResult {
                        verdict: TrialVerdict {
                            score: 9,
                            is_catastrophic: false,
                            failure_mode: None,
                            checks: input.checks.clone(),
                        },
                        model: model.clone(),
                        usage: usage(4),
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
                        self.started_run_ids.retain(|current| current != run_id);
                    }
                    self.runs.entry(run_id.clone()).or_default().push(event.clone());
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
                    self.operations.borrow_mut().push("plan");
                    Ok(self.plan.clone())
                }

                fn validate_pool_plan_freshness(
                    &self,
                    _plan: &PoolPlan,
                    _now: &Timestamp,
                ) -> Result<(), SkillEvalError> {
                    self.operations.borrow_mut().push("freshness");
                    if self.freshness_error {
                        Err(SkillEvalError::InvalidConfiguration("stale plan".to_owned()))
                    } else {
                        Ok(())
                    }
                }
            }

            impl PoolStore for FakeRuntime {
                fn create_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
                    self.operations.borrow_mut().push("create");
                    self.pool_states.push(state.clone());
                    *self.persisted.borrow_mut() = Some(state.clone());
                    Ok(())
                }

                fn load_pool(&self, _run_id: &PoolRunId) -> Result<PoolRunState, SkillEvalError> {
                    self.persisted
                        .borrow()
                        .clone()
                        .ok_or_else(|| SkillEvalError::NotFound("pool".to_owned()))
                }

                fn save_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
                    self.operations.borrow_mut().push("save");
                    self.pool_states.push(state.clone());
                    *self.persisted.borrow_mut() = Some(state.clone());
                    Ok(())
                }
            }

            impl Clock for FakeRuntime {
                fn now(&self) -> Timestamp {
                    self.operations.borrow_mut().push("clock");
                    Timestamp("2026-08-24T12:00:00-0400".to_owned())
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

            struct FakeProgress;

            impl ProgressSink for FakeProgress {
                fn emit(&mut self, _event: &RunEvent) -> Result<(), SkillEvalError> {
                    Ok(())
                }
            }

            struct FakePoolProgress {
                persisted: Rc<RefCell<Option<PoolRunState>>>,
                states: Vec<PoolRunState>,
                is_persisted_before_emit: bool,
            }

            impl FakePoolProgress {
                fn new(persisted: Rc<RefCell<Option<PoolRunState>>>) -> Self {
                    Self {
                        persisted,
                        states: Vec::new(),
                        is_persisted_before_emit: true,
                    }
                }
            }

            impl PoolProgressSink for FakePoolProgress {
                fn emit_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
                    self.is_persisted_before_emit &= self.persisted.borrow().as_ref() == Some(state);
                    self.states.push(state.clone());
                    Ok(())
                }
            }
        }
    };
}
