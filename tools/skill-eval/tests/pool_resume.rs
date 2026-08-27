#[macro_export]
macro_rules! pool_resume_tests {
    () => {
        mod pool_resume {
            use std::cell::{Cell, RefCell};
            use std::collections::BTreeMap;
            use std::path::{Path, PathBuf};
            use std::rc::Rc;

            use $crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, CandidateArtifact, CaseDefinition,
                CaseDrive, CaseId, CheckResult, ConfidenceInterval, ExecutionDefinition,
                HarnessIdentity, JudgeInput, JudgeResult, ModelIdentity, PoolChildRun,
                PoolChildStatus, PoolEntrant, PoolEntrantEvidence, PoolPauseReason, PoolPlan,
                PoolPolicy, PoolRunConfiguration, PoolRunId, PoolRunState, PoolRunStatus,
                PoolStage, PromptJudgeRequest, PromptJudgeResult, QualificationPolicy,
                QualificationPurpose, RankedPool, RunConfiguration, RunEvent, RunId, RunMode,
                SkillEvalError, Tier, TierAssignment, TierDestination, Timestamp, TrialKey,
                TrialRecord, TrialSelector, TrialUsage, TrialVerdict,
            };
            use $crate::ports::{
                ArtifactSource, CandidateRunner, Clock, HarnessResolver, Judge, ModelResolver,
                PoolPlanSource, PoolProgressSink, PoolRunIdSource, PoolRuntime, PoolStore,
                QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
            };

            use super::{
                add_pool_resume_spending, build_report, next_pool_child_index,
                preallocate_pool_children, requested_pool_child_model, resume_pool_qualification,
            };

            #[test]
            fn pending_child_starts_under_preallocated_id_in_stable_order() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                runtime
                    .state
                    .configuration
                    .entrants
                    .get_mut(&Tier::T2)
                    .unwrap()[1]
                    .candidate_timeout_seconds = Some(43);
                runtime.persist();
                runtime.complete_child(0);
                let expected = runtime.state.child_runs[2].run_id.clone();
                let mut progress = FakeProgress::new(runtime.persisted.clone());

                let state = resume_pool_qualification(
                    &PoolRunId("pool-1".to_owned()),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                assert_eq!(state.child_runs[2].status, PoolChildStatus::Completed);
                assert_eq!(runtime.started_ids, vec![expected]);
                assert_eq!(runtime.next_calls, 0);
                assert_eq!(runtime.executed_models, vec![model(Tier::T2, 1); 5]);
                assert_eq!(runtime.candidate_timeouts, vec![Some(43); 5]);
                assert!(progress.is_persisted_before_emit);
            }

            #[test]
            fn thinking_preallocation_is_complete_unique_and_stably_model_indexed() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                configure_thinking_levels(&mut runtime, &["low", "medium", "high"], "medium");
                let configuration = runtime.state.configuration.clone();
                let children =
                    preallocate_pool_children(&[Tier::T2], &configuration, &mut runtime).unwrap();

                assert_eq!(children.len(), 10);
                assert_eq!(runtime.next_calls, 10);
                assert_eq!(
                    children
                        .iter()
                        .map(|child| (
                            child.entrant_index,
                            child.thinking_index,
                            child.stage,
                            child.run_id.0.as_str(),
                        ))
                        .collect::<Vec<_>>(),
                    vec![
                        (0, 0, PoolStage::Calibration, "new-child-1"),
                        (0, 0, PoolStage::Qualification, "new-child-2"),
                        (0, 1, PoolStage::Calibration, "new-child-3"),
                        (0, 1, PoolStage::Qualification, "new-child-4"),
                        (0, 2, PoolStage::Calibration, "new-child-5"),
                        (0, 2, PoolStage::Qualification, "new-child-6"),
                        (1, 0, PoolStage::Calibration, "new-child-7"),
                        (1, 0, PoolStage::Qualification, "new-child-8"),
                        (2, 0, PoolStage::Calibration, "new-child-9"),
                        (2, 0, PoolStage::Qualification, "new-child-10"),
                    ]
                );
                assert_eq!(
                    children
                        .iter()
                        .map(|child| &child.run_id)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len(),
                    children.len()
                );
            }

            #[test]
            fn fixed_reasoning_model_keeps_one_calibration_and_one_qualification_slot() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                let configuration = runtime.state.configuration.clone();
                let children =
                    preallocate_pool_children(&[Tier::T2], &configuration, &mut runtime).unwrap();

                assert_eq!(children.len(), 6);
                assert!(children.iter().all(|child| child.thinking_index == 0));
                assert_eq!(next_pool_child_index(&runtime.state).unwrap(), Some(0));
            }

            #[test]
            fn restart_preserves_each_preallocated_level_and_advances_next_model() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                configure_thinking_levels(&mut runtime, &["low", "medium", "high"], "medium");
                let preallocated = runtime
                    .state
                    .child_runs
                    .iter()
                    .map(|child| child.run_id.clone())
                    .collect::<Vec<_>>();
                let mut progress = FakeProgress::new(runtime.persisted.clone());

                for expected in ["low", "medium", "high"] {
                    let prior_started = runtime.started_ids.len();
                    let state = resume_pool_qualification(
                        &PoolRunId("pool-1".to_owned()),
                        &mut runtime,
                        &mut progress,
                    )
                    .unwrap();
                    assert_eq!(runtime.executed_models.last().unwrap().thinking, expected);
                    assert_eq!(runtime.started_ids.len(), prior_started + 1);
                    assert!(state.child_runs.iter().all(|child| {
                        child.stage != PoolStage::Calibration
                            || child.entrant_index != 0
                            || child.status != PoolChildStatus::Skipped
                    }));
                }

                let selected = thinking_model(Tier::T2, 0, "low");
                assert_eq!(runtime.state.pools[0].thinking_selections, vec![selected]);
                let advanced = resume_pool_qualification(
                    &PoolRunId("pool-1".to_owned()),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();
                assert_eq!(runtime.executed_models.last(), Some(&model(Tier::T2, 1)));
                assert_eq!(
                    advanced.pools[0].thinking_selections,
                    vec![thinking_model(Tier::T2, 0, "low"), model(Tier::T2, 1)]
                );
                assert_eq!(
                    runtime
                        .state
                        .child_runs
                        .iter()
                        .map(|child| child.run_id.clone())
                        .collect::<Vec<_>>(),
                    preallocated
                );
            }

            #[test]
            fn restart_repeats_no_completed_thinking_level() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                configure_thinking_levels(&mut runtime, &["low", "medium", "high"], "medium");
                runtime.failing_thinking = Some("medium".to_owned());
                let mut progress = FakeProgress::new(runtime.persisted.clone());

                for _ in 0..3 {
                    resume_pool_qualification(
                        &PoolRunId("pool-1".to_owned()),
                        &mut runtime,
                        &mut progress,
                    )
                    .unwrap();
                }

                assert_eq!(
                    runtime
                        .executed_models
                        .iter()
                        .map(|model| model.thinking.as_str())
                        .collect::<Vec<_>>(),
                    vec![
                        "low", "low", "low", "low", "low", "medium", "medium", "medium", "medium",
                        "medium", "high", "high", "high", "high", "high",
                    ]
                );
                assert_eq!(runtime.started_ids.len(), 3);
            }

            #[test]
            fn requested_identity_substitutes_only_the_indexed_thinking_level() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                configure_thinking_levels(&mut runtime, &["low", "medium", "high"], "medium");
                let child = runtime
                    .state
                    .child_runs
                    .iter()
                    .find(|child| {
                        child.entrant_index == 0
                            && child.thinking_index == 2
                            && child.stage == PoolStage::Calibration
                    })
                    .unwrap();
                let requested = requested_pool_child_model(&runtime.state, child).unwrap();
                assert_eq!(requested.tier, Tier::T2);
                assert_eq!(requested.provider, "pool");
                assert_eq!(requested.model, "model-T2-0");
                assert_eq!(requested.thinking, "high");

                let mut drifting = child.clone();
                drifting.thinking_index = 3;
                assert!(requested_pool_child_model(&runtime.state, &drifting).is_err());
            }

            #[test]
            fn qualification_eligibility_uses_the_promoted_exact_thinking_identity() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                configure_thinking_levels(&mut runtime, &["low", "high"], "low");
                let selected = thinking_model(Tier::T2, 0, "low");
                let evidence = thinking_evidence(selected.clone(), true);
                let stronger_evidence =
                    thinking_evidence(thinking_model(Tier::T2, 0, "high"), true);
                let pool = RankedPool {
                    tier: Tier::T2,
                    calibration: vec![evidence, stronger_evidence],
                    thinking_selections: vec![selected.clone()],
                    retained_lower_routes: Vec::new(),
                    promoted: vec![selected, model(Tier::T2, 1)],
                    qualification: Vec::new(),
                    ranked: Vec::new(),
                    is_complete: false,
                };
                for child in &mut runtime.state.child_runs {
                    if child.stage == PoolStage::Calibration {
                        child.status = PoolChildStatus::Completed;
                    }
                }
                runtime.state.pools = vec![pool];

                let child_index = next_pool_child_index(&runtime.state).unwrap().unwrap();
                let child = &runtime.state.child_runs[child_index];
                assert_eq!(child.entrant_index, 0);
                assert_eq!(child.thinking_index, 0);
                assert_eq!(child.stage, PoolStage::Qualification);
                assert_eq!(runtime.state.child_runs[3].status, PoolChildStatus::Pending);
            }

            #[test]
            fn retained_lower_route_continues_stronger() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                configure_thinking_levels(&mut runtime, &["off", "medium", "high"], "medium");
                runtime
                    .state
                    .configuration
                    .entrants
                    .get_mut(&Tier::T2)
                    .unwrap()[0]
                    .retained_lower_thinking_level = Some("off".to_owned());
                for child in &mut runtime.state.child_runs {
                    if child.stage == PoolStage::Calibration {
                        child.status = PoolChildStatus::Completed;
                    }
                }
                let mut lower_final = thinking_evidence(thinking_model(Tier::T2, 0, "off"), true);
                lower_final.stage = PoolStage::Qualification;
                lower_final.completed_trials = 15;
                lower_final.expected_trials = 15;
                let calibration = vec![
                    thinking_evidence(thinking_model(Tier::T2, 0, "off"), true),
                    thinking_evidence(thinking_model(Tier::T2, 0, "medium"), true),
                    thinking_evidence(thinking_model(Tier::T2, 0, "high"), true),
                    thinking_evidence(model(Tier::T2, 1), true),
                    thinking_evidence(model(Tier::T2, 2), true),
                ];
                runtime.state.pools.push(RankedPool {
                    tier: Tier::T2,
                    calibration,
                    thinking_selections: vec![
                        thinking_model(Tier::T2, 0, "off"),
                        model(Tier::T2, 1),
                        model(Tier::T2, 2),
                    ],
                    retained_lower_routes: vec![thinking_model(Tier::T2, 0, "off")],
                    promoted: vec![
                        thinking_model(Tier::T2, 0, "medium"),
                        model(Tier::T2, 1),
                        model(Tier::T2, 2),
                    ],
                    qualification: vec![lower_final],
                    ranked: Vec::new(),
                    is_complete: false,
                });
                let lower_child = runtime
                    .state
                    .child_runs
                    .iter_mut()
                    .find(|child| {
                        child.entrant_index == 0
                            && child.thinking_index == 0
                            && child.stage == PoolStage::Qualification
                    })
                    .unwrap();
                lower_child.status = PoolChildStatus::Completed;
                let lower_run_id = lower_child.run_id.clone();

                let next = next_pool_child_index(&runtime.state).unwrap().unwrap();
                assert_eq!(runtime.state.child_runs[next].entrant_index, 0);
                assert_eq!(runtime.state.child_runs[next].thinking_index, 1);
                assert_eq!(
                    runtime.state.child_runs[next].stage,
                    PoolStage::Qualification
                );
                assert_ne!(runtime.state.child_runs[next].run_id, lower_run_id);

                runtime.state.child_runs[next].status = PoolChildStatus::Completed;
                let mut medium_final =
                    thinking_evidence(thinking_model(Tier::T2, 0, "medium"), false);
                medium_final.stage = PoolStage::Qualification;
                medium_final.completed_trials = 15;
                medium_final.expected_trials = 15;
                runtime.state.pools[0].qualification.push(medium_final);
                let high = next_pool_child_index(&runtime.state).unwrap().unwrap();
                assert_eq!(runtime.state.child_runs[high].entrant_index, 0);
                assert_eq!(runtime.state.child_runs[high].thinking_index, 2);
                assert_eq!(
                    runtime.state.child_runs[high].stage,
                    PoolStage::Qualification
                );

                runtime.state.child_runs[high].status = PoolChildStatus::Completed;
                let mut high_final = thinking_evidence(thinking_model(Tier::T2, 0, "high"), true);
                high_final.stage = PoolStage::Qualification;
                high_final.completed_trials = 15;
                high_final.expected_trials = 15;
                runtime.state.pools[0].qualification.push(high_final);
                let next_entrant = next_pool_child_index(&runtime.state).unwrap().unwrap();
                assert_eq!(runtime.state.child_runs[next_entrant].entrant_index, 1);
                assert_eq!(
                    runtime.state.child_runs[next_entrant].stage,
                    PoolStage::Qualification
                );
            }

            #[test]
            fn running_child_continues_each_persisted_boundary_without_duplicate_work() {
                for boundary in [
                    Boundary::RunStarted,
                    Boundary::Discovery,
                    Boundary::Started,
                    Boundary::Candidate,
                    Boundary::Trial,
                ] {
                    let mut runtime = FakeRuntime::new(PoolChildStatus::Running);
                    runtime.seed_child(boundary, false);
                    let prior_execute = runtime.execute_calls;
                    let prior_grade = runtime.grade_calls;
                    let mut progress = FakeProgress::new(runtime.persisted.clone());

                    let state = resume_pool_qualification(
                        &PoolRunId("pool-1".to_owned()),
                        &mut runtime,
                        &mut progress,
                    )
                    .unwrap();

                    assert_eq!(state.child_runs[0].status, PoolChildStatus::Completed);
                    assert_eq!(runtime.next_calls, 0);
                    assert_eq!(runtime.events(&RunId("child-c0".to_owned())).len(), 18);
                    assert_eq!(
                        runtime.execute_calls - prior_execute,
                        match boundary {
                            Boundary::RunStarted | Boundary::Discovery | Boundary::Started => 5,
                            Boundary::Candidate | Boundary::Trial => 4,
                            Boundary::Completion => unreachable!(),
                        }
                    );
                    assert_eq!(
                        runtime.grade_calls - prior_grade,
                        match boundary {
                            Boundary::RunStarted
                            | Boundary::Discovery
                            | Boundary::Started
                            | Boundary::Candidate => 5,
                            Boundary::Trial => 4,
                            Boundary::Completion => unreachable!(),
                        }
                    );
                    assert!(progress.is_persisted_before_emit);
                }
            }

            #[test]
            fn paused_child_saves_legal_resume_transitions_before_child_resume() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Paused);
                runtime.state.status = PoolRunStatus::Paused;
                runtime.state.pause = Some(PoolPauseReason::Infrastructure {
                    message: "stopped".to_owned(),
                });
                runtime.seed_child(Boundary::Candidate, true);
                runtime.persist();
                let mut progress = FakeProgress::new(runtime.persisted.clone());

                let state = resume_pool_qualification(
                    &PoolRunId("pool-1".to_owned()),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                assert_eq!(state.status, PoolRunStatus::Running);
                assert_eq!(state.child_runs[0].status, PoolChildStatus::Completed);
                let statuses = progress
                    .states
                    .iter()
                    .map(|state| (state.status, state.child_runs[0].status))
                    .collect::<Vec<_>>();
                assert_eq!(
                    &statuses[..2],
                    &[
                        (PoolRunStatus::Running, PoolChildStatus::Paused),
                        (PoolRunStatus::Running, PoolChildStatus::Running),
                    ]
                );
                assert!(matches!(
                    runtime.events(&RunId("child-c0".to_owned()))[5],
                    RunEvent::RunResumed { .. }
                ));
                assert!(progress.is_persisted_before_emit);
            }

            #[test]
            fn resume_recovers_between_parent_and_child_status_saves() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Paused);
                runtime.seed_child(Boundary::Candidate, true);
                runtime.persist();
                let mut progress = FakeProgress::new(runtime.persisted.clone());

                let state = resume_pool_qualification(
                    &PoolRunId("pool-1".to_owned()),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                assert_eq!(state.status, PoolRunStatus::Running);
                assert_eq!(state.child_runs[0].status, PoolChildStatus::Completed);
                assert_eq!(
                    progress.states[0].child_runs[0].status,
                    PoolChildStatus::Running
                );
                assert!(progress.is_persisted_before_emit);
            }

            #[test]
            fn completed_children_are_skipped_and_only_one_child_runs() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                runtime.complete_child(0);
                runtime.complete_child(2);
                let expected = runtime.state.child_runs[4].run_id.clone();
                let untouched = runtime.state.child_runs[1].run_id.clone();
                let mut progress = FakeProgress::new(runtime.persisted.clone());

                let state = resume_pool_qualification(
                    &PoolRunId("pool-1".to_owned()),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                assert_eq!(runtime.started_ids, vec![expected]);
                assert!(!runtime.runs.contains_key(&untouched));
                assert_eq!(
                    state
                        .child_runs
                        .iter()
                        .filter(|child| child.status == PoolChildStatus::Completed)
                        .count(),
                    3
                );
            }

            #[test]
            fn qualification_waits_for_promotion_without_starting_a_child() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                for index in [0, 2, 4] {
                    runtime.complete_child(index);
                }
                let before = runtime.state.clone();

                let state = resume_pool_qualification(
                    &PoolRunId("pool-1".to_owned()),
                    &mut runtime,
                    &mut FakeProgress::new_detached(),
                )
                .unwrap();

                assert_eq!(state, before);
                assert!(runtime.started_ids.is_empty());
                assert_eq!(runtime.child_calls(), 0);
            }

            #[test]
            fn skipped_qualification_child_is_terminal_and_never_runs() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                for index in [0, 2, 4] {
                    runtime.complete_child(index);
                }
                runtime.state.child_runs[1].status = PoolChildStatus::Skipped;
                runtime.state.pools.push(RankedPool {
                    tier: Tier::T2,
                    calibration: vec![
                        thinking_evidence(model(Tier::T2, 0), false),
                        thinking_evidence(model(Tier::T2, 1), true),
                        thinking_evidence(model(Tier::T2, 2), true),
                    ],
                    thinking_selections: vec![model(Tier::T2, 1), model(Tier::T2, 2)],
                    retained_lower_routes: Vec::new(),
                    promoted: vec![model(Tier::T2, 1), model(Tier::T2, 2)],
                    qualification: Vec::new(),
                    ranked: Vec::new(),
                    is_complete: false,
                });
                runtime.persist();
                let skipped = runtime.state.child_runs[1].run_id.clone();
                let expected = runtime.state.child_runs[3].run_id.clone();
                let persisted = runtime.persisted.clone();

                let state = resume_pool_qualification(
                    &PoolRunId("pool-1".to_owned()),
                    &mut runtime,
                    &mut FakeProgress::new(persisted),
                )
                .unwrap();

                assert_eq!(runtime.started_ids, vec![expected]);
                assert!(!runtime.runs.contains_key(&skipped));
                assert_eq!(state.child_runs[1].status, PoolChildStatus::Skipped);
            }

            #[test]
            fn artifact_definition_drift_rejects_before_model_or_child_calls() {
                for drift in [
                    "root",
                    "revision",
                    "case",
                    "support",
                    "execution",
                    "destination",
                ] {
                    let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                    match drift {
                        "root" => runtime.artifact.root = PathBuf::from("other"),
                        "revision" => runtime.artifact.revision = "changed".to_owned(),
                        "case" => runtime.artifact.cases[0].expect = "changed".to_owned(),
                        "support" => runtime.artifact.cases[0]
                            .support_files
                            .push(PathBuf::from("support.txt")),
                        "execution" => runtime.artifact.cases[0].execution.timeout_seconds = 99,
                        "destination" => runtime
                            .artifact
                            .required_destinations
                            .push(TierDestination::SkillTarget),
                        _ => unreachable!(),
                    }
                    let calls = runtime.child_calls();

                    let error = resume_pool_qualification(
                        &PoolRunId("pool-1".to_owned()),
                        &mut runtime,
                        &mut FakeProgress::new_detached(),
                    )
                    .unwrap_err();

                    assert!(
                        matches!(error, SkillEvalError::InvalidConfiguration(_)),
                        "{drift}"
                    );
                    assert_eq!(runtime.child_calls(), calls, "{drift}");
                }
            }

            #[test]
            fn model_plan_child_harness_and_judge_drift_fail_before_resume_work() {
                for drift in ["plan", "child", "model", "harness", "judge"] {
                    let mut runtime = FakeRuntime::new(PoolChildStatus::Running);
                    runtime.seed_child(Boundary::Trial, false);
                    match drift {
                        "plan" => runtime.state.configuration.policy.minimum_score = 6,
                        "child" => {
                            runtime.state.child_runs[0].run_id = RunId("replacement".to_owned())
                        }
                        "model" => runtime.exact_provider = "changed".to_owned(),
                        "harness" => runtime.harness_version = "changed".to_owned(),
                        "judge" => runtime.judge_model = "changed".to_owned(),
                        _ => unreachable!(),
                    }
                    runtime.persist();
                    let execute = runtime.execute_calls;
                    let grade = runtime.grade_calls;

                    let error = resume_pool_qualification(
                        &PoolRunId("pool-1".to_owned()),
                        &mut runtime,
                        &mut FakeProgress::new_detached(),
                    )
                    .unwrap_err();

                    assert!(
                        matches!(
                            error,
                            SkillEvalError::InvalidConfiguration(_) | SkillEvalError::NotFound(_)
                        ),
                        "{drift}"
                    );
                    assert_eq!(runtime.execute_calls, execute, "{drift}");
                    assert_eq!(runtime.grade_calls, grade, "{drift}");
                }
            }

            #[test]
            fn repeated_quota_pause_keeps_checkpoints_and_counts_only_usage_delta() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Paused);
                runtime.state.status = PoolRunStatus::Paused;
                runtime.state.pause = Some(PoolPauseReason::Quota {
                    model: model(Tier::T2, 0),
                    reset_at: Some(now()),
                });
                runtime.state.spent_millionths_of_dollar = 5;
                runtime.seed_child(Boundary::Candidate, true);
                runtime.quota_grades = 2;
                runtime.persist();

                let mut progress = FakeProgress::new(runtime.persisted.clone());
                let first = resume_pool_qualification(
                    &PoolRunId("pool-1".to_owned()),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();
                assert_eq!(first.status, PoolRunStatus::Paused);
                assert_eq!(first.spent_millionths_of_dollar, 5);
                assert_eq!(runtime.execute_calls, 0);

                let mut progress = FakeProgress::new(runtime.persisted.clone());
                let second = resume_pool_qualification(
                    &PoolRunId("pool-1".to_owned()),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();
                assert_eq!(second.status, PoolRunStatus::Paused);
                assert_eq!(second.spent_millionths_of_dollar, 5);
                assert_eq!(runtime.execute_calls, 0);

                let mut progress = FakeProgress::new(runtime.persisted.clone());
                let completed = resume_pool_qualification(
                    &PoolRunId("pool-1".to_owned()),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();
                assert_eq!(completed.child_runs[0].status, PoolChildStatus::Completed);
                assert_eq!(completed.spent_millionths_of_dollar, 17);
                assert_eq!(runtime.execute_calls, 4);
                assert_eq!(runtime.grade_calls, 7);
            }

            #[test]
            fn spending_delta_rejects_decreasing_totals_and_overflow() {
                let mut runtime = FakeRuntime::new(PoolChildStatus::Running);
                runtime.seed_child(Boundary::Trial, false);
                let report = build_report(&RunId("child-c0".to_owned()), &runtime).unwrap();
                let mut progress = FakeProgress::new(runtime.persisted.clone());

                let mut decreasing = runtime.state.clone();
                assert!(
                    add_pool_resume_spending(
                        &mut decreasing,
                        report.total_usage.cost_millionths_of_dollar + 1,
                        &report,
                        &mut runtime,
                        &mut progress,
                    )
                    .is_err()
                );

                let mut overflowing = runtime.state.clone();
                overflowing.spent_millionths_of_dollar = u64::MAX;
                assert!(
                    add_pool_resume_spending(
                        &mut overflowing,
                        0,
                        &report,
                        &mut runtime,
                        &mut progress,
                    )
                    .is_err()
                );
            }

            #[test]
            fn terminal_and_malformed_states_are_rejected_without_child_work() {
                for status in [
                    PoolRunStatus::Pending,
                    PoolRunStatus::AwaitingDecision,
                    PoolRunStatus::Completed,
                    PoolRunStatus::Failed,
                ] {
                    let mut runtime = FakeRuntime::new(PoolChildStatus::Pending);
                    runtime.state.status = status;
                    runtime.persist();
                    let calls = runtime.child_calls();
                    assert!(
                        resume_pool_qualification(
                            &PoolRunId("pool-1".to_owned()),
                            &mut runtime,
                            &mut FakeProgress::new_detached(),
                        )
                        .is_err()
                    );
                    assert_eq!(runtime.child_calls(), calls);
                }

                let mut failed = FakeRuntime::new(PoolChildStatus::Failed);
                assert!(
                    resume_pool_qualification(
                        &PoolRunId("pool-1".to_owned()),
                        &mut failed,
                        &mut FakeProgress::new_detached(),
                    )
                    .is_err()
                );
                assert_eq!(failed.child_calls(), 0);
            }

            #[derive(Clone, Copy, Eq, PartialEq)]
            enum Boundary {
                RunStarted,
                Discovery,
                Started,
                Candidate,
                Trial,
                Completion,
            }

            struct FakeRuntime {
                artifact: ArtifactDefinition,
                state: PoolRunState,
                persisted: Rc<RefCell<PoolRunState>>,
                runs: BTreeMap<RunId, Vec<RunEvent>>,
                next_calls: usize,
                exact_calls: Cell<usize>,
                resolver_calls: Cell<usize>,
                execute_calls: usize,
                grade_calls: usize,
                started_ids: Vec<RunId>,
                executed_models: Vec<ModelIdentity>,
                candidate_timeouts: Vec<Option<u32>>,
                exact_provider: String,
                harness_version: String,
                judge_model: String,
                quota_grades: usize,
                failing_thinking: Option<String>,
            }

            impl FakeRuntime {
                fn new(first_status: PoolChildStatus) -> Self {
                    let artifact = artifact();
                    let mut state = pool_state(artifact.clone());
                    state.child_runs[0].status = first_status;
                    let persisted = Rc::new(RefCell::new(state.clone()));
                    Self {
                        artifact,
                        state,
                        persisted,
                        runs: BTreeMap::new(),
                        next_calls: 0,
                        exact_calls: Cell::new(0),
                        resolver_calls: Cell::new(0),
                        execute_calls: 0,
                        grade_calls: 0,
                        started_ids: Vec::new(),
                        executed_models: Vec::new(),
                        candidate_timeouts: Vec::new(),
                        exact_provider: "pool".to_owned(),
                        harness_version: "runner-1".to_owned(),
                        judge_model: "judge-1".to_owned(),
                        quota_grades: 0,
                        failing_thinking: None,
                    }
                }

                fn persist(&mut self) {
                    *self.persisted.borrow_mut() = self.state.clone();
                }

                fn events(&self, run_id: &RunId) -> Vec<RunEvent> {
                    self.runs.get(run_id).cloned().unwrap_or_default()
                }

                fn child_calls(&self) -> usize {
                    self.exact_calls.get()
                        + self.resolver_calls.get()
                        + self.execute_calls
                        + self.grade_calls
                        + self.started_ids.len()
                }

                fn complete_child(&mut self, index: usize) {
                    let prior = self.state.child_runs[index].status;
                    self.state.child_runs[index].status = PoolChildStatus::Running;
                    let child = self.state.child_runs[index].clone();
                    self.seed_events(&child, Boundary::Trial, false);
                    self.state.child_runs[index].status = PoolChildStatus::Completed;
                    if prior == PoolChildStatus::Pending {
                        self.persist();
                    }
                }

                fn seed_child(&mut self, boundary: Boundary, is_paused: bool) {
                    let child = self.state.child_runs[0].clone();
                    self.seed_events(&child, boundary, is_paused);
                }

                fn seed_events(
                    &mut self,
                    child: &PoolChildRun,
                    boundary: Boundary,
                    is_paused: bool,
                ) {
                    let candidate_model = model(child.tier, usize::from(child.entrant_index));
                    let configuration = child_configuration(
                        child,
                        self.artifact.clone(),
                        self.state.configuration.policy.minimum_score,
                    );
                    let key = TrialKey {
                        artifact: self.artifact.name.clone(),
                        tier: child.tier,
                        route_index: 0,
                        case: self.artifact.cases[0].id.clone(),
                        attempt: 1,
                    };
                    let harness = harness(&self.artifact, &self.harness_version);
                    let candidate = candidate(&child.run_id, &key, &candidate_model, &harness);
                    let mut events = vec![RunEvent::RunStarted {
                        at: now(),
                        configuration,
                    }];
                    if boundary != Boundary::RunStarted {
                        events.push(RunEvent::DiscoveryCompleted {
                            at: now(),
                            artifacts: vec![$crate::model::ArtifactDiscovery {
                                artifact: self.artifact.name.clone(),
                                kind: self.artifact.kind,
                                revision: self.artifact.revision.clone(),
                                cases: self
                                    .artifact
                                    .cases
                                    .iter()
                                    .map(|case| $crate::model::CaseDiscovery {
                                        id: case.id.clone(),
                                        drive: case.execution.drive.clone(),
                                        is_holdout: false,
                                    })
                                    .collect(),
                            }],
                        });
                    }
                    if matches!(
                        boundary,
                        Boundary::Started
                            | Boundary::Candidate
                            | Boundary::Trial
                            | Boundary::Completion
                    ) {
                        events.push(RunEvent::TrialStarted {
                            at: now(),
                            key: key.clone(),
                            models: vec![candidate_model.clone()],
                            harness: harness.clone(),
                        });
                    }
                    if matches!(
                        boundary,
                        Boundary::Candidate | Boundary::Trial | Boundary::Completion
                    ) {
                        events.push(RunEvent::CandidateExecuted {
                            at: now(),
                            candidate: candidate.clone(),
                        });
                    }
                    if matches!(boundary, Boundary::Trial | Boundary::Completion) {
                        events.push(RunEvent::TrialCompleted {
                            at: now(),
                            record: record(candidate, judge()),
                        });
                    }
                    if boundary == Boundary::Completion {
                        events.push(RunEvent::PoolChildCompleted {
                            at: now(),
                            artifact: self.artifact.name.clone(),
                            tier: child.tier,
                        });
                    }
                    if is_paused {
                        events.push(RunEvent::RunPaused {
                            at: now(),
                            reason: $crate::model::PauseReason::Infrastructure {
                                message: "stopped".to_owned(),
                            },
                        });
                    }
                    self.runs.insert(child.run_id.clone(), events);
                }
            }

            impl ArtifactSource for FakeRuntime {
                fn load(&self, _root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
                    Ok(self.artifact.clone())
                }
            }

            impl ModelResolver for FakeRuntime {
                fn candidates(&self, _tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    self.resolver_calls.set(self.resolver_calls.get() + 1);
                    unreachable!()
                }

                fn qualification_routes(
                    &self,
                    _tier: Tier,
                ) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    Err(SkillEvalError::InvalidConfiguration(
                        "artifact qualification routes are unavailable in pool tests".to_owned(),
                    ))
                }

                fn exact_candidate(
                    &self,
                    requested: &ModelIdentity,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    self.exact_calls.set(self.exact_calls.get() + 1);
                    let mut resolved = requested.clone();
                    resolved.provider = self.exact_provider.clone();
                    Ok(resolved)
                }

                fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
                    self.resolver_calls.set(self.resolver_calls.get() + 1);
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
                    self.resolver_calls.set(self.resolver_calls.get() + 1);
                    let mut current = judge();
                    current.model = self.judge_model.clone();
                    Ok(current)
                }
            }

            impl HarnessResolver for FakeRuntime {
                fn identity(
                    &self,
                    artifact: &ArtifactDefinition,
                    _execution: &ExecutionDefinition,
                ) -> Result<HarnessIdentity, SkillEvalError> {
                    Ok(harness(artifact, &self.harness_version))
                }
            }

            impl RunIdSource for FakeRuntime {
                fn next(&mut self) -> Result<RunId, SkillEvalError> {
                    self.next_calls += 1;
                    Ok(RunId(format!("new-child-{}", self.next_calls)))
                }
            }

            impl PoolRunIdSource for FakeRuntime {
                fn next_pool(&mut self) -> Result<PoolRunId, SkillEvalError> {
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
                    self.executed_models.push(model.clone());
                    self.candidate_timeouts.push(candidate_timeout_seconds);
                    Ok(candidate(run_id, key, model, harness))
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
                    self.grade_calls += 1;
                    if self.quota_grades > 0 {
                        self.quota_grades -= 1;
                        return Err(SkillEvalError::Quota {
                            model: model.clone(),
                            reset_at: Some(now()),
                        });
                    }
                    let mut verdict = verdict();
                    if self.failing_thinking.as_ref() == Some(&input.candidate.model.thinking) {
                        verdict.score = 1;
                    }
                    Ok(JudgeResult {
                        verdict,
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
                        self.started_ids.push(run_id.clone());
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
                        SkillEvalError::NotFound(format!("missing child {}", run_id.0))
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
                    unreachable!()
                }

                fn validate_pool_plan_freshness(
                    &self,
                    _plan: &PoolPlan,
                    _now: &Timestamp,
                ) -> Result<(), SkillEvalError> {
                    unreachable!()
                }
            }

            impl PoolStore for FakeRuntime {
                fn create_pool(&mut self, _state: &PoolRunState) -> Result<(), SkillEvalError> {
                    unreachable!()
                }

                fn load_pool(&self, _run_id: &PoolRunId) -> Result<PoolRunState, SkillEvalError> {
                    Ok(self.persisted.borrow().clone())
                }

                fn save_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
                    self.state = state.clone();
                    *self.persisted.borrow_mut() = state.clone();
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

            struct FakeProgress {
                persisted: Option<Rc<RefCell<PoolRunState>>>,
                states: Vec<PoolRunState>,
                is_persisted_before_emit: bool,
            }

            impl FakeProgress {
                fn new(persisted: Rc<RefCell<PoolRunState>>) -> Self {
                    Self {
                        persisted: Some(persisted),
                        states: Vec::new(),
                        is_persisted_before_emit: true,
                    }
                }

                fn new_detached() -> Self {
                    Self {
                        persisted: None,
                        states: Vec::new(),
                        is_persisted_before_emit: true,
                    }
                }
            }

            impl PoolProgressSink for FakeProgress {
                fn emit_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
                    if let Some(persisted) = &self.persisted {
                        self.is_persisted_before_emit &= *persisted.borrow() == *state;
                    }
                    self.states.push(state.clone());
                    Ok(())
                }
            }

            fn pool_state(artifact: ArtifactDefinition) -> PoolRunState {
                let entrants = [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5]
                    .into_iter()
                    .map(|tier| {
                        (
                            tier,
                            (0..3)
                                .map(|index| {
                                    let model = model(tier, index);
                                    PoolEntrant {
                                        thinking_levels: vec![model.thinking.clone()],
                                        retained_lower_thinking_level: None,
                                        model,
                                        candidate_timeout_seconds: None,
                                        catalog_observed_at: now(),
                                    }
                                })
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                let child_runs = (0_u8..3)
                    .flat_map(|entrant_index| {
                        [PoolStage::Calibration, PoolStage::Qualification]
                            .into_iter()
                            .map(move |stage| PoolChildRun {
                                tier: Tier::T2,
                                entrant_index,
                                thinking_index: 0,
                                stage,
                                run_id: RunId(format!(
                                    "child-{}{}",
                                    match stage {
                                        PoolStage::Calibration => "c",
                                        PoolStage::Qualification => "q",
                                    },
                                    entrant_index
                                )),
                                status: PoolChildStatus::Pending,
                            })
                    })
                    .collect();
                PoolRunState {
                    configuration: PoolRunConfiguration {
                        run_id: PoolRunId("pool-1".to_owned()),
                        created_at: now(),
                        artifacts: vec![artifact],
                        entrants,
                        control: model(Tier::T1, 9),
                        policy: PoolPolicy {
                            calibration_repeats_per_case: 1,
                            qualification_repeats_per_case: 3,
                            promotion_count: 2,
                            minimum_score: 7,
                            calibration_minimum_reliability_basis_points: 8_000,
                            qualification_minimum_reliability_basis_points: 10_000,
                            maximum_catalog_age_seconds: 3_600,
                            spending_limit_millionths_of_dollar: 10_000,
                            is_provider_limit_enforced: true,
                        },
                    },
                    selected_tiers: vec![Tier::T2],
                    status: PoolRunStatus::Running,
                    child_runs,
                    pools: Vec::new(),
                    pause: None,
                    spent_millionths_of_dollar: 0,
                }
            }

            fn configure_thinking_levels(runtime: &mut FakeRuntime, levels: &[&str], start: &str) {
                let entrant = &mut runtime
                    .state
                    .configuration
                    .entrants
                    .get_mut(&Tier::T2)
                    .unwrap()[0];
                entrant.thinking_levels = levels.iter().map(ToString::to_string).collect();
                entrant.model.thinking = start.to_owned();
                runtime.state.child_runs = runtime.state.configuration.entrants[&Tier::T2]
                    .iter()
                    .enumerate()
                    .flat_map(|(entrant_index, entrant)| {
                        (0..entrant.thinking_levels.len()).flat_map(move |thinking_index| {
                            [PoolStage::Calibration, PoolStage::Qualification]
                                .into_iter()
                                .map(move |stage| PoolChildRun {
                                    tier: Tier::T2,
                                    entrant_index: u8::try_from(entrant_index).unwrap(),
                                    thinking_index: u8::try_from(thinking_index).unwrap(),
                                    stage,
                                    run_id: RunId(format!(
                                        "child-{}-{entrant_index}-{thinking_index}",
                                        match stage {
                                            PoolStage::Calibration => "c",
                                            PoolStage::Qualification => "q",
                                        }
                                    )),
                                    status: PoolChildStatus::Pending,
                                })
                        })
                    })
                    .collect();
                runtime.state.pools.clear();
                runtime.runs.clear();
                runtime.persist();
            }

            fn thinking_model(tier: Tier, index: usize, thinking: &str) -> ModelIdentity {
                let mut identity = model(tier, index);
                identity.thinking = thinking.to_owned();
                identity
            }

            fn thinking_evidence(
                requested_model: ModelIdentity,
                is_passing: bool,
            ) -> PoolEntrantEvidence {
                let candidate_usage = usage(3);
                let judge_usage = usage(4);
                PoolEntrantEvidence {
                    stage: PoolStage::Calibration,
                    requested_model: requested_model.clone(),
                    effective_model: requested_model,
                    judge_model: judge(),
                    harnesses: thinking_harnesses(),
                    is_passing,
                    completed_trials: 5,
                    expected_trials: 5,
                    failed_trials: u32::from(!is_passing),
                    catastrophic_trials: 0,
                    score: ConfidenceInterval {
                        lower: if is_passing { 0.8 } else { 0.1 },
                        estimate: if is_passing { 0.9 } else { 0.1 },
                        upper: if is_passing { 1.0 } else { 0.2 },
                    },
                    total_usage: TrialUsage {
                        input_tokens: 2,
                        output_tokens: 2,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                        turns: 2,
                        tool_calls: 0,
                        elapsed_milliseconds: 2,
                        cost_millionths_of_dollar: 7,
                    },
                    candidate_usage,
                    judge_usage,
                }
            }

            fn thinking_harnesses() -> Vec<HarnessIdentity> {
                (1..=5)
                    .map(|index| HarnessIdentity {
                        runner_version: "runner-1".to_owned(),
                        pi_version: "pi-1".to_owned(),
                        artifact_revision: "revision-1".to_owned(),
                        tool_policy_digest: format!("case-{index}"),
                    })
                    .collect()
            }

            fn child_configuration(
                child: &PoolChildRun,
                artifact: ArtifactDefinition,
                minimum_score: u8,
            ) -> RunConfiguration {
                RunConfiguration {
                    run_id: child.run_id.clone(),
                    mode: RunMode::Execute,
                    artifacts: vec![artifact],
                    change: None,
                    policy: QualificationPolicy {
                        purpose: QualificationPurpose::ModelPool,
                        candidate_tiers: vec![child.tier],
                        reference_tier: Tier::T1,
                        judge_tier: Tier::T5,
                        repeats_per_case: match child.stage {
                            PoolStage::Calibration => 1,
                            PoolStage::Qualification => 3,
                        },
                        minimum_score,
                        noninferiority_margin: 0.0,
                        confidence_level: 0.95,
                    },
                    qualification_routes: Default::default(),
                    created_at: now(),
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
                    cases: (1..=5)
                        .map(|index| CaseDefinition {
                            id: CaseId(format!("case-{index}")),
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
                        })
                        .collect(),
                }
            }

            fn model(tier: Tier, index: usize) -> ModelIdentity {
                ModelIdentity {
                    tier,
                    provider: "pool".to_owned(),
                    model: format!("model-{tier:?}-{index}"),
                    thinking: "medium".to_owned(),
                }
            }

            fn judge() -> ModelIdentity {
                ModelIdentity {
                    tier: Tier::T5,
                    provider: "judge".to_owned(),
                    model: "judge-1".to_owned(),
                    thinking: "high".to_owned(),
                }
            }

            fn harness(artifact: &ArtifactDefinition, runner: &str) -> HarnessIdentity {
                HarnessIdentity {
                    runner_version: runner.to_owned(),
                    pi_version: "pi-1".to_owned(),
                    artifact_revision: artifact.revision.clone(),
                    tool_policy_digest: "tools-1".to_owned(),
                }
            }

            fn candidate(
                run_id: &RunId,
                key: &TrialKey,
                model: &ModelIdentity,
                harness: &HarnessIdentity,
            ) -> CandidateArtifact {
                CandidateArtifact {
                    key: key.clone(),
                    model: model.clone(),
                    harness: harness.clone(),
                    artifact_path: PathBuf::from(&run_id.0).join("artifact.txt"),
                    transcript_path: PathBuf::from(&run_id.0).join("transcript.jsonl"),
                    usage: usage(3),
                }
            }

            fn record(candidate: CandidateArtifact, judge_model: ModelIdentity) -> TrialRecord {
                TrialRecord {
                    key: candidate.key,
                    model: candidate.model,
                    harness: candidate.harness,
                    artifact_path: candidate.artifact_path,
                    transcript_path: candidate.transcript_path,
                    candidate_usage: candidate.usage,
                    judge_model,
                    judge_usage: usage(4),
                    verdict: verdict(),
                }
            }

            fn verdict() -> TrialVerdict {
                TrialVerdict {
                    score: 9,
                    is_catastrophic: false,
                    failure_mode: None,
                    checks: Vec::new(),
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

            fn now() -> Timestamp {
                Timestamp("2026-08-25T12:00:00-0400".to_owned())
            }
        }
    };
}
