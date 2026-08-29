#[cfg(test)]
macro_rules! frontier_decision_tests {
    () => {
        mod frontier_decision_tests {
            use std::cell::Cell;
            use std::collections::BTreeMap;
            use std::path::{Path, PathBuf};

            use $crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, CandidateArtifact, CaseDefinition,
                CaseDrive, CaseId, CheckResult, Decision, ExecutionDefinition, FrontierApplyReport,
                FrontierBaselineLedger, FrontierCaseGroup, FrontierCaseReference,
                FrontierCellEvidence, FrontierCellStatus, FrontierConfidenceMethod,
                FrontierDecisionRequest, FrontierEntrant, FrontierInspection, FrontierModelProgress,
                FrontierPlan, FrontierPolicy,
                FrontierRunConfiguration, FrontierRunId, FrontierRunState, FrontierRunStatus,
                FrontierSuite,
                FrontierSuiteIdentity, FrontierTierSuite, FrontierTrialSelector, HarnessIdentity,
                JudgeInput, JudgeResult, ModelIdentity, PromptJudgeRequest, PromptJudgeResult,
                RunEvent, RunId, SkillEvalError, T1ScreenSnapshotIdentity, Tier, TierAssignment,
                TierDestination, Timestamp, TrialKey, TrialRecord, TrialSelector, TrialUsage,
                TrialVerdict,
            };
            use $crate::ports::{
                ArtifactSource, CandidateRunner, Clock, FrontierRuntime, HarnessResolver, Judge,
                ModelResolver, QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
            };

            use super::{frontier_plan_digest, record_frontier_decision};

            #[test]
            fn rejection_is_terminal_and_preserves_ledger_without_publication() {
                let mut runtime = DecisionRuntime::new();
                let ledger = runtime.ledger.clone();
                let result = record_frontier_decision(
                    &request(Decision::Rejected, "  owner rejected  "),
                    &mut runtime,
                )
                .unwrap();

                assert_eq!(result.status, FrontierRunStatus::Rejected);
                assert_eq!(result.decision.unwrap().reason, "owner rejected");
                assert_eq!(runtime.state.as_ref().unwrap().status, FrontierRunStatus::Rejected);
                assert_eq!(runtime.ledger, ledger);
                assert_eq!(runtime.accept_calls, 0);
                assert_eq!(runtime.apply_calls, 0);
                assert_eq!(runtime.model_calls.get(), 0);
            }

            #[test]
            fn acceptance_commits_one_exact_hash_chained_suffix() {
                let mut runtime = DecisionRuntime::new();
                let result = record_frontier_decision(
                    &request(Decision::Accepted, "owner approved"),
                    &mut runtime,
                )
                .unwrap();

                assert_eq!(result.status, FrontierRunStatus::Accepted);
                assert_eq!(runtime.accept_calls, 1);
                assert_eq!(runtime.ledger.baselines.len(), 1);
                let baseline = &runtime.ledger.baselines[0];
                assert_eq!(baseline.run_id, FrontierRunId("decision-1".to_owned()));
                assert_eq!(baseline.previous_entry_sha256, None);
                assert_eq!(baseline.pools[&Tier::T5].len(), 1);
                assert_eq!(baseline.pools[&Tier::T5][0].rank, 1);
                assert!(baseline.pools[&Tier::T5][0].is_active);
                assert!(baseline.capabilities.is_empty());
                assert_eq!(runtime.apply_calls, 0);
                assert_eq!(runtime.model_calls.get(), 0);
            }

            #[test]
            fn quota_skipped_entrant_can_reach_an_owner_decision() {
                for decision in [Decision::Accepted, Decision::Rejected] {
                    let mut runtime = DecisionRuntime::new();
                    runtime.trials.clear();
                    let state = runtime.state.as_mut().unwrap();
                    let model = ModelIdentity {
                        provider: state.configuration.plan.entrants[0].provider.clone(),
                        model: state.configuration.plan.entrants[0].model.clone(),
                        tier: state.configuration.plan.entrants[0].entry_tier,
                        thinking: state.configuration.plan.entrants[0].thinking_levels[0].clone(),
                    };
                    state.models[0].selected_routes.clear();
                    state.models[0].next_tier = None;
                    state.models[0].next_thinking_index = None;
                    state.models[0].is_exhausted = true;
                    state.cells = vec![FrontierCellEvidence {
                        model,
                        status: FrontierCellStatus::Skipped,
                        set_aside_reason: None,
                        completed_trials: 0,
                        expected_trials: 0,
                        failed_trials: 0,
                        score: None,
                        total_usage: zero_usage(),
                    }];
                    state.spent_millionths_of_dollar = 0;

                    let result = record_frontier_decision(
                        &request(decision, "owner reviewed quota set-aside"),
                        &mut runtime,
                    )
                    .unwrap();

                    assert_eq!(
                        result.status,
                        if decision == Decision::Accepted {
                            FrontierRunStatus::Accepted
                        } else {
                            FrontierRunStatus::Rejected
                        }
                    );
                }
            }

            #[test]
            fn blank_repeated_and_every_nonawaiting_status_fail_without_writes() {
                let mut blank = DecisionRuntime::new();
                assert!(record_frontier_decision(&request(Decision::Accepted, " \t "), &mut blank).is_err());
                assert_eq!(blank.load_calls.get(), 0);

                for status in [
                    FrontierRunStatus::Pending,
                    FrontierRunStatus::Running,
                    FrontierRunStatus::Paused,
                    FrontierRunStatus::Failed,
                    FrontierRunStatus::Accepted,
                    FrontierRunStatus::Rejected,
                ] {
                    let mut runtime = DecisionRuntime::new();
                    runtime.state.as_mut().unwrap().status = status;
                    assert!(record_frontier_decision(
                        &request(Decision::Rejected, "reason"),
                        &mut runtime,
                    )
                    .is_err());
                    assert_eq!(runtime.save_calls, 0);
                    assert_eq!(runtime.accept_calls, 0);
                }

                let mut repeated = DecisionRuntime::new();
                record_frontier_decision(
                    &request(Decision::Rejected, "first"),
                    &mut repeated,
                )
                .unwrap();
                assert!(record_frontier_decision(
                    &request(Decision::Rejected, "second"),
                    &mut repeated,
                )
                .is_err());
                assert_eq!(repeated.save_calls, 1);
            }

            #[test]
            fn stale_partial_conflicting_and_gate_drift_fail_closed() {
                let mut stale = DecisionRuntime::new();
                stale.plan.policy.minimum_trial_score = 6;
                assert!(record_frontier_decision(
                    &request(Decision::Accepted, "reason"),
                    &mut stale,
                )
                .is_err());

                let mut partial = DecisionRuntime::new();
                partial.trials.pop();
                assert!(record_frontier_decision(
                    &request(Decision::Accepted, "reason"),
                    &mut partial,
                )
                .is_err());

                let mut conflict = DecisionRuntime::new();
                conflict.trials[0].harness.tool_policy_digest = "changed".to_owned();
                assert!(record_frontier_decision(
                    &request(Decision::Accepted, "reason"),
                    &mut conflict,
                )
                .is_err());

                let mut gate = DecisionRuntime::new();
                gate.state.as_mut().unwrap().cells[0]
                    .score
                    .as_mut()
                    .unwrap()
                    .critical_passed_trials = 0;
                assert!(record_frontier_decision(
                    &request(Decision::Accepted, "reason"),
                    &mut gate,
                )
                .is_err());

                for runtime in [&stale, &partial, &conflict, &gate] {
                    assert_eq!(runtime.save_calls, 0);
                    assert_eq!(runtime.accept_calls, 0);
                    assert_eq!(runtime.model_calls.get(), 0);
                }
            }

            #[test]
            fn acceptance_failure_leaves_pretransaction_authority_and_retry_finishes() {
                let mut runtime = DecisionRuntime::new();
                runtime.fail_accept_once = true;
                let old_state = runtime.state.clone();
                let old_ledger = runtime.ledger.clone();

                assert!(record_frontier_decision(
                    &request(Decision::Accepted, "owner approved"),
                    &mut runtime,
                )
                .is_err());
                assert_eq!(runtime.state, old_state);
                assert_eq!(runtime.ledger, old_ledger);

                let accepted = record_frontier_decision(
                    &request(Decision::Accepted, "owner approved"),
                    &mut runtime,
                )
                .unwrap();
                assert_eq!(accepted.status, FrontierRunStatus::Accepted);
                assert_eq!(runtime.state, Some(accepted));
                assert_eq!(runtime.ledger.baselines.len(), 1);
            }

            struct DecisionRuntime {
                state: Option<FrontierRunState>,
                plan: FrontierPlan,
                suite: FrontierSuite,
                trials: Vec<TrialRecord>,
                ledger: FrontierBaselineLedger,
                load_calls: Cell<u32>,
                model_calls: Cell<u32>,
                save_calls: u32,
                accept_calls: u32,
                apply_calls: u32,
                fail_accept_once: bool,
            }

            impl DecisionRuntime {
                fn new() -> Self {
                    let plan = plan();
                    let suite = suite();
                    let trials = trials();
                    let state = state(&plan, &suite, &trials);
                    Self {
                        state: Some(state),
                        plan,
                        suite,
                        trials,
                        ledger: FrontierBaselineLedger {
                            version: 1,
                            baselines: Vec::new(),
                        },
                        load_calls: Cell::new(0),
                        model_calls: Cell::new(0),
                        save_calls: 0,
                        accept_calls: 0,
                        apply_calls: 0,
                        fail_accept_once: false,
                    }
                }
            }

            impl QualificationRuntime for DecisionRuntime {}

            impl FrontierRuntime for DecisionRuntime {
                fn load_frontier_plan(
                    &self,
                    _path: &Path,
                ) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError> {
                    Ok((self.plan.clone(), self.suite.clone()))
                }

                fn next_frontier_run_id(&mut self) -> Result<FrontierRunId, SkillEvalError> {
                    panic!("decision allocated a run")
                }

                fn create_frontier(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    panic!("decision created a run")
                }

                fn load_frontier(
                    &self,
                    run_id: &FrontierRunId,
                ) -> Result<FrontierRunState, SkillEvalError> {
                    self.load_calls.set(self.load_calls.get() + 1);
                    let state = self
                        .state
                        .clone()
                        .ok_or_else(|| SkillEvalError::NotFound("frontier".to_owned()))?;
                    if state.configuration.run_id != *run_id {
                        return Err(SkillEvalError::NotFound("frontier".to_owned()));
                    }
                    Ok(state)
                }

                fn load_frontier_trials(
                    &self,
                    _run_id: &FrontierRunId,
                ) -> Result<Vec<TrialRecord>, SkillEvalError> {
                    Ok(self.trials.clone())
                }

                fn save_frontier(
                    &mut self,
                    state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    self.save_calls += 1;
                    self.state = Some(state.clone());
                    Ok(())
                }

                fn save_frontier_trial(
                    &mut self,
                    _run_id: &FrontierRunId,
                    _trial: &TrialRecord,
                ) -> Result<(), SkillEvalError> {
                    panic!("decision saved a trial")
                }

                fn inspect_frontier(
                    &self,
                    selector: &FrontierTrialSelector,
                ) -> Result<FrontierInspection, SkillEvalError> {
                    self.trials
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
                        .ok_or_else(|| SkillEvalError::NotFound("trial".to_owned()))
                }

                fn load_frontier_baselines(
                    &self,
                    path: &Path,
                ) -> Result<FrontierBaselineLedger, SkillEvalError> {
                    assert_eq!(path, Path::new("config/model-frontier-baseline.json"));
                    Ok(self.ledger.clone())
                }

                fn accept_frontier_baseline(
                    &mut self,
                    state: &FrontierRunState,
                    path: &Path,
                    ledger: &FrontierBaselineLedger,
                ) -> Result<(), SkillEvalError> {
                    self.accept_calls += 1;
                    assert_eq!(path, Path::new("config/model-frontier-baseline.json"));
                    if self.fail_accept_once {
                        self.fail_accept_once = false;
                        return Err(SkillEvalError::Io {
                            path: path.to_path_buf(),
                            message: "injected transaction failure".to_owned(),
                        });
                    }
                    self.state = Some(state.clone());
                    self.ledger = ledger.clone();
                    Ok(())
                }

                fn apply_frontier_routes(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<FrontierApplyReport, SkillEvalError> {
                    self.apply_calls += 1;
                    panic!("decision published routes")
                }
            }

            impl Clock for DecisionRuntime {
                fn now(&self) -> Timestamp {
                    timestamp()
                }
            }

            impl ArtifactSource for DecisionRuntime {
                fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
                    let name = root.file_name().unwrap().to_str().unwrap();
                    let case = self
                        .suite
                        .tiers
                        .values()
                        .flat_map(|tier| &tier.cases)
                        .find(|reference| reference.artifact_path == root)
                        .unwrap();
                    Ok(ArtifactDefinition {
                        name: ArtifactName(name.to_owned()),
                        kind: ArtifactKind::Skill,
                        root: root.to_path_buf(),
                        revision: case.artifact_revision.clone(),
                        required_destinations: vec![TierDestination::SkillMinimum],
                        current_tiers: Vec::new(),
                        cases: vec![CaseDefinition {
                            id: case.case.clone(),
                            input: "input".to_owned(),
                            expect: "expect".to_owned(),
                            source: "fixture".to_owned(),
                            is_holdout: true,
                            support_files: Vec::new(),
                            execution: ExecutionDefinition {
                                drive: CaseDrive::Response,
                                allowed_tools: Vec::new(),
                                timeout_seconds: 1,
                            },
                        }],
                    })
                }
            }

            impl HarnessResolver for DecisionRuntime {
                fn identity(
                    &self,
                    artifact: &ArtifactDefinition,
                    _execution: &ExecutionDefinition,
                ) -> Result<HarnessIdentity, SkillEvalError> {
                    Ok(HarnessIdentity {
                        runner_version: "runner-1".to_owned(),
                        pi_version: "pi-1".to_owned(),
                        artifact_revision: artifact.revision.clone(),
                        tool_policy_digest: "policy-1".to_owned(),
                    })
                }
            }

            impl ModelResolver for DecisionRuntime {
                fn candidates(&self, _tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    self.model_calls.set(self.model_calls.get() + 1);
                    panic!("decision resolved candidates")
                }

                fn qualification_routes(
                    &self,
                    _tier: Tier,
                ) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    self.model_calls.set(self.model_calls.get() + 1);
                    panic!("decision resolved routes")
                }

                fn exact_candidate(
                    &self,
                    _requested: &ModelIdentity,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    self.model_calls.set(self.model_calls.get() + 1);
                    panic!("decision resolved a model")
                }

                fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
                    self.model_calls.set(self.model_calls.get() + 1);
                    panic!("decision resolved a judge tier")
                }

                fn judge(
                    &self,
                    _judge_tier: Tier,
                    _candidate: Option<&ModelIdentity>,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    self.model_calls.set(self.model_calls.get() + 1);
                    panic!("decision resolved a judge")
                }
            }

            impl RunIdSource for DecisionRuntime {
                fn next(&mut self) -> Result<RunId, SkillEvalError> {
                    panic!("decision allocated an ordinary run")
                }
            }

            impl CandidateRunner for DecisionRuntime {
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
                    panic!("decision executed a candidate")
                }
            }

            impl Verifier for DecisionRuntime {
                fn verify(
                    &mut self,
                    _case: &CaseDefinition,
                    _candidate: &CandidateArtifact,
                ) -> Result<Vec<CheckResult>, SkillEvalError> {
                    panic!("decision ran verification")
                }
            }

            impl Judge for DecisionRuntime {
                fn grade(
                    &mut self,
                    _model: &ModelIdentity,
                    _input: &JudgeInput,
                ) -> Result<JudgeResult, SkillEvalError> {
                    panic!("decision graded a candidate")
                }

                fn grade_prompt(
                    &mut self,
                    _model: &ModelIdentity,
                    _request: &PromptJudgeRequest,
                ) -> Result<PromptJudgeResult, SkillEvalError> {
                    panic!("decision graded a prompt")
                }
            }

            impl RunStore for DecisionRuntime {
                fn append(
                    &mut self,
                    _run_id: &RunId,
                    _event: &RunEvent,
                ) -> Result<(), SkillEvalError> {
                    panic!("decision appended an ordinary event")
                }

                fn replay(
                    &self,
                    _run_id: &RunId,
                    _visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
                ) -> Result<(), SkillEvalError> {
                    panic!("decision replayed an ordinary run")
                }

                fn find_trial(
                    &self,
                    _selector: &TrialSelector,
                ) -> Result<TrialRecord, SkillEvalError> {
                    panic!("decision found an ordinary trial")
                }
            }

            impl TierWriter for DecisionRuntime {
                fn write(
                    &mut self,
                    _artifact: &ArtifactDefinition,
                    _assignments: &[TierAssignment],
                ) -> Result<(), SkillEvalError> {
                    panic!("decision wrote tiers")
                }
            }

            fn request(decision: Decision, reason: &str) -> FrontierDecisionRequest {
                FrontierDecisionRequest {
                    run_id: FrontierRunId("decision-1".to_owned()),
                    decision,
                    reason: reason.to_owned(),
                }
            }

            fn state(
                plan: &FrontierPlan,
                suite: &FrontierSuite,
                trials: &[TrialRecord],
            ) -> FrontierRunState {
                let model = candidate();
                let cell = $crate::statistics::evaluate_frontier_cell(
                    &suite.tiers[&Tier::T5],
                    &model,
                    trials,
                    &plan.policy,
                )
                .unwrap();
                let initial = FrontierModelProgress {
                    provider: model.provider.clone(),
                    model: model.model.clone(),
                    entry_tier: Tier::T5,
                    selected_routes: Vec::new(),
                    next_tier: Some(Tier::T5),
                    next_thinking_index: Some(0),
                    is_exhausted: false,
                };
                let progress = $crate::statistics::advance_frontier_model(
                    &plan.entrants[0],
                    &initial,
                    std::slice::from_ref(&cell),
                )
                .unwrap();
                let spend = trials.iter().fold(0_u64, |total, trial| {
                    total
                        + trial.candidate_usage.cost_millionths_of_dollar
                        + trial.judge_usage.cost_millionths_of_dollar
                });
                FrontierRunState {
                    configuration: FrontierRunConfiguration {
                        run_id: FrontierRunId("decision-1".to_owned()),
                        created_at: timestamp(),
                        plan_path: PathBuf::from("frontier-plan.json"),
                        plan_sha256: frontier_plan_digest(plan).unwrap(),
                        plan: plan.clone(),
                    },
                    status: FrontierRunStatus::AwaitingDecision,
                    models: vec![progress],
                    cells: vec![cell],
                    infrastructure_events: Vec::new(),
                    pause: None,
                    decision: None,
                    spent_millionths_of_dollar: spend,
                }
            }

            fn plan() -> FrontierPlan {
                FrontierPlan {
                    version: 1,
                    suite: FrontierSuiteIdentity {
                        path: PathBuf::from("suite.json"),
                        sha256: "a".repeat(64),
                        version: 1,
                    },
                    capabilities: T1ScreenSnapshotIdentity {
                        path: PathBuf::from("capabilities.json"),
                        sha256: "b".repeat(64),
                        version: 1,
                        observed_at_unix_seconds: 1_893_456_000,
                        pi_version: "pi-1".to_owned(),
                    },
                    entrants: vec![FrontierEntrant {
                        provider: "anthropic".to_owned(),
                        model: "candidate".to_owned(),
                        entry_tier: Tier::T5,
                        thinking_levels: vec!["off".to_owned()],
                        catalog_observed_at: timestamp(),
                    }],
                    judge: judge(),
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
                        spending_limit_millionths_of_dollar: 10_000,
                        is_provider_limit_enforced: true,
                        is_first_party_only: true,
                    },
                }
            }

            fn suite() -> FrontierSuite {
                FrontierSuite {
                    version: 1,
                    tiers: [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5]
                        .into_iter()
                        .map(|tier| {
                            let cases = (0..30)
                                .map(|index| FrontierCaseReference {
                                    artifact_path: PathBuf::from(format!(
                                        "skills/{tier:?}-case-{index:02}"
                                    )),
                                    artifact_revision: format!("revision-{tier:?}-{index:02}"),
                                    case: CaseId(format!("{tier:?}-case-{index:02}")),
                                    group: match index % 4 {
                                        0 => FrontierCaseGroup::Normal,
                                        1 => FrontierCaseGroup::Edge,
                                        2 => FrontierCaseGroup::Adversarial,
                                        _ => FrontierCaseGroup::Critical,
                                    },
                                    is_confirmation: index == 29,
                                })
                                .collect();
                            (
                                tier,
                                FrontierTierSuite {
                                    group_weights_basis_points: BTreeMap::from([
                                        (FrontierCaseGroup::Normal, 2_500),
                                        (FrontierCaseGroup::Edge, 2_500),
                                        (FrontierCaseGroup::Adversarial, 2_500),
                                        (FrontierCaseGroup::Critical, 2_500),
                                    ]),
                                    cases,
                                },
                            )
                        })
                        .collect(),
                }
            }

            fn trials() -> Vec<TrialRecord> {
                (0..30)
                    .flat_map(|index| {
                        (1..=3).map(move |attempt| TrialRecord {
                            key: TrialKey {
                                artifact: ArtifactName(format!("T5-case-{index:02}")),
                                tier: Tier::T5,
                                route_index: 0,
                                case: CaseId(format!("T5-case-{index:02}")),
                                attempt,
                            },
                            model: candidate(),
                            harness: HarnessIdentity {
                                runner_version: "runner-1".to_owned(),
                                pi_version: "pi-1".to_owned(),
                                artifact_revision: format!("revision-T5-{index:02}"),
                                tool_policy_digest: "policy-1".to_owned(),
                            },
                            artifact_path: PathBuf::from(format!(
                                "artifact-{index:02}-{attempt}"
                            )),
                            transcript_path: PathBuf::from(format!(
                                "transcript-{index:02}-{attempt}"
                            )),
                            candidate_usage: usage(1),
                            judge_model: judge(),
                            judge_usage: zero_usage(),
                            verdict: TrialVerdict {
                                score: 10,
                                is_catastrophic: false,
                                failure_mode: None,
                                checks: Vec::new(),
                            },
                        })
                    })
                    .collect()
            }

            fn candidate() -> ModelIdentity {
                ModelIdentity {
                    provider: "anthropic".to_owned(),
                    model: "candidate".to_owned(),
                    tier: Tier::T5,
                    thinking: "off".to_owned(),
                }
            }

            fn judge() -> ModelIdentity {
                ModelIdentity {
                    provider: "openai-codex".to_owned(),
                    model: "judge".to_owned(),
                    tier: Tier::T5,
                    thinking: "high".to_owned(),
                }
            }

            fn timestamp() -> Timestamp {
                Timestamp("2030-01-01T00:00:00+0000".to_owned())
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
        }
    };
}
