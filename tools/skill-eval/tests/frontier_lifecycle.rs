#[cfg(test)]
macro_rules! frontier_lifecycle_tests {
    () => {
        mod frontier_lifecycle {
            use std::cell::{Cell, RefCell};
            use std::collections::BTreeMap;
            use std::path::{Path, PathBuf};
            use std::rc::Rc;

            use $crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, CandidateArtifact, CaseDefinition,
                CaseDrive, CaseId, CheckResult, ExecutionDefinition, FrontierApplyReport,
                FrontierBaselineLedger, FrontierCaseGroup, FrontierCaseReference,
                FrontierConfidenceMethod, FrontierEntrant, FrontierInspection, FrontierPlan,
                FrontierPolicy, FrontierRunId, FrontierRunState, FrontierRunStatus, FrontierSuite,
                FrontierSuiteIdentity, FrontierTierSuite, HarnessIdentity, JudgeInput, JudgeResult,
                ModelIdentity, PromptJudgeRequest, PromptJudgeResult, RunEvent, RunId,
                SkillEvalError, T1ScreenSnapshotIdentity, Tier, TierAssignment, TierDestination,
                Timestamp, TrialKey, TrialRecord, TrialSelector, TrialUsage, TrialVerdict,
            };
            use $crate::ports::{
                ArtifactSource, CandidateRunner, Clock, FrontierProgressSink, FrontierRuntime,
                HarnessResolver, Judge, ModelResolver, QualificationRuntime, RunIdSource, RunStore,
                TierWriter, Verifier,
            };

            use super::{resume_frontier, start_frontier};

            #[derive(Default)]
            struct Durable {
                state: Option<FrontierRunState>,
                trials: Vec<TrialRecord>,
                log: Vec<&'static str>,
                save_error_once: bool,
            }

            struct FakeRuntime {
                durable: Rc<RefCell<Durable>>,
                plan: FrontierPlan,
                suite: FrontierSuite,
                candidate_error: Option<SkillEvalError>,
                verifier_error: Option<SkillEvalError>,
                judge_error: Option<SkillEvalError>,
                recovery_error: Option<SkillEvalError>,
                recovered_cost: Option<u64>,
                exceptional_retry: bool,
                execute_calls: u32,
                timeouts: Vec<Option<u32>>,
                models: Vec<ModelIdentity>,
                harness_runner_version: String,
                now: Timestamp,
                bulk_trial_loads: Cell<u32>,
                selector_inspections: Cell<u32>,
            }

            impl FakeRuntime {
                fn new() -> Self {
                    let suite = suite();
                    Self {
                        durable: Rc::new(RefCell::new(Durable::default())),
                        plan: plan(),
                        suite,
                        candidate_error: None,
                        verifier_error: None,
                        judge_error: None,
                        recovery_error: None,
                        recovered_cost: None,
                        exceptional_retry: false,
                        execute_calls: 0,
                        timeouts: Vec::new(),
                        models: Vec::new(),
                        harness_runner_version: "runner-1".to_owned(),
                        now: Timestamp("2030-01-01T00:00:00+0000".to_owned()),
                        bulk_trial_loads: Cell::new(0),
                        selector_inspections: Cell::new(0),
                    }
                }
            }

            struct Progress {
                durable: Rc<RefCell<Durable>>,
                states: Vec<FrontierRunState>,
            }

            impl FrontierProgressSink for Progress {
                fn emit_frontier(
                    &mut self,
                    state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    assert_eq!(self.durable.borrow().state.as_ref(), Some(state));
                    self.durable.borrow_mut().log.push("emit");
                    self.states.push(state.clone());
                    Ok(())
                }
            }

            impl QualificationRuntime for FakeRuntime {}

            impl FrontierRuntime for FakeRuntime {
                fn load_frontier_plan(
                    &self,
                    _path: &Path,
                ) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError> {
                    Ok((self.plan.clone(), self.suite.clone()))
                }

                fn next_frontier_run_id(&mut self) -> Result<FrontierRunId, SkillEvalError> {
                    Ok(FrontierRunId("frontier-1".to_owned()))
                }

                fn authorize_exceptional_frontier_retry(
                    &self,
                    _event: &$crate::model::FrontierInfrastructureEvent,
                ) -> bool {
                    self.exceptional_retry
                }

                fn create_frontier(
                    &mut self,
                    state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    let mut durable = self.durable.borrow_mut();
                    if durable.state.is_some() {
                        return Err(SkillEvalError::InvalidConfiguration(
                            "duplicate create".to_owned(),
                        ));
                    }
                    durable.log.push("create");
                    durable.state = Some(state.clone());
                    Ok(())
                }

                fn load_frontier(
                    &self,
                    _run_id: &FrontierRunId,
                ) -> Result<FrontierRunState, SkillEvalError> {
                    self.durable
                        .borrow()
                        .state
                        .clone()
                        .ok_or_else(|| SkillEvalError::NotFound("frontier".to_owned()))
                }

                fn save_frontier(
                    &mut self,
                    state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    let mut durable = self.durable.borrow_mut();
                    if durable.save_error_once && !durable.trials.is_empty() {
                        durable.save_error_once = false;
                        return Err(SkillEvalError::Io {
                            path: PathBuf::from("state"),
                            message: "synthetic save failure".to_owned(),
                        });
                    }
                    durable.log.push("save");
                    durable.state = Some(state.clone());
                    Ok(())
                }

                fn recover_frontier_trial(
                    &mut self,
                    state: &FrontierRunState,
                    key: &TrialKey,
                    _artifact: &ArtifactDefinition,
                    _case: &CaseDefinition,
                    model: &ModelIdentity,
                    harness: &HarnessIdentity,
                ) -> Result<Option<TrialRecord>, SkillEvalError> {
                    if let Some(error) = self.recovery_error.take() {
                        return Err(error);
                    }
                    Ok(self.recovered_cost.take().map(|cost| TrialRecord {
                        key: key.clone(),
                        model: model.clone(),
                        harness: harness.clone(),
                        artifact_path: PathBuf::from("recovered/artifact"),
                        transcript_path: PathBuf::from("recovered/transcript.jsonl"),
                        candidate_usage: usage(cost),
                        judge_model: state.configuration.plan.judge.clone(),
                        judge_usage: usage(0),
                        verdict: TrialVerdict {
                            score: 10,
                            is_catastrophic: false,
                            failure_mode: None,
                            checks: Vec::new(),
                        },
                    }))
                }

                fn save_frontier_trial(
                    &mut self,
                    _run_id: &FrontierRunId,
                    trial: &TrialRecord,
                ) -> Result<(), SkillEvalError> {
                    let mut durable = self.durable.borrow_mut();
                    if durable.trials.iter().any(|stored| stored.key == trial.key) {
                        return Err(SkillEvalError::InvalidConfiguration(
                            "duplicate trial".to_owned(),
                        ));
                    }
                    durable.log.push("trial");
                    durable.trials.push(trial.clone());
                    Ok(())
                }

                fn load_frontier_trials(
                    &self,
                    _run_id: &FrontierRunId,
                ) -> Result<Vec<TrialRecord>, SkillEvalError> {
                    self.bulk_trial_loads
                        .set(self.bulk_trial_loads.get().saturating_add(1));
                    Ok(self.durable.borrow().trials.clone())
                }

                fn inspect_frontier(
                    &self,
                    selector: &$crate::model::FrontierTrialSelector,
                ) -> Result<FrontierInspection, SkillEvalError> {
                    self.selector_inspections
                        .set(self.selector_inspections.get().saturating_add(1));
                    self.durable
                        .borrow()
                        .trials
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
                    _path: &Path,
                ) -> Result<FrontierBaselineLedger, SkillEvalError> {
                    panic!("lifecycle read a baseline")
                }

                fn accept_frontier_baseline(
                    &mut self,
                    _state: &FrontierRunState,
                    _path: &Path,
                    _ledger: &FrontierBaselineLedger,
                ) -> Result<(), SkillEvalError> {
                    panic!("lifecycle accepted a baseline")
                }

                fn apply_frontier_routes(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<FrontierApplyReport, SkillEvalError> {
                    panic!("lifecycle published routes")
                }
            }

            impl Clock for FakeRuntime {
                fn now(&self) -> Timestamp {
                    self.now.clone()
                }
            }

            impl ArtifactSource for FakeRuntime {
                fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
                    let stem = root
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("tier-T5");
                    Ok(artifact(stem))
                }
            }

            impl ModelResolver for FakeRuntime {
                fn candidates(&self, _tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    Ok(Vec::new())
                }

                fn qualification_routes(
                    &self,
                    _tier: Tier,
                ) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    Ok(Vec::new())
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
                    panic!("frontier used the incumbent tier judge instead of the frozen judge")
                }
            }

            impl HarnessResolver for FakeRuntime {
                fn identity(
                    &self,
                    artifact: &ArtifactDefinition,
                    _execution: &ExecutionDefinition,
                ) -> Result<HarnessIdentity, SkillEvalError> {
                    Ok(HarnessIdentity {
                        runner_version: self.harness_runner_version.clone(),
                        pi_version: "pi-1".to_owned(),
                        artifact_revision: artifact.revision.clone(),
                        tool_policy_digest: "tools-1".to_owned(),
                    })
                }
            }

            impl RunIdSource for FakeRuntime {
                fn next(&mut self) -> Result<RunId, SkillEvalError> {
                    Ok(RunId("unused".to_owned()))
                }
            }

            impl CandidateRunner for FakeRuntime {
                fn execute(
                    &mut self,
                    _run_id: &RunId,
                    key: &TrialKey,
                    _artifact: &ArtifactDefinition,
                    _case: &CaseDefinition,
                    model: &ModelIdentity,
                    harness: &HarnessIdentity,
                    candidate_timeout_seconds: Option<u32>,
                ) -> Result<CandidateArtifact, SkillEvalError> {
                    assert!(self.durable.borrow().state.is_some());
                    self.execute_calls += 1;
                    self.timeouts.push(candidate_timeout_seconds);
                    self.models.push(model.clone());
                    if let Some(error) = self.candidate_error.take() {
                        return Err(error);
                    }
                    Ok(CandidateArtifact {
                        key: key.clone(),
                        model: model.clone(),
                        harness: harness.clone(),
                        artifact_path: PathBuf::from(format!("artifacts/{}.txt", key.case.0)),
                        transcript_path: PathBuf::from(format!("transcripts/{}.json", key.case.0)),
                        usage: usage(1),
                    })
                }
            }

            impl Verifier for FakeRuntime {
                fn verify(
                    &mut self,
                    _case: &CaseDefinition,
                    _candidate: &CandidateArtifact,
                ) -> Result<Vec<CheckResult>, SkillEvalError> {
                    if let Some(error) = self.verifier_error.take() {
                        return Err(error);
                    }
                    Ok(Vec::new())
                }
            }

            impl Judge for FakeRuntime {
                fn grade(
                    &mut self,
                    model: &ModelIdentity,
                    _input: &JudgeInput,
                ) -> Result<JudgeResult, SkillEvalError> {
                    if let Some(error) = self.judge_error.take() {
                        return Err(error);
                    }
                    Ok(JudgeResult {
                        model: model.clone(),
                        usage: usage(1),
                        verdict: TrialVerdict {
                            score: 0,
                            is_catastrophic: false,
                            failure_mode: Some("fixture failure".to_owned()),
                            checks: Vec::new(),
                        },
                    })
                }

                fn grade_prompt(
                    &mut self,
                    _model: &ModelIdentity,
                    _request: &PromptJudgeRequest,
                ) -> Result<PromptJudgeResult, SkillEvalError> {
                    panic!("lifecycle called prompt judge")
                }
            }

            impl RunStore for FakeRuntime {
                fn append(
                    &mut self,
                    _run_id: &RunId,
                    _event: &RunEvent,
                ) -> Result<(), SkillEvalError> {
                    panic!("lifecycle appended an ordinary event")
                }

                fn replay(
                    &self,
                    _run_id: &RunId,
                    _visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
                ) -> Result<(), SkillEvalError> {
                    panic!("lifecycle replayed an ordinary run")
                }

                fn find_trial(
                    &self,
                    _selector: &TrialSelector,
                ) -> Result<TrialRecord, SkillEvalError> {
                    panic!("lifecycle searched an ordinary run")
                }
            }

            impl TierWriter for FakeRuntime {
                fn write(
                    &mut self,
                    _artifact: &ArtifactDefinition,
                    _assignments: &[TierAssignment],
                ) -> Result<(), SkillEvalError> {
                    panic!("lifecycle wrote tiers")
                }
            }

            #[test]
            fn fake_full_lifecycle_is_durable_and_uses_the_exact_unbounded_route() {
                let mut runtime = FakeRuntime::new();
                let durable = runtime.durable.clone();
                let mut progress = Progress {
                    durable,
                    states: Vec::new(),
                };

                let state =
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .unwrap();

                assert_eq!(state.status, FrontierRunStatus::AwaitingDecision);
                assert_eq!(runtime.execute_calls, 30);
                assert!(runtime.timeouts.iter().all(Option::is_none));
                assert!(runtime.models.iter().all(|model| {
                    model.provider == "anthropic"
                        && model.model == "candidate"
                        && model.thinking == "off"
                        && model.tier == Tier::T5
                }));
                let durable = runtime.durable.borrow();
                assert_eq!(durable.trials.len(), 30);
                assert!(durable.trials.iter().all(|trial| {
                    trial.candidate_usage.cost_millionths_of_dollar == 1
                        && trial.judge_usage.cost_millionths_of_dollar == 1
                }));
                assert_eq!(state.spent_millionths_of_dollar, 60);
                assert!(
                    durable
                        .log
                        .windows(2)
                        .any(|events| events == ["trial", "save"])
                );
                assert!(progress.states.iter().all(|saved| {
                    durable
                        .state
                        .as_ref()
                        .is_some_and(|terminal| saved.configuration == terminal.configuration)
                }));
            }

            #[test]
            fn candidate_quota_sets_the_entrant_aside_without_a_retry() {
                let mut runtime = FakeRuntime::new();
                runtime.candidate_error = Some(SkillEvalError::Quota {
                    model: candidate(),
                    reset_at: Some(runtime.now()),
                });
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };
                let state =
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .unwrap();
                assert_eq!(state.status, FrontierRunStatus::AwaitingDecision);
                assert_eq!(state.spent_millionths_of_dollar, 10);
                assert_eq!(state.infrastructure_events.len(), 1);
                assert_eq!(
                    state.infrastructure_events[0].charged_millionths_of_dollar,
                    0
                );
                assert_eq!(state.cells.len(), 1);
                assert_eq!(
                    state.cells[0].status,
                    $crate::model::FrontierCellStatus::Skipped
                );
                assert!(state.models[0].is_exhausted);
                assert_eq!(runtime.execute_calls, 6);
                assert_eq!(runtime.durable.borrow().trials.len(), 5);
            }

            #[test]
            fn judge_quota_remains_paused_after_the_discovery_snapshot_ages_out() {
                let mut runtime = FakeRuntime::new();
                runtime.candidate_error = Some(SkillEvalError::Quota {
                    model: judge(),
                    reset_at: Some(runtime.now()),
                });
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };
                let paused =
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .unwrap();
                assert_eq!(paused.status, FrontierRunStatus::Paused);

                runtime.now = Timestamp("2030-01-01T02:00:00+0000".to_owned());
                let state =
                    resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress)
                        .unwrap();
                assert_eq!(state.status, FrontierRunStatus::Paused);
                assert_eq!(state.pause, paused.pause);
                assert_eq!(runtime.execute_calls, 6);
                assert_eq!(runtime.durable.borrow().trials.len(), 5);
            }

            #[test]
            fn legacy_candidate_quota_pause_sets_aside_without_another_call() {
                let mut runtime = FakeRuntime::new();
                runtime.candidate_error = Some(SkillEvalError::Quota {
                    model: judge(),
                    reset_at: None,
                });
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };
                let mut paused =
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .unwrap();
                paused.pause = Some($crate::model::PoolPauseReason::Quota {
                    model: candidate(),
                    reset_at: None,
                });
                runtime.durable.borrow_mut().state = Some(paused.clone());

                let state =
                    resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress)
                        .unwrap();

                assert_eq!(state.status, FrontierRunStatus::AwaitingDecision);
                assert_eq!(state.cells.len(), 1);
                assert_eq!(
                    state.cells[0].status,
                    $crate::model::FrontierCellStatus::Skipped
                );
                assert!(state.models[0].is_exhausted);
                assert_eq!(runtime.execute_calls, 6);
                assert_eq!(runtime.durable.borrow().trials.len(), 5);
            }

            #[test]
            fn resume_bulk_loads_durable_trials_once() {
                let mut runtime = FakeRuntime::new();
                runtime.verifier_error = Some(SkillEvalError::Process {
                    program: "local-verifier".to_owned(),
                    exit_code: Some(1),
                    standard_error: "first".to_owned(),
                });
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };
                let paused =
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .unwrap();
                assert_eq!(runtime.durable.borrow().trials.len(), 5);

                let resumed =
                    resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress)
                        .unwrap();

                assert_eq!(resumed.status, FrontierRunStatus::AwaitingDecision);
                assert_eq!(runtime.bulk_trial_loads.get(), 1);
                assert_eq!(runtime.selector_inspections.get(), 0);
            }

            #[test]
            fn resume_does_not_reuse_a_failed_attempt_charge() {
                let mut runtime = FakeRuntime::new();
                runtime.verifier_error = Some(SkillEvalError::Process {
                    program: "local-verifier".to_owned(),
                    exit_code: Some(1),
                    standard_error: "first".to_owned(),
                });
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };
                let paused =
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .unwrap();
                assert_eq!(paused.infrastructure_events.len(), 1);
                assert_eq!(
                    paused.infrastructure_events[0].charged_millionths_of_dollar,
                    10
                );
                assert_eq!(paused.spent_millionths_of_dollar, 20);

                let resumed =
                    resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress)
                        .unwrap();
                assert_eq!(resumed.status, FrontierRunStatus::AwaitingDecision);
                assert_eq!(resumed.infrastructure_events, paused.infrastructure_events);
                assert_eq!(resumed.spent_millionths_of_dollar, 70);
                assert_eq!(runtime.execute_calls, 31);
            }

            #[test]
            fn repeated_candidate_infrastructure_sets_the_entrant_aside() {
                let mut runtime = FakeRuntime::new();
                runtime.candidate_error = Some(SkillEvalError::Process {
                    program: "pi".to_owned(),
                    exit_code: Some(1),
                    standard_error: "first".to_owned(),
                });
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };
                let paused =
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .unwrap();
                assert_eq!(paused.infrastructure_events.len(), 1);

                runtime.candidate_error = Some(SkillEvalError::Process {
                    program: "pi".to_owned(),
                    exit_code: Some(1),
                    standard_error: "second".to_owned(),
                });
                let paused =
                    resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress)
                        .unwrap();
                assert_eq!(paused.infrastructure_events.len(), 2);
                let calls = runtime.execute_calls;
                let completed =
                    resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress)
                        .unwrap();
                assert_eq!(completed.status, FrontierRunStatus::AwaitingDecision);
                assert_eq!(
                    completed.infrastructure_events,
                    paused.infrastructure_events
                );
                assert_eq!(completed.cells.len(), 1);
                assert_eq!(
                    completed.cells[0].status,
                    $crate::model::FrontierCellStatus::Skipped
                );
                assert_eq!(
                    completed.cells[0].set_aside_reason,
                    Some($crate::model::FrontierSetAsideReason::Infrastructure)
                );
                assert!(completed.models[0].is_exhausted);
                assert_eq!(runtime.execute_calls, calls);
            }

            #[test]
            fn repeated_non_candidate_infrastructure_remains_paused() {
                let mut runtime = FakeRuntime::new();
                runtime.verifier_error = Some(SkillEvalError::InvalidConfiguration(
                    "first verifier configuration failure".to_owned(),
                ));
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };
                let paused =
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .unwrap();

                runtime.verifier_error = Some(SkillEvalError::InvalidConfiguration(
                    "second verifier configuration failure".to_owned(),
                ));
                let paused =
                    resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress)
                        .unwrap();
                let calls = runtime.execute_calls;
                let unchanged =
                    resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress)
                        .unwrap();

                assert_eq!(unchanged, paused);
                assert_eq!(unchanged.status, FrontierRunStatus::Paused);
                assert!(unchanged.cells.is_empty());
                assert_eq!(runtime.execute_calls, calls);
                assert!(unchanged.infrastructure_events.iter().all(|event| {
                    event.failure_stage == Some($crate::model::FrontierFailureStage::Verifier)
                }));
            }

            #[test]
            fn owner_authorized_judge_retry_gets_one_extra_dispatch() {
                let mut runtime = FakeRuntime::new();
                runtime.judge_error = Some(SkillEvalError::InvalidConfiguration(
                    "first judge failure".to_owned(),
                ));
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };
                let paused =
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .unwrap();
                runtime.judge_error = Some(SkillEvalError::InvalidConfiguration(
                    "second judge failure".to_owned(),
                ));
                let paused =
                    resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress)
                        .unwrap();
                let calls = runtime.execute_calls;
                runtime.exceptional_retry = true;

                let completed =
                    resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress)
                        .unwrap();

                assert_eq!(completed.status, FrontierRunStatus::AwaitingDecision);
                assert!(runtime.execute_calls > calls);
                assert_eq!(completed.infrastructure_events.len(), 2);
            }

            #[test]
            fn judge_configuration_failure_is_a_retryable_infrastructure_pause() {
                let mut runtime = FakeRuntime::new();
                runtime.judge_error = Some(SkillEvalError::InvalidConfiguration(
                    "judge packet is too large".to_owned(),
                ));
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };

                let paused =
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .unwrap();

                assert_eq!(paused.status, FrontierRunStatus::Paused);
                assert_eq!(paused.infrastructure_events.len(), 1);
                assert_eq!(
                    paused.infrastructure_events[0].failure_stage,
                    Some($crate::model::FrontierFailureStage::Judge)
                );
            }

            #[test]
            fn candidate_and_recovery_configuration_failures_remain_terminal() {
                let mut candidate_runtime = FakeRuntime::new();
                candidate_runtime.candidate_error = Some(SkillEvalError::InvalidConfiguration(
                    "candidate configuration failure".to_owned(),
                ));
                let mut candidate_progress = Progress {
                    durable: candidate_runtime.durable.clone(),
                    states: Vec::new(),
                };

                assert!(
                    start_frontier(
                        Path::new("frontier-plan.json"),
                        &mut candidate_runtime,
                        &mut candidate_progress,
                    )
                    .is_err()
                );

                let mut recovery_runtime = FakeRuntime::new();
                recovery_runtime.recovery_error = Some(SkillEvalError::InvalidConfiguration(
                    "recovery configuration failure".to_owned(),
                ));
                let mut recovery_progress = Progress {
                    durable: recovery_runtime.durable.clone(),
                    states: Vec::new(),
                };

                assert!(
                    start_frontier(
                        Path::new("frontier-plan.json"),
                        &mut recovery_runtime,
                        &mut recovery_progress,
                    )
                    .is_err()
                );
                assert_eq!(recovery_runtime.execute_calls, 5);
            }

            #[test]
            fn recovered_trial_over_the_frozen_reservation_is_rejected_before_append() {
                let mut runtime = FakeRuntime::new();
                runtime.recovered_cost = Some(11);
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };

                assert!(
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .is_err()
                );

                assert_eq!(runtime.execute_calls, 0);
                assert!(runtime.durable.borrow().trials.is_empty());
                assert_eq!(
                    runtime
                        .durable
                        .borrow()
                        .state
                        .as_ref()
                        .unwrap()
                        .spent_millionths_of_dollar,
                    0
                );
            }

            #[test]
            fn durable_trial_recovers_after_parent_save_failure_without_a_duplicate_call() {
                let mut runtime = FakeRuntime::new();
                runtime.durable.borrow_mut().save_error_once = true;
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };
                assert!(
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress,)
                        .is_err()
                );
                assert_eq!(runtime.execute_calls, 6);
                assert_eq!(runtime.durable.borrow().trials.len(), 6);
                let run_id = runtime
                    .durable
                    .borrow()
                    .state
                    .as_ref()
                    .unwrap()
                    .configuration
                    .run_id
                    .clone();

                let state = resume_frontier(&run_id, &mut runtime, &mut progress).unwrap();

                assert_eq!(state.status, FrontierRunStatus::AwaitingDecision);
                assert_eq!(runtime.execute_calls, 30);
                assert_eq!(runtime.durable.borrow().trials.len(), 30);
            }

            #[test]
            fn durable_trial_harness_drift_fails_resume_before_another_model_call() {
                let mut runtime = FakeRuntime::new();
                runtime.durable.borrow_mut().save_error_once = true;
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };
                assert!(
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .is_err()
                );
                assert_eq!(runtime.execute_calls, 6);
                runtime.harness_runner_version = "runner-2".to_owned();
                let run_id = runtime
                    .durable
                    .borrow()
                    .state
                    .as_ref()
                    .unwrap()
                    .configuration
                    .run_id
                    .clone();

                assert!(resume_frontier(&run_id, &mut runtime, &mut progress).is_err());
                assert_eq!(runtime.execute_calls, 6);
            }

            #[test]
            fn tampered_persisted_model_progress_fails_before_another_model_call() {
                for field in [
                    "selected_routes",
                    "next_tier",
                    "next_thinking_index",
                    "is_exhausted",
                ] {
                    let mut runtime = FakeRuntime::new();
                    runtime.candidate_error = Some(SkillEvalError::Quota {
                        model: candidate(),
                        reset_at: None,
                    });
                    let mut progress = Progress {
                        durable: runtime.durable.clone(),
                        states: Vec::new(),
                    };
                    let paused = start_frontier(
                        Path::new("frontier-plan.json"),
                        &mut runtime,
                        &mut progress,
                    )
                    .unwrap();
                    {
                        let mut durable = runtime.durable.borrow_mut();
                        let model = &mut durable.state.as_mut().unwrap().models[0];
                        match field {
                            "selected_routes" => model.selected_routes.push(candidate()),
                            "next_tier" => model.next_tier = Some(Tier::T4),
                            "next_thinking_index" => model.next_thinking_index = None,
                            "is_exhausted" => model.is_exhausted = true,
                            _ => unreachable!(),
                        }
                    }
                    let calls = runtime.execute_calls;

                    assert!(
                        resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress,)
                            .is_err(),
                        "{field} drift resumed"
                    );
                    assert_eq!(runtime.execute_calls, calls, "{field} made a model call");
                }
            }

            #[test]
            fn corrupt_durable_trial_fails_resume_before_another_model_call() {
                let mut runtime = FakeRuntime::new();
                runtime.candidate_error = Some(SkillEvalError::Quota {
                    model: candidate(),
                    reset_at: None,
                });
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };
                let paused =
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .unwrap();
                let model = candidate();
                let key = TrialKey {
                    artifact: ArtifactName("tier-T5".to_owned()),
                    tier: Tier::T5,
                    route_index: 9,
                    case: CaseId("T5-case-00".to_owned()),
                    attempt: 1,
                };
                runtime.durable.borrow_mut().trials.push(TrialRecord {
                    key,
                    model: model.clone(),
                    harness: HarnessIdentity {
                        runner_version: "runner-1".to_owned(),
                        pi_version: "pi-1".to_owned(),
                        artifact_revision: "revision-T5".to_owned(),
                        tool_policy_digest: "tools-1".to_owned(),
                    },
                    artifact_path: PathBuf::from("artifacts/corrupt.txt"),
                    transcript_path: PathBuf::from("transcripts/corrupt.json"),
                    candidate_usage: usage(1),
                    judge_model: judge(),
                    judge_usage: usage(1),
                    verdict: TrialVerdict {
                        score: 0,
                        is_catastrophic: false,
                        failure_mode: Some("fixture failure".to_owned()),
                        checks: Vec::new(),
                    },
                });
                let calls = runtime.execute_calls;

                assert!(
                    resume_frontier(&paused.configuration.run_id, &mut runtime, &mut progress,)
                        .is_err()
                );
                assert_eq!(runtime.execute_calls, calls);
            }

            #[test]
            fn terminal_resume_and_plan_drift_fail_before_model_calls() {
                let mut runtime = FakeRuntime::new();
                let mut progress = Progress {
                    durable: runtime.durable.clone(),
                    states: Vec::new(),
                };
                let terminal =
                    start_frontier(Path::new("frontier-plan.json"), &mut runtime, &mut progress)
                        .unwrap();
                let calls = runtime.execute_calls;
                assert!(
                    resume_frontier(&terminal.configuration.run_id, &mut runtime, &mut progress,)
                        .is_err()
                );
                assert_eq!(runtime.execute_calls, calls);

                let mut running = terminal;
                running.status = FrontierRunStatus::Running;
                runtime.durable.borrow_mut().state = Some(running.clone());
                runtime.plan.policy.minimum_trial_score = 1;
                assert!(
                    resume_frontier(&running.configuration.run_id, &mut runtime, &mut progress,)
                        .is_err()
                );
                assert_eq!(runtime.execute_calls, calls);
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
                        pi_version: "1".to_owned(),
                    },
                    entrants: vec![FrontierEntrant {
                        provider: "anthropic".to_owned(),
                        model: "candidate".to_owned(),
                        entry_tier: Tier::T5,
                        thinking_levels: vec!["off".to_owned()],
                        catalog_observed_at: Timestamp("2030-01-01T00:00:00+0000".to_owned()),
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
                        spending_limit_millionths_of_dollar: 15_000,
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
                        .map(|tier| (tier, tier_suite(tier)))
                        .collect(),
                }
            }

            fn tier_suite(tier: Tier) -> FrontierTierSuite {
                FrontierTierSuite {
                    group_weights_basis_points: BTreeMap::from([
                        (FrontierCaseGroup::Normal, 2_500),
                        (FrontierCaseGroup::Edge, 2_500),
                        (FrontierCaseGroup::Adversarial, 2_500),
                        (FrontierCaseGroup::Critical, 2_500),
                    ]),
                    cases: (0..30)
                        .map(|index| FrontierCaseReference {
                            artifact_path: PathBuf::from(format!("tier-{tier:?}")),
                            artifact_revision: format!("revision-{tier:?}"),
                            case: CaseId(format!("{tier:?}-case-{index:02}")),
                            group: match index % 4 {
                                0 => FrontierCaseGroup::Normal,
                                1 => FrontierCaseGroup::Edge,
                                2 => FrontierCaseGroup::Adversarial,
                                _ => FrontierCaseGroup::Critical,
                            },
                            is_confirmation: true,
                        })
                        .collect(),
                }
            }

            fn artifact(stem: &str) -> ArtifactDefinition {
                let tier = stem.trim_start_matches("tier-");
                ArtifactDefinition {
                    name: ArtifactName(stem.to_owned()),
                    kind: ArtifactKind::Skill,
                    root: PathBuf::from(stem),
                    revision: format!("revision-{tier}"),
                    required_destinations: vec![TierDestination::SkillMinimum],
                    current_tiers: Vec::new(),
                    cases: (0..30)
                        .map(|index| CaseDefinition {
                            id: CaseId(format!("{tier}-case-{index:02}")),
                            input: "input".to_owned(),
                            expect: "expect".to_owned(),
                            source: "fixture".to_owned(),
                            is_holdout: true,
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
        }
    };
}
