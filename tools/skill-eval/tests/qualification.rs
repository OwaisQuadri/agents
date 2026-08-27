#[macro_export]
macro_rules! qualification_harness_tests {
    () => {
        mod deterministic_qualification_harness {
            use std::fs;

            use super::{FakeQualificationRuntime, ScriptedOutcome, TemporaryRoot};
            use $crate::cli::execute_command;
            use $crate::model::{
                ArtifactChange, ArtifactName, ArtifactStatus, AuditBriefRequest, CaseId, CliCommand,
                CliRequest, Decision, EvidenceRole, OutputFormat, OwnEvalEvidence, PauseReason,
                ModelIdentity, PromptJudgeRequest, QualificationPolicy, QualificationPurpose,
                QualifyRequest, RunEvent, RunId, RunStatus, SkillEvalError, Tier, TierAssignment,
                TierDestination,
                TrialSelector,
            };
            use $crate::ports::{HarnessResolver, RunStore};
            use $crate::service::build_report;

            #[test]
            fn cheapest_first_staircase_scripts_pass_fail_and_catastrophic_results() {
                let pass_root = TemporaryRoot::new("pass");
                let mut pass_runtime = FakeQualificationRuntime::new(&pass_root);
                run_qualify(
                    &pass_root,
                    &mut pass_runtime,
                    vec![Tier::T2, Tier::T1, Tier::T3],
                    None,
                )
                .unwrap();
                let pass = build_report(&RunId("fake-run-0001".to_owned()), &pass_runtime).unwrap();
                let pass_boundary = pass.artifacts[0].boundary.as_ref().unwrap();
                assert_eq!(pass_boundary.accepted.tier, Tier::T1);
                assert_eq!(pass_boundary.failing, None);
                assert_eq!(candidate_tiers(&pass_runtime, &pass.run_id), vec![Tier::T2, Tier::T1]);

                let fail_root = TemporaryRoot::new("fail");
                let mut fail_runtime = FakeQualificationRuntime::new(&fail_root);
                fail_runtime.script(Tier::T2, [ScriptedOutcome::Fail]);
                run_qualify(
                    &fail_root,
                    &mut fail_runtime,
                    vec![Tier::T2, Tier::T3],
                    None,
                )
                .unwrap();
                let fail = build_report(&RunId("fake-run-0001".to_owned()), &fail_runtime).unwrap();
                let fail_boundary = fail.artifacts[0].boundary.as_ref().unwrap();
                assert_eq!(fail_boundary.failing.as_ref().unwrap().tier, Tier::T2);
                assert_eq!(fail_boundary.accepted.tier, Tier::T3);
                assert_eq!(candidate_tiers(&fail_runtime, &fail.run_id), vec![Tier::T2, Tier::T3]);

                let catastrophic_root = TemporaryRoot::new("catastrophic");
                let mut catastrophic_runtime = FakeQualificationRuntime::new(&catastrophic_root);
                catastrophic_runtime.script(Tier::T2, [ScriptedOutcome::Catastrophic]);
                run_qualify(
                    &catastrophic_root,
                    &mut catastrophic_runtime,
                    vec![Tier::T2, Tier::T3],
                    None,
                )
                .unwrap();
                let catastrophic = build_report(
                    &RunId("fake-run-0001".to_owned()),
                    &catastrophic_runtime,
                )
                .unwrap();
                assert_eq!(
                    catastrophic.artifacts[0]
                        .boundary
                        .as_ref()
                        .unwrap()
                        .failing
                        .as_ref()
                        .unwrap()
                        .tier,
                    Tier::T2
                );
            }

            #[test]
            fn failed_route_retries_higher_thinking_before_tier_escalation() {
                let root = TemporaryRoot::new("horizontal-routes");
                let mut runtime = FakeQualificationRuntime::new(&root);
                runtime.set_qualification_routes(
                    Tier::T2,
                    vec![
                        exact_route(Tier::T2, "anthropic", "haiku", "low"),
                        exact_route(Tier::T2, "anthropic", "haiku", "medium"),
                        exact_route(Tier::T2, "openai-codex", "luna", "low"),
                    ],
                );
                runtime.script(Tier::T2, [ScriptedOutcome::Fail, ScriptedOutcome::Pass]);
                runtime.script(Tier::T1, [ScriptedOutcome::Fail]);

                run_qualify(
                    &root,
                    &mut runtime,
                    vec![Tier::T2, Tier::T1, Tier::T3],
                    None,
                )
                .unwrap();

                let run_id = RunId("fake-run-0001".to_owned());
                let routes = executed_routes(&runtime, &run_id, Tier::T2);
                assert_eq!(routes.len(), 2);
                assert_eq!(routes[0].thinking, "low");
                assert_eq!(routes[1].thinking, "medium");
                assert_eq!(routes[0].model, routes[1].model);
                assert!(executed_routes(&runtime, &run_id, Tier::T3).is_empty());
            }

            #[test]
            fn every_current_tier_route_fails_before_the_next_tier_starts() {
                let root = TemporaryRoot::new("tier-exhaustion");
                let mut runtime = FakeQualificationRuntime::new(&root);
                runtime.set_qualification_routes(
                    Tier::T2,
                    vec![
                        exact_route(Tier::T2, "anthropic", "haiku", "medium"),
                        exact_route(Tier::T2, "openai-codex", "luna", "low"),
                    ],
                );
                runtime.script(
                    Tier::T2,
                    [ScriptedOutcome::Fail, ScriptedOutcome::Fail],
                );

                run_qualify(&root, &mut runtime, vec![Tier::T2, Tier::T3], None).unwrap();

                let run_id = RunId("fake-run-0001".to_owned());
                let sequence = events(&runtime, &run_id)
                    .into_iter()
                    .filter_map(|event| match event {
                        RunEvent::CandidateExecuted { candidate, .. }
                            if matches!(candidate.key.tier, Tier::T2 | Tier::T3) =>
                        {
                            Some((candidate.key.tier, candidate.key.route_index))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    sequence,
                    vec![(Tier::T2, 0), (Tier::T2, 1), (Tier::T3, 0)]
                );
            }

            #[test]
            fn absent_route_order_fails_execution_but_not_dry_run_discovery() {
                let root = TemporaryRoot::new("absent-routes");
                let mut runtime = FakeQualificationRuntime::new(&root);
                runtime.set_qualification_routes(Tier::T1, Vec::new());
                let execute = run_qualify(&root, &mut runtime, vec![Tier::T1], None);

                assert!(matches!(
                    execute,
                    Err(SkillEvalError::InvalidConfiguration(message))
                        if message.contains("absent")
                ));
                assert_eq!(runtime.execute_call_count(), 0);

                let dry_run = qualification_request(&runtime, vec![Tier::T1], None, true);
                run_command(
                    &root,
                    &mut runtime,
                    CliCommand::Qualify { request: dry_run },
                )
                .unwrap();
                assert_eq!(runtime.execute_call_count(), 0);
            }

            #[test]
            fn quota_resume_does_not_repeat_a_completed_exact_route() {
                let root = TemporaryRoot::new("exact-route-resume");
                let mut runtime = FakeQualificationRuntime::new(&root);
                runtime.set_qualification_routes(
                    Tier::T2,
                    vec![
                        exact_route(Tier::T2, "anthropic", "haiku", "low"),
                        exact_route(Tier::T2, "anthropic", "haiku", "medium"),
                    ],
                );
                runtime.script(
                    Tier::T2,
                    [ScriptedOutcome::Fail, ScriptedOutcome::Quota],
                );
                run_qualify(&root, &mut runtime, vec![Tier::T2, Tier::T3], None).unwrap();
                let run_id = RunId("fake-run-0001".to_owned());
                let first_key = events(&runtime, &run_id)
                    .into_iter()
                    .find_map(|event| match event {
                        RunEvent::TrialCompleted { record, .. }
                            if record.key.tier == Tier::T2 && record.key.route_index == 0 =>
                        {
                            Some(record.key)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(checkpoint_event_counts(&runtime, &run_id, &first_key), (1, 1));

                run_command(
                    &root,
                    &mut runtime,
                    CliCommand::Resume {
                        run_id: run_id.clone(),
                    },
                )
                .unwrap();

                assert_eq!(checkpoint_event_counts(&runtime, &run_id, &first_key), (1, 1));
                assert_eq!(executed_routes(&runtime, &run_id, Tier::T2).len(), 2);
            }

            #[test]
            fn quota_resume_reuses_checkpoint_and_identity_drift_stops_resume() {
                let root = TemporaryRoot::new("resume");
                let mut runtime = FakeQualificationRuntime::new(&root);
                runtime.script(
                    Tier::T2,
                    [ScriptedOutcome::Quota, ScriptedOutcome::Pass],
                );
                run_qualify(&root, &mut runtime, vec![Tier::T2, Tier::T1], None).unwrap();
                let run_id = RunId("fake-run-0001".to_owned());
                let paused = build_report(&run_id, &runtime).unwrap();
                assert_eq!(paused.status, RunStatus::Paused);
                assert!(matches!(paused.pause, Some(PauseReason::Quota { .. })));
                let checkpoint = paused.artifacts[0].pending_candidates[0].clone();
                assert_eq!(checkpoint_event_counts(&runtime, &run_id, &checkpoint.key), (1, 0));

                run_command(&root, &mut runtime, CliCommand::Resume { run_id: run_id.clone() })
                    .unwrap();
                let resumed = build_report(&run_id, &runtime).unwrap();
                assert_eq!(resumed.status, RunStatus::AwaitingDecision);
                assert_eq!(checkpoint_event_counts(&runtime, &run_id, &checkpoint.key), (1, 1));
                let completed = completed_candidate(&runtime, &run_id, Tier::T2);
                assert_eq!(completed.artifact_path, checkpoint.artifact_path);
                assert_eq!(completed.transcript_path, checkpoint.transcript_path);
                assert_eq!(completed.model, checkpoint.model);
                assert_eq!(completed.harness, checkpoint.harness);

                run_command(&root, &mut runtime, CliCommand::Resume { run_id: run_id.clone() })
                    .unwrap();
                assert_eq!(checkpoint_event_counts(&runtime, &run_id, &checkpoint.key), (1, 1));

                let drift_root = TemporaryRoot::new("drift");
                let mut drift_runtime = FakeQualificationRuntime::new(&drift_root);
                drift_runtime.script(Tier::T2, [ScriptedOutcome::Quota]);
                run_qualify(
                    &drift_root,
                    &mut drift_runtime,
                    vec![Tier::T2],
                    None,
                )
                .unwrap();
                drift_runtime.drift_runner_identity();
                let result = run_command(
                    &drift_root,
                    &mut drift_runtime,
                    CliCommand::Resume {
                        run_id: RunId("fake-run-0001".to_owned()),
                    },
                );
                assert!(matches!(result, Err(SkillEvalError::InvalidConfiguration(_))));
            }

            #[test]
            fn non_monotonic_evidence_requires_review() {
                let root = TemporaryRoot::new("non-monotonic");
                let mut runtime = FakeQualificationRuntime::new(&root);
                runtime.script(Tier::T2, [ScriptedOutcome::Pass]);
                runtime.script(Tier::T3, [ScriptedOutcome::Fail]);

                run_qualify(&root, &mut runtime, vec![Tier::T2, Tier::T3], None).unwrap();

                let report = build_report(&RunId("fake-run-0001".to_owned()), &runtime).unwrap();
                assert_eq!(report.artifacts[0].status, ArtifactStatus::NeedsReview);
                assert_eq!(
                    report.artifacts[0].review_reason.as_deref(),
                    Some("candidate evidence is non-monotonic")
                );
                assert_eq!(report.artifacts[0].boundary, None);
            }

            #[test]
            fn harness_run_ids_and_exact_model_identities_are_stable() {
                let root = TemporaryRoot::new("identity");
                let mut runtime = FakeQualificationRuntime::new(&root);
                run_qualify(&root, &mut runtime, vec![Tier::T1], None).unwrap();
                let dry_run = qualification_request(&runtime, vec![Tier::T1], None, true);
                run_command(
                    &root,
                    &mut runtime,
                    CliCommand::Qualify { request: dry_run },
                )
                .unwrap();
                assert!(root.path().join("runs/fake-run-0001/events.jsonl").is_file());
                assert!(root.path().join("runs/fake-run-0002/events.jsonl").is_file());

                let run_id = RunId("fake-run-0001".to_owned());
                let events = events(&runtime, &run_id);
                let (route, frozen_harness) = events
                    .iter()
                    .find_map(|event| match event {
                        RunEvent::TrialStarted {
                            key,
                            models,
                            harness,
                            ..
                        } if key.tier == Tier::T1 => Some((models, harness)),
                        _ => None,
                    })
                    .unwrap();
                let effective = completed_candidate(&runtime, &run_id, Tier::T1);
                assert_eq!(route.len(), 1);
                assert_eq!(route[0].model, "effective-T1");
                assert_eq!(effective.model, route[0]);
                assert_eq!(frozen_harness.runner_version, "fake-runner-v1");
                assert_eq!(frozen_harness.pi_version, "fake-pi-v1");
                assert_eq!(frozen_harness.artifact_revision, "synthetic-revision-v1");
                assert_eq!(frozen_harness.tool_policy_digest, "tools:read;timeout:10");
                assert_eq!(effective.harness, *frozen_harness);

                runtime.use_requested_model();
                let identity = runtime
                    .identity(runtime.artifact(), &runtime.artifact().cases[0].execution)
                    .unwrap();
                assert_eq!(identity, *frozen_harness);
            }

            #[test]
            fn every_command_runs_offline_against_temporary_roots() {
                let root = TemporaryRoot::new("commands");
                let mut runtime = FakeQualificationRuntime::new(&root);
                let change = changed_artifact(&runtime);
                run_qualify(
                    &root,
                    &mut runtime,
                    vec![Tier::T1],
                    Some(change),
                )
                .unwrap();
                let run_id = RunId("fake-run-0001".to_owned());
                let artifact = ArtifactName("synthetic-skill".to_owned());
                let rubric_path = runtime.artifact().root.join("evals/rubric.md");
                let own_eval_path = runtime.artifact().root.join("evals/own.json");
                let artifact_bytes = [
                    fs::read(&rubric_path).unwrap(),
                    fs::read(&own_eval_path).unwrap(),
                ];
                let artifact_tiers = runtime.artifact().current_tiers.clone();

                run_command(
                    &root,
                    &mut runtime,
                    CliCommand::Report { run_id: run_id.clone() },
                )
                .unwrap();
                run_command(
                    &root,
                    &mut runtime,
                    CliCommand::Inspect {
                        selector: TrialSelector {
                            run_id: run_id.clone(),
                            artifact: artifact.clone(),
                            tier: Tier::T1,
                            route_index: 0,
                            case: CaseId("synthetic-case".to_owned()),
                            attempt: 1,
                        },
                    },
                )
                .unwrap();
                let early_apply = run_command(
                    &root,
                    &mut runtime,
                    CliCommand::Apply {
                        run_id: run_id.clone(),
                        artifact: artifact.clone(),
                    },
                );
                assert!(matches!(
                    early_apply,
                    Err(SkillEvalError::InvalidArguments(message))
                        if message == "tier assignments require a ready publication gate"
                ));
                assert_eq!(fs::read(&rubric_path).unwrap(), artifact_bytes[0]);
                assert_eq!(fs::read(&own_eval_path).unwrap(), artifact_bytes[1]);
                assert_eq!(runtime.artifact().current_tiers, artifact_tiers);
                assert!(runtime.written_assignments().is_empty());

                let assignments = assignments(Tier::T1);
                run_command(
                    &root,
                    &mut runtime,
                    CliCommand::Decide {
                        run_id: run_id.clone(),
                        artifact: artifact.clone(),
                        decision: Decision::Accepted,
                        assignments: assignments.clone(),
                        reason: None,
                    },
                )
                .unwrap();
                run_command(
                    &root,
                    &mut runtime,
                    CliCommand::Apply {
                        run_id: run_id.clone(),
                        artifact: artifact.clone(),
                    },
                )
                .unwrap();
                assert_eq!(runtime.written_assignments(), &[assignments]);

                let audit_root = root.path().join("audit-output");
                let artifact_root = runtime.artifact().root.clone();
                runtime.use_requested_model();
                run_command(
                    &root,
                    &mut runtime,
                    CliCommand::AuditBriefs {
                        request: AuditBriefRequest {
                            artifact_roots: vec![artifact_root],
                            output_root: audit_root.clone(),
                        },
                    },
                )
                .unwrap();
                assert!(audit_root.join("artifact-0001/brief.json").is_file());
                run_command(
                    &root,
                    &mut runtime,
                    CliCommand::Judge {
                        request: PromptJudgeRequest {
                            prompt: "grade synthetic output".to_owned(),
                            candidate_model: None,
                            timeout_seconds: 10,
                        },
                    },
                )
                .unwrap();

                let resume_root = TemporaryRoot::new("command-resume");
                let mut resume_runtime = FakeQualificationRuntime::new(&resume_root);
                resume_runtime.script(Tier::T1, [ScriptedOutcome::Quota]);
                run_qualify(
                    &resume_root,
                    &mut resume_runtime,
                    vec![Tier::T1],
                    None,
                )
                .unwrap();
                run_command(
                    &resume_root,
                    &mut resume_runtime,
                    CliCommand::Resume {
                        run_id: RunId("fake-run-0001".to_owned()),
                    },
                )
                .unwrap();
            }

            #[test]
            fn accept_and_reject_commands_append_decision_events() {
                for (label, decision, assignments, reason) in [
                    ("accept", Decision::Accepted, assignments(Tier::T1), None),
                    (
                        "reject",
                        Decision::Rejected,
                        Vec::new(),
                        Some("owner keeps the incumbent".to_owned()),
                    ),
                ] {
                    let root = TemporaryRoot::new(label);
                    let mut runtime = FakeQualificationRuntime::new(&root);
                    run_qualify(&root, &mut runtime, vec![Tier::T1], None).unwrap();
                    let run_id = RunId("fake-run-0001".to_owned());
                    run_command(
                        &root,
                        &mut runtime,
                        CliCommand::Decide {
                            run_id: run_id.clone(),
                            artifact: ArtifactName("synthetic-skill".to_owned()),
                            decision,
                            assignments,
                            reason,
                        },
                    )
                    .unwrap();
                    assert!(matches!(
                        events(&runtime, &run_id).last(),
                        Some(RunEvent::DecisionRecorded { decision: recorded, .. })
                            if recorded.decision == decision
                    ));
                }
            }

            fn run_qualify(
                root: &TemporaryRoot,
                runtime: &mut FakeQualificationRuntime,
                tiers: Vec<Tier>,
                change: Option<ArtifactChange>,
            ) -> Result<Vec<u8>, SkillEvalError> {
                let request = qualification_request(runtime, tiers, change, false);
                run_command(root, runtime, CliCommand::Qualify { request })
            }

            fn run_command(
                root: &TemporaryRoot,
                runtime: &mut FakeQualificationRuntime,
                command: CliCommand,
            ) -> Result<Vec<u8>, SkillEvalError> {
                let mut output = Vec::new();
                execute_command(
                    CliRequest {
                        runs_root: root.path().join("runs"),
                        output_format: OutputFormat::JsonLines,
                        command,
                    },
                    runtime,
                    &mut output,
                )?;
                assert!(!output.is_empty());
                Ok(output)
            }

            fn qualification_request(
                runtime: &FakeQualificationRuntime,
                candidate_tiers: Vec<Tier>,
                change: Option<ArtifactChange>,
                is_dry_run: bool,
            ) -> QualifyRequest {
                QualifyRequest {
                    artifact_roots: vec![runtime.artifact().root.clone()],
                    change,
                    policy: QualificationPolicy {
                        purpose: QualificationPurpose::Artifact,
                        candidate_tiers,
                        reference_tier: Tier::T4,
                        judge_tier: Tier::T5,
                        repeats_per_case: 1,
                        minimum_score: 7,
                        noninferiority_margin: 0.1,
                        confidence_level: 0.95,
                    },
                    is_dry_run,
                }
            }

            fn changed_artifact(runtime: &FakeQualificationRuntime) -> ArtifactChange {
                ArtifactChange {
                    artifact: runtime.artifact().name.clone(),
                    kind: runtime.artifact().kind,
                    incumbent_revision: "synthetic-revision-v0".to_owned(),
                    candidate_revision: runtime.artifact().revision.clone(),
                    own_eval: OwnEvalEvidence {
                        artifact_revision: runtime.artifact().revision.clone(),
                        path: runtime.artifact().root.join("evals/own.json"),
                    },
                }
            }

            fn assignments(tier: Tier) -> Vec<TierAssignment> {
                vec![
                    TierAssignment {
                        destination: TierDestination::SkillMinimum,
                        tier,
                    },
                    TierAssignment {
                        destination: TierDestination::SkillTarget,
                        tier,
                    },
                ]
            }

            fn events(runtime: &FakeQualificationRuntime, run_id: &RunId) -> Vec<RunEvent> {
                let mut events = Vec::new();
                runtime
                    .replay(run_id, &mut |event| {
                        events.push(event);
                        Ok(())
                    })
                    .unwrap();
                events
            }

            fn candidate_tiers(
                runtime: &FakeQualificationRuntime,
                run_id: &RunId,
            ) -> Vec<Tier> {
                events(runtime, run_id)
                    .into_iter()
                    .filter_map(|event| match event {
                        RunEvent::TierEvaluated { evidence, .. }
                            if evidence.role == EvidenceRole::Candidate =>
                        {
                            Some(evidence.tier)
                        }
                        _ => None,
                    })
                    .collect()
            }

            fn checkpoint_event_counts(
                runtime: &FakeQualificationRuntime,
                run_id: &RunId,
                key: &$crate::model::TrialKey,
            ) -> (usize, usize) {
                events(runtime, run_id).into_iter().fold(
                    (0, 0),
                    |(executed, completed), event| match event {
                        RunEvent::CandidateExecuted { candidate, .. } if candidate.key == *key => {
                            (executed + 1, completed)
                        }
                        RunEvent::TrialCompleted { record, .. } if record.key == *key => {
                            (executed, completed + 1)
                        }
                        _ => (executed, completed),
                    },
                )
            }

            fn executed_routes(
                runtime: &FakeQualificationRuntime,
                run_id: &RunId,
                tier: Tier,
            ) -> Vec<ModelIdentity> {
                events(runtime, run_id)
                    .into_iter()
                    .filter_map(|event| match event {
                        RunEvent::CandidateExecuted { candidate, .. }
                            if candidate.key.tier == tier =>
                        {
                            Some(candidate.model)
                        }
                        _ => None,
                    })
                    .collect()
            }

            fn exact_route(
                tier: Tier,
                provider: &str,
                model: &str,
                thinking: &str,
            ) -> ModelIdentity {
                ModelIdentity {
                    tier,
                    provider: provider.to_owned(),
                    model: model.to_owned(),
                    thinking: thinking.to_owned(),
                }
            }

            fn completed_candidate(
                runtime: &FakeQualificationRuntime,
                run_id: &RunId,
                tier: Tier,
            ) -> $crate::model::TrialRecord {
                events(runtime, run_id)
                    .into_iter()
                    .find_map(|event| match event {
                        RunEvent::TrialCompleted { record, .. } if record.key.tier == tier => {
                            Some(record)
                        }
                        _ => None,
                    })
                    .unwrap()
            }
        }
    };
}
