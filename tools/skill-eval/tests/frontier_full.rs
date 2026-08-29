#[cfg(test)]
macro_rules! frontier_full_tests {
    () => {
        mod frontier_full_tests {
            use std::collections::{BTreeMap, BTreeSet};
            use std::path::Path;

            use $crate::model::{
                ConfidenceInterval, Decision, FrontierBaselineChange, FrontierCapabilityEvidence,
                FrontierCellStatus, FrontierDecisionRequest, FrontierEvidenceIdentity,
                FrontierInspection, FrontierRunId, FrontierRunState, FrontierRunStatus,
                FrontierTrialSelector, PoolPauseReason, SkillEvalError, Tier,
            };
            use $crate::ports::FrontierProgressSink;
            use $crate::service::{
                apply_frontier_baseline, build_frontier_report, inspect_frontier,
                record_frontier_decision, resume_frontier, start_frontier,
            };
            use $crate::testing::{
                FakeFrontierAttempt, FakeFrontierAttemptKind, FakeFrontierRuntime, TemporaryRoot,
            };

            #[derive(Default)]
            struct Progress {
                states: Vec<FrontierRunState>,
            }

            impl FrontierProgressSink for Progress {
                fn emit_frontier(
                    &mut self,
                    state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    self.states.push(state.clone());
                    Ok(())
                }
            }

            #[test]
            fn deterministic_frontier_covers_the_full_lifecycle_without_external_calls() {
                let root = TemporaryRoot::new("frontier-full");
                let mut runtime = FakeFrontierRuntime::new(&root);
                let mut progress = Progress::default();
                let plan_path = runtime.plan_path().to_path_buf();

                let mixed_pause =
                    start_frontier(&plan_path, &mut runtime, &mut progress).unwrap();
                assert_eq!(mixed_pause.status, FrontierRunStatus::Paused);
                assert!(matches!(
                    mixed_pause.pause,
                    Some(PoolPauseReason::Infrastructure { .. })
                ));
                assert_eq!(mixed_pause.infrastructure_events.len(), 1);
                let infrastructure_attempt = runtime.attempts()[0].clone();
                let infrastructure =
                    inspect_frontier(&selector(&infrastructure_attempt), &runtime).unwrap();
                assert!(matches!(
                    infrastructure,
                    FrontierInspection::Infrastructure { event }
                        if event.infrastructure_attempt == 1
                ));

                let accepted_candidate = resume_frontier(
                    &mixed_pause.configuration.run_id,
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();
                assert_complete_matrix(&accepted_candidate);
                assert_eq!(accepted_candidate.status, FrontierRunStatus::AwaitingDecision);
                assert!(progress
                    .states
                    .iter()
                    .any(|state| !state.infrastructure_events.is_empty()));

                let accepted_run_id = accepted_candidate.configuration.run_id.clone();
                let accepted_trials = runtime.saved_trials(&accepted_run_id);
                assert_execution_is_exact(
                    runtime.attempts(),
                    &accepted_run_id,
                    &accepted_trials,
                    true,
                );
                assert_all_evidence_is_inspectable(&runtime, &accepted_run_id, &accepted_trials);
                let terminal_call_count = runtime.attempts().len();
                assert!(resume_frontier(&accepted_run_id, &mut runtime, &mut progress).is_err());
                assert_eq!(runtime.attempts().len(), terminal_call_count);

                let accepted = record_frontier_decision(
                    &decision(&accepted_run_id, Decision::Accepted, "owner accepted fixture"),
                    &mut runtime,
                )
                .unwrap();
                assert_eq!(accepted.status, FrontierRunStatus::Accepted);
                let first_apply = apply_frontier_baseline(&accepted_run_id, &mut runtime).unwrap();
                assert!(first_apply.is_changed);
                let applied_bytes = runtime.routing_bytes();
                let second_apply = apply_frontier_baseline(&accepted_run_id, &mut runtime).unwrap();
                assert!(!second_apply.is_changed);
                assert_eq!(second_apply.active_routes, first_apply.active_routes);
                assert_eq!(runtime.routing_bytes(), applied_bytes);

                let second_attempt_start = runtime.attempts().len();
                let rejected_candidate =
                    start_frontier(&plan_path, &mut runtime, &mut progress).unwrap();
                assert_complete_matrix(&rejected_candidate);
                assert_eq!(rejected_candidate.status, FrontierRunStatus::AwaitingDecision);
                let rejected_run_id = rejected_candidate.configuration.run_id.clone();
                let rejected_trials = runtime.saved_trials(&rejected_run_id);
                assert_execution_is_exact(
                    &runtime.attempts()[second_attempt_start..],
                    &rejected_run_id,
                    &rejected_trials,
                    false,
                );
                assert_all_evidence_is_inspectable(&runtime, &rejected_run_id, &rejected_trials);

                let compared = build_frontier_report(
                    &rejected_run_id,
                    Some(Path::new("config/model-frontier-baseline.json")),
                    &runtime,
                )
                .unwrap();
                assert!(compared.models.iter().all(|model| {
                    model.baseline_change != FrontierBaselineChange::NotCompared
                }));
                assert!(compared
                    .models
                    .iter()
                    .any(|model| model.baseline_change == FrontierBaselineChange::Unchanged));
                assert_capability_tag_is_rank_neutral(&runtime, &rejected_candidate, &compared);

                let rejected = record_frontier_decision(
                    &decision(&rejected_run_id, Decision::Rejected, "owner rejected fixture"),
                    &mut runtime,
                )
                .unwrap();
                assert_eq!(rejected.status, FrontierRunStatus::Rejected);
                assert_eq!(runtime.baseline_ledger().baselines.len(), 1);
            }

            fn assert_complete_matrix(state: &FrontierRunState) {
                let entry_tiers = state
                    .configuration
                    .plan
                    .entrants
                    .iter()
                    .map(|entrant| entrant.entry_tier)
                    .collect::<BTreeSet<_>>();
                assert_eq!(
                    entry_tiers,
                    BTreeSet::from([Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5])
                );
                assert!(state.models.iter().all(|model| model.is_exhausted));
                assert!(state
                    .cells
                    .iter()
                    .any(|cell| cell.status == FrontierCellStatus::Passed));
                assert!(state
                    .cells
                    .iter()
                    .any(|cell| cell.status == FrontierCellStatus::Failed));
                assert!(state
                    .cells
                    .iter()
                    .any(|cell| cell.status == FrontierCellStatus::Indeterminate));
                assert_eq!(state.cells.len(), 17);
            }

            fn assert_execution_is_exact(
                attempts: &[FakeFrontierAttempt],
                run_id: &FrontierRunId,
                trials: &[$crate::model::TrialRecord],
                is_interrupted: bool,
            ) {
                let terminal = trials
                    .iter()
                    .map(|trial| execution_identity(&trial.model, &trial.key))
                    .collect::<BTreeSet<_>>();
                assert_eq!(terminal.len(), trials.len(), "terminal trial key repeated");
                let completed = attempts
                    .iter()
                    .filter(|attempt| attempt.kind == FakeFrontierAttemptKind::Completed)
                    .map(|attempt| execution_identity(&attempt.model, &attempt.key))
                    .collect::<BTreeSet<_>>();
                assert_eq!(completed, terminal, "an unplanned key ran");
                assert!(attempts.iter().all(|attempt| attempt.run_id == *run_id));
                let exceptional = attempts
                    .iter()
                    .filter(|attempt| attempt.kind != FakeFrontierAttemptKind::Completed)
                    .collect::<Vec<_>>();
                if is_interrupted {
                    assert_eq!(exceptional.len(), 1);
                    assert_eq!(exceptional[0].kind, FakeFrontierAttemptKind::Infrastructure);
                    assert!(terminal.contains(&execution_identity(
                        &exceptional[0].model,
                        &exceptional[0].key,
                    )));
                } else {
                    assert!(exceptional.is_empty());
                }
            }

            fn assert_all_evidence_is_inspectable(
                runtime: &FakeFrontierRuntime,
                run_id: &FrontierRunId,
                trials: &[$crate::model::TrialRecord],
            ) {
                for trial in trials {
                    let selected = inspect_frontier(
                        &FrontierTrialSelector {
                            run_id: run_id.clone(),
                            provider: trial.model.provider.clone(),
                            model: trial.model.model.clone(),
                            tier: trial.model.tier,
                            thinking: trial.model.thinking.clone(),
                            artifact: trial.key.artifact.clone(),
                            case: trial.key.case.clone(),
                            attempt: trial.key.attempt,
                        },
                        runtime,
                    )
                    .unwrap();
                    assert_eq!(selected, FrontierInspection::Trial { trial: trial.clone() });
                }
            }

            fn assert_capability_tag_is_rank_neutral(
                runtime: &FakeFrontierRuntime,
                state: &FrontierRunState,
                compared: &$crate::model::FrontierReport,
            ) {
                let baseline = runtime.baseline_ledger().baselines[0].clone();
                let mut tagged = baseline.clone();
                let route = baseline.pools[&Tier::T1][0].model.clone();
                tagged.capabilities.push(FrontierCapabilityEvidence {
                    model: route,
                    tag: "tool-use".to_owned(),
                    capability_revision: "synthetic-capability-v1".to_owned(),
                    score: ConfidenceInterval {
                        lower: 0.0,
                        estimate: 1.0,
                        upper: 1.0,
                    },
                    evidence: FrontierEvidenceIdentity {
                        path: Path::new("fixtures/capabilities.json").to_path_buf(),
                        sha256: state.configuration.plan.capabilities.sha256.clone(),
                    },
                });
                let tagged_report = $crate::frontier_report::derive_frontier_report(
                    state,
                    &state.models,
                    &state.cells,
                    Some(&tagged),
                )
                .unwrap();
                let ranks = |report: &$crate::model::FrontierReport| {
                    report
                        .models
                        .iter()
                        .map(|model| (model.model.clone(), model.pool_memberships.clone()))
                        .collect::<BTreeMap<_, _>>()
                };
                assert_eq!(ranks(&tagged_report), ranks(compared));
            }

            fn execution_identity(
                model: &$crate::model::ModelIdentity,
                key: &$crate::model::TrialKey,
            ) -> (String, String, Tier, String, $crate::model::TrialKey) {
                (
                    model.provider.clone(),
                    model.model.clone(),
                    model.tier,
                    model.thinking.clone(),
                    key.clone(),
                )
            }

            fn selector(attempt: &FakeFrontierAttempt) -> FrontierTrialSelector {
                FrontierTrialSelector {
                    run_id: attempt.run_id.clone(),
                    provider: attempt.model.provider.clone(),
                    model: attempt.model.model.clone(),
                    tier: attempt.model.tier,
                    thinking: attempt.model.thinking.clone(),
                    artifact: attempt.key.artifact.clone(),
                    case: attempt.key.case.clone(),
                    attempt: attempt.key.attempt,
                }
            }

            fn decision(
                run_id: &FrontierRunId,
                decision: Decision,
                reason: &str,
            ) -> FrontierDecisionRequest {
                FrontierDecisionRequest {
                    run_id: run_id.clone(),
                    decision,
                    reason: reason.to_owned(),
                }
            }
        }
    };
}
