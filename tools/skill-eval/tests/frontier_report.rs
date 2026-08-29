#[cfg(test)]
macro_rules! frontier_report_tests {
    () => {
        mod frontier_report_tests {
            use std::cell::Cell;
            use std::collections::BTreeMap;
            use std::path::{Path, PathBuf};

            use $crate::model::{
                ArtifactDefinition, ArtifactName, CandidateArtifact, CaseDefinition, CaseId,
                CheckResult, ExecutionDefinition, FrontierApplyReport, FrontierBaseline,
                FrontierBaselineChange, FrontierBaselineLedger, FrontierCellEvidence,
                FrontierCellStatus, FrontierConfidenceMethod, FrontierEntrant,
                FrontierEvidenceIdentity, FrontierInspection, FrontierModelProgress, FrontierPlan,
                FrontierPolicy, FrontierPoolMembership, FrontierRunConfiguration, FrontierRunId,
                FrontierRunState, FrontierRunStatus, FrontierScore, FrontierSuite,
                FrontierSuiteIdentity, FrontierTrialSelector, HarnessIdentity, JudgeInput,
                JudgeResult, ModelIdentity, PromptJudgeRequest, PromptJudgeResult, RunEvent, RunId,
                SkillEvalError, T1ScreenSnapshotIdentity, Tier, TierAssignment, Timestamp,
                TrialKey, TrialRecord, TrialSelector, TrialUsage, TrialVerdict,
            };
            use $crate::ports::{
                ArtifactSource, CandidateRunner, Clock, FrontierRuntime, HarnessResolver, Judge,
                ModelResolver, QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
            };

            use super::frontier_report::{derive_frontier_report, inspection_matches};
            use super::{inspect_frontier, validate_frontier_inspection_selector};

            #[test]
            fn report_derivation_builds_one_frozen_matrix_with_metrics_and_membership() {
                let state = state();
                let progress = vec![progress()];
                let evidence = vec![cell(Tier::T1, "low", FrontierCellStatus::Passed, 9_100)];

                let first = derive_frontier_report(&state, &progress, &evidence, None).unwrap();
                let second = derive_frontier_report(&state, &progress, &evidence, None).unwrap();

                assert_eq!(first, second);
                assert_eq!(first.models.len(), 1);
                let model = &first.models[0];
                assert_eq!(model.cells.len(), 1);
                assert_eq!(model.cells[0].model.thinking, "low");
                assert_eq!(model.cells[0].model.tier, Tier::T1);
                assert_eq!(model.cells[0].status, FrontierCellStatus::Passed);
                assert_eq!(
                    model.cells[0]
                        .score
                        .as_ref()
                        .unwrap()
                        .weighted_pass_basis_points,
                    9_100
                );
                assert_eq!(
                    model.cells[0]
                        .score
                        .as_ref()
                        .unwrap()
                        .lower_bound_basis_points,
                    8_800
                );
                assert_eq!(
                    model.cells[0]
                        .score
                        .as_ref()
                        .unwrap()
                        .critical_passed_trials,
                    2
                );
                assert_eq!(model.cells[0].total_usage.input_tokens, 11);
                assert_eq!(model.cells[0].total_usage.turns, 2);
                assert_eq!(model.cells[0].total_usage.tool_calls, 3);
                assert_eq!(model.cells[0].total_usage.elapsed_milliseconds, 17);
                assert_eq!(model.cells[0].total_usage.cost_millionths_of_dollar, 19);
                assert_eq!(model.selected_routes, vec![route(Tier::T1, "low")]);
                assert_eq!(model.highest_passing_tier, Some(Tier::T1));
                assert_eq!(model.pool_memberships[&Tier::T1].rank, 1);
                assert!(model.pool_memberships[&Tier::T1].is_active);
                assert_eq!(model.baseline_change, FrontierBaselineChange::NotCompared);
            }

            #[test]
            fn report_derivation_keeps_quota_skipped_evidence() {
                let state = state();
                let progress = FrontierModelProgress {
                    provider: "anthropic".to_owned(),
                    model: "alpha".to_owned(),
                    entry_tier: Tier::T1,
                    selected_routes: Vec::new(),
                    next_tier: None,
                    next_thinking_index: None,
                    is_exhausted: true,
                };
                let skipped = FrontierCellEvidence {
                    model: route(Tier::T1, "low"),
                    status: FrontierCellStatus::Skipped,
                    set_aside_reason: None,
                    completed_trials: 0,
                    expected_trials: 0,
                    failed_trials: 0,
                    score: None,
                    total_usage: zero_usage(),
                };

                let report = derive_frontier_report(&state, &[progress], &[skipped], None).unwrap();

                assert_eq!(report.models[0].cells.len(), 1);
                assert_eq!(
                    report.models[0].cells[0].status,
                    FrontierCellStatus::Skipped
                );
                assert!(report.models[0].selected_routes.is_empty());
                assert_eq!(report.models[0].highest_passing_tier, None);
            }

            #[test]
            fn report_derivation_marks_baseline_better_worse_unchanged_and_new() {
                let state = state();
                let evidence = vec![cell(Tier::T1, "low", FrontierCellStatus::Passed, 9_100)];
                let unchanged = baseline(vec![membership(Tier::T1, "low")]);
                assert_eq!(
                    derive_frontier_report(&state, &[progress()], &evidence, Some(&unchanged))
                        .unwrap()
                        .models[0]
                        .baseline_change,
                    FrontierBaselineChange::Unchanged
                );

                let stronger_incumbent = baseline(vec![membership(Tier::T2, "high")]);
                assert_eq!(
                    derive_frontier_report(
                        &state,
                        &[progress()],
                        &evidence,
                        Some(&stronger_incumbent),
                    )
                    .unwrap()
                    .models[0]
                        .baseline_change,
                    FrontierBaselineChange::Worse
                );

                let weaker_incumbent = baseline(vec![membership(Tier::T1, "high")]);
                assert_eq!(
                    derive_frontier_report(
                        &state,
                        &[progress()],
                        &evidence,
                        Some(&weaker_incumbent),
                    )
                    .unwrap()
                    .models[0]
                        .baseline_change,
                    FrontierBaselineChange::Better
                );

                assert_eq!(
                    derive_frontier_report(
                        &state,
                        &[progress()],
                        &evidence,
                        Some(&baseline(Vec::new())),
                    )
                    .unwrap()
                    .models[0]
                        .baseline_change,
                    FrontierBaselineChange::New
                );
            }

            #[test]
            fn report_derivation_rejects_duplicate_foreign_incomplete_and_drifted_evidence() {
                let state = state();
                let passing = cell(Tier::T1, "low", FrontierCellStatus::Passed, 9_100);
                assert!(
                    derive_frontier_report(
                        &state,
                        &[progress()],
                        &[passing.clone(), passing.clone()],
                        None,
                    )
                    .is_err()
                );

                let mut foreign = passing.clone();
                foreign.model.model = "foreign".to_owned();
                assert!(
                    derive_frontier_report(
                        &state,
                        &[progress()],
                        &[passing.clone(), foreign],
                        None,
                    )
                    .is_err()
                );

                let mut incomplete = passing.clone();
                incomplete.score = None;
                assert!(
                    derive_frontier_report(&state, &[progress()], &[incomplete], None,).is_err()
                );

                let mut drifted = progress();
                drifted.selected_routes.clear();
                assert!(derive_frontier_report(&state, &[drifted], &[passing], None).is_err());
            }

            #[test]
            fn exact_inspection_calls_the_runtime_once_and_rejects_mismatch_without_other_calls() {
                let trial_selector = selector();
                let runtime = InspectionRuntime::new(FrontierInspection::Trial { trial: trial() });
                let result = inspect_frontier(&trial_selector, &runtime).unwrap();
                assert!(matches!(result, FrontierInspection::Trial { .. }));
                assert_eq!(runtime.inspection_calls.get(), 1);
                assert_eq!(runtime.load_calls.get(), 1);

                let mut wrong = trial();
                wrong.key.case = CaseId("other".to_owned());
                let runtime = InspectionRuntime::new(FrontierInspection::Trial { trial: wrong });
                assert!(inspect_frontier(&trial_selector, &runtime).is_err());
                assert_eq!(runtime.inspection_calls.get(), 1);

                let mut unsafe_selector = selector();
                unsafe_selector.case = CaseId("../case".to_owned());
                let runtime = InspectionRuntime::new(FrontierInspection::Trial { trial: trial() });
                assert!(inspect_frontier(&unsafe_selector, &runtime).is_err());
                assert_eq!(runtime.load_calls.get(), 0);
                assert_eq!(runtime.inspection_calls.get(), 0);
                assert!(validate_frontier_inspection_selector(&unsafe_selector).is_err());
            }

            #[test]
            fn inspection_identity_accepts_exact_infrastructure_and_rejects_wrong_type_identity() {
                let selector = selector();
                let event = $crate::model::FrontierInfrastructureEvent {
                    model: route(Tier::T1, "low"),
                    artifact: selector.artifact.clone(),
                    case: selector.case.clone(),
                    attempt: selector.attempt,
                    infrastructure_attempt: 1,
                    failure_stage: None,
                    charged_millionths_of_dollar: 0,
                    message: "temporary".to_owned(),
                    occurred_at: timestamp(),
                };
                assert!(inspection_matches(
                    &selector,
                    &FrontierInspection::Infrastructure {
                        event: event.clone()
                    }
                ));
                let mut wrong = event;
                wrong.infrastructure_attempt = 0;
                assert!(!inspection_matches(
                    &selector,
                    &FrontierInspection::Infrastructure { event: wrong }
                ));
            }

            struct InspectionRuntime {
                state: FrontierRunState,
                result: FrontierInspection,
                load_calls: Cell<u32>,
                inspection_calls: Cell<u32>,
            }

            impl InspectionRuntime {
                fn new(result: FrontierInspection) -> Self {
                    Self {
                        state: state(),
                        result,
                        load_calls: Cell::new(0),
                        inspection_calls: Cell::new(0),
                    }
                }
            }

            impl QualificationRuntime for InspectionRuntime {}

            impl FrontierRuntime for InspectionRuntime {
                fn load_frontier_plan(
                    &self,
                    _path: &Path,
                ) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError> {
                    panic!("inspection loaded the plan")
                }

                fn next_frontier_run_id(&mut self) -> Result<FrontierRunId, SkillEvalError> {
                    panic!("inspection allocated a run")
                }

                fn create_frontier(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    panic!("inspection created a run")
                }

                fn load_frontier(
                    &self,
                    run_id: &FrontierRunId,
                ) -> Result<FrontierRunState, SkillEvalError> {
                    self.load_calls.set(self.load_calls.get() + 1);
                    assert_eq!(run_id, &self.state.configuration.run_id);
                    Ok(self.state.clone())
                }

                fn save_frontier(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    panic!("inspection saved a run")
                }

                fn save_frontier_trial(
                    &mut self,
                    _run_id: &FrontierRunId,
                    _trial: &TrialRecord,
                ) -> Result<(), SkillEvalError> {
                    panic!("inspection saved a trial")
                }

                fn inspect_frontier(
                    &self,
                    _selector: &FrontierTrialSelector,
                ) -> Result<FrontierInspection, SkillEvalError> {
                    self.inspection_calls.set(self.inspection_calls.get() + 1);
                    Ok(self.result.clone())
                }

                fn load_frontier_baselines(
                    &self,
                    _path: &Path,
                ) -> Result<FrontierBaselineLedger, SkillEvalError> {
                    panic!("inspection loaded baselines")
                }

                fn accept_frontier_baseline(
                    &mut self,
                    _state: &FrontierRunState,
                    _path: &Path,
                    _ledger: &FrontierBaselineLedger,
                ) -> Result<(), SkillEvalError> {
                    panic!("inspection accepted a baseline")
                }

                fn apply_frontier_routes(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<FrontierApplyReport, SkillEvalError> {
                    panic!("inspection applied routes")
                }
            }

            impl ArtifactSource for InspectionRuntime {
                fn load(&self, _root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
                    panic!("inspection loaded an artifact")
                }
            }

            impl ModelResolver for InspectionRuntime {
                fn candidates(&self, _tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    panic!("inspection resolved candidates")
                }

                fn qualification_routes(
                    &self,
                    _tier: Tier,
                ) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    panic!("inspection resolved routes")
                }

                fn exact_candidate(
                    &self,
                    _requested: &ModelIdentity,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    panic!("inspection resolved an exact model")
                }

                fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
                    panic!("inspection resolved the judge tier")
                }

                fn judge(
                    &self,
                    _judge_tier: Tier,
                    _candidate: Option<&ModelIdentity>,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    panic!("inspection resolved a judge")
                }
            }

            impl HarnessResolver for InspectionRuntime {
                fn identity(
                    &self,
                    _artifact: &ArtifactDefinition,
                    _execution: &ExecutionDefinition,
                ) -> Result<HarnessIdentity, SkillEvalError> {
                    panic!("inspection resolved a harness")
                }
            }

            impl RunIdSource for InspectionRuntime {
                fn next(&mut self) -> Result<RunId, SkillEvalError> {
                    panic!("inspection allocated an ordinary run")
                }
            }

            impl CandidateRunner for InspectionRuntime {
                fn execute(
                    &mut self,
                    _run_id: &RunId,
                    _key: &TrialKey,
                    _artifact: &ArtifactDefinition,
                    _case: &CaseDefinition,
                    _model: &ModelIdentity,
                    _harness: &HarnessIdentity,
                    _candidate_timeout_seconds: Option<u32>,
                ) -> Result<CandidateArtifact, SkillEvalError> {
                    panic!("inspection executed a candidate")
                }
            }

            impl Verifier for InspectionRuntime {
                fn verify(
                    &mut self,
                    _case: &CaseDefinition,
                    _candidate: &CandidateArtifact,
                ) -> Result<Vec<CheckResult>, SkillEvalError> {
                    panic!("inspection verified a candidate")
                }
            }

            impl Judge for InspectionRuntime {
                fn grade(
                    &mut self,
                    _model: &ModelIdentity,
                    _input: &JudgeInput,
                ) -> Result<JudgeResult, SkillEvalError> {
                    panic!("inspection called a judge")
                }

                fn grade_prompt(
                    &mut self,
                    _model: &ModelIdentity,
                    _request: &PromptJudgeRequest,
                ) -> Result<PromptJudgeResult, SkillEvalError> {
                    panic!("inspection called a prompt judge")
                }
            }

            impl RunStore for InspectionRuntime {
                fn append(
                    &mut self,
                    _run_id: &RunId,
                    _event: &RunEvent,
                ) -> Result<(), SkillEvalError> {
                    panic!("inspection appended an event")
                }

                fn replay(
                    &self,
                    _run_id: &RunId,
                    _visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
                ) -> Result<(), SkillEvalError> {
                    panic!("inspection replayed events")
                }

                fn find_trial(
                    &self,
                    _selector: &TrialSelector,
                ) -> Result<TrialRecord, SkillEvalError> {
                    panic!("inspection loaded ordinary trials")
                }
            }

            impl Clock for InspectionRuntime {
                fn now(&self) -> Timestamp {
                    panic!("inspection read the clock")
                }
            }

            impl TierWriter for InspectionRuntime {
                fn write(
                    &mut self,
                    _artifact: &ArtifactDefinition,
                    _assignments: &[TierAssignment],
                ) -> Result<(), SkillEvalError> {
                    panic!("inspection wrote tiers")
                }
            }

            fn state() -> FrontierRunState {
                FrontierRunState {
                    configuration: FrontierRunConfiguration {
                        run_id: FrontierRunId("frontier-1".to_owned()),
                        created_at: timestamp(),
                        plan_path: PathBuf::from("plan.json"),
                        plan_sha256: "a".repeat(64),
                        plan: plan(),
                    },
                    status: FrontierRunStatus::Running,
                    models: vec![progress()],
                    cells: Vec::new(),
                    infrastructure_events: Vec::new(),
                    pause: None,
                    decision: None,
                    spent_millionths_of_dollar: 19,
                }
            }

            fn plan() -> FrontierPlan {
                FrontierPlan {
                    version: 1,
                    suite: FrontierSuiteIdentity {
                        path: PathBuf::from("suite.json"),
                        sha256: "b".repeat(64),
                        version: 1,
                    },
                    capabilities: T1ScreenSnapshotIdentity {
                        path: PathBuf::from("capabilities.json"),
                        sha256: "c".repeat(64),
                        version: 1,
                        observed_at_unix_seconds: 1,
                        pi_version: "pi-1".to_owned(),
                    },
                    entrants: vec![FrontierEntrant {
                        provider: "anthropic".to_owned(),
                        model: "alpha".to_owned(),
                        entry_tier: Tier::T1,
                        thinking_levels: vec!["low".to_owned(), "high".to_owned()],
                        catalog_observed_at: timestamp(),
                    }],
                    judge: ModelIdentity {
                        provider: "openai-codex".to_owned(),
                        model: "judge".to_owned(),
                        tier: Tier::T5,
                        thinking: "high".to_owned(),
                    },
                    policy: policy(),
                }
            }

            fn policy() -> FrontierPolicy {
                FrontierPolicy {
                    screening_trials_per_case: 1,
                    confirmation_trials_per_case: 3,
                    maximum_trials_per_case: 5,
                    minimum_trial_score: 8,
                    minimum_weighted_pass_basis_points: 8_500,
                    minimum_lower_bound_basis_points: 8_000,
                    confidence_level_basis_points: 9_500,
                    confidence_method: FrontierConfidenceMethod::StratifiedBootstrap,
                    confidence_resamples: 10,
                    maximum_infrastructure_attempts: 2,
                    maximum_catalog_age_seconds: 3_600,
                    active_pool_size: 5,
                    maximum_trial_cost_millionths_of_dollar: 100,
                    spending_limit_millionths_of_dollar: 10_000,
                    is_provider_limit_enforced: true,
                    is_first_party_only: true,
                }
            }

            fn progress() -> FrontierModelProgress {
                FrontierModelProgress {
                    provider: "anthropic".to_owned(),
                    model: "alpha".to_owned(),
                    entry_tier: Tier::T1,
                    selected_routes: vec![route(Tier::T1, "low")],
                    next_tier: Some(Tier::T2),
                    next_thinking_index: Some(0),
                    is_exhausted: false,
                }
            }

            fn cell(
                tier: Tier,
                thinking: &str,
                status: FrontierCellStatus,
                weighted: u16,
            ) -> FrontierCellEvidence {
                FrontierCellEvidence {
                    model: route(tier, thinking),
                    status,
                    set_aside_reason: None,
                    completed_trials: 2,
                    expected_trials: 2,
                    failed_trials: 0,
                    score: Some(FrontierScore {
                        weighted_pass_basis_points: weighted,
                        lower_bound_basis_points: 8_800,
                        critical_passed_trials: 2,
                        critical_expected_trials: 2,
                        is_group_coverage_complete: true,
                    }),
                    total_usage: TrialUsage {
                        input_tokens: 11,
                        output_tokens: 13,
                        cache_read_tokens: 5,
                        cache_write_tokens: 7,
                        turns: 2,
                        tool_calls: 3,
                        elapsed_milliseconds: 17,
                        cost_millionths_of_dollar: 19,
                    },
                }
            }

            fn route(tier: Tier, thinking: &str) -> ModelIdentity {
                ModelIdentity {
                    provider: "anthropic".to_owned(),
                    model: "alpha".to_owned(),
                    tier,
                    thinking: thinking.to_owned(),
                }
            }

            fn membership(tier: Tier, thinking: &str) -> FrontierPoolMembership {
                FrontierPoolMembership {
                    model: route(tier, thinking),
                    rank: 1,
                    is_active: true,
                }
            }

            fn baseline(memberships: Vec<FrontierPoolMembership>) -> FrontierBaseline {
                let pools =
                    memberships
                        .into_iter()
                        .fold(BTreeMap::new(), |mut pools, membership| {
                            pools
                                .entry(membership.model.tier)
                                .or_insert_with(Vec::new)
                                .push(membership);
                            pools
                        });
                FrontierBaseline {
                    accepted_at: timestamp(),
                    run_id: FrontierRunId("baseline-1".to_owned()),
                    run_evidence: FrontierEvidenceIdentity {
                        path: PathBuf::from("baseline.json"),
                        sha256: "d".repeat(64),
                    },
                    previous_entry_sha256: None,
                    pools,
                    capabilities: Vec::new(),
                }
            }

            fn selector() -> FrontierTrialSelector {
                FrontierTrialSelector {
                    run_id: FrontierRunId("frontier-1".to_owned()),
                    provider: "anthropic".to_owned(),
                    model: "alpha".to_owned(),
                    tier: Tier::T1,
                    thinking: "low".to_owned(),
                    artifact: ArtifactName("artifact".to_owned()),
                    case: CaseId("case".to_owned()),
                    attempt: 1,
                }
            }

            fn trial() -> TrialRecord {
                TrialRecord {
                    key: TrialKey {
                        artifact: ArtifactName("artifact".to_owned()),
                        tier: Tier::T1,
                        route_index: 0,
                        case: CaseId("case".to_owned()),
                        attempt: 1,
                    },
                    model: route(Tier::T1, "low"),
                    harness: HarnessIdentity {
                        runner_version: "runner".to_owned(),
                        pi_version: "pi".to_owned(),
                        artifact_revision: "revision".to_owned(),
                        tool_policy_digest: "policy".to_owned(),
                    },
                    artifact_path: PathBuf::from("artifact.txt"),
                    transcript_path: PathBuf::from("transcript.json"),
                    candidate_usage: zero_usage(),
                    judge_model: plan().judge,
                    judge_usage: zero_usage(),
                    verdict: TrialVerdict {
                        score: 10,
                        is_catastrophic: false,
                        failure_mode: None,
                        checks: Vec::new(),
                    },
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

            fn timestamp() -> Timestamp {
                Timestamp("2030-01-01T00:00:00+0000".to_owned())
            }
        }
    };
}
