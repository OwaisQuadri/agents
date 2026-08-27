#[macro_export]
macro_rules! publication_tests {
    () => {
        mod publication {
            use std::path::PathBuf;

            use $crate::model::{
                ArtifactChange, ArtifactKind, ArtifactName, ArtifactReport, ArtifactStatus,
                ConfidenceInterval, Decision, DecisionRecord, EvidenceRole, HarnessIdentity,
                ModelIdentity, OwnEvalEvidence, PublicationStatus, QualificationBoundary,
                QualificationReport, RunId, RunMode, RunStatus, Tier, TierAssignment,
                TierDestination, TierEvidence, TierStatus, Timestamp, TrialUsage,
            };

            use super::evaluate_publication_gate;

            #[test]
            fn tc_38_requires_current_matching_supported_and_accepted_evidence() {
                let change = change(ArtifactKind::Skill);
                let ready = report(
                    &change,
                    vec![TierDestination::SkillMinimum],
                    vec![assignment(TierDestination::SkillMinimum, Tier::T2)],
                );
                assert_status(&change, &ready, PublicationStatus::Ready);

                let mut stale_eval = change.clone();
                stale_eval.own_eval.artifact_revision = "incumbent".to_owned();
                assert_status(&stale_eval, &ready, PublicationStatus::Blocked);

                let mut stale_report = ready.clone();
                stale_report.change.as_mut().unwrap().candidate_revision = "later".to_owned();
                assert_status(&change, &stale_report, PublicationStatus::Blocked);

                let mut missing = ready.clone();
                missing.artifacts.clear();
                assert_status(&change, &missing, PublicationStatus::Blocked);

                let mut duplicate = ready.clone();
                duplicate.artifacts.push(duplicate.artifacts[0].clone());
                assert_status(&change, &duplicate, PublicationStatus::Blocked);

                let mut wrong_kind = ready.clone();
                wrong_kind.artifacts[0].kind = ArtifactKind::Agent;
                assert_status(&change, &wrong_kind, PublicationStatus::Blocked);

                let mut mismatched_evidence = ready.clone();
                mismatched_evidence.artifacts[0]
                    .boundary
                    .as_mut()
                    .unwrap()
                    .accepted
                    .harnesses[0]
                    .artifact_revision = "incumbent".to_owned();
                assert_status(&change, &mismatched_evidence, PublicationStatus::Blocked);

                let mut stale_evidence = ready.clone();
                stale_evidence.artifacts[0].tiers[0].harnesses[0].artifact_revision =
                    "incumbent".to_owned();
                stale_evidence.artifacts[0].boundary = Some(QualificationBoundary {
                    failing: None,
                    accepted: stale_evidence.artifacts[0].tiers[0].clone(),
                });
                assert_status(&change, &stale_evidence, PublicationStatus::Blocked);

                let mut no_boundary = ready.clone();
                no_boundary.status = RunStatus::Running;
                no_boundary.artifacts[0].status = ArtifactStatus::Running;
                no_boundary.artifacts[0].boundary = None;
                no_boundary.artifacts[0].decision = None;
                assert_status(
                    &change,
                    &no_boundary,
                    PublicationStatus::AwaitingQualification,
                );

                let mut no_decision = ready.clone();
                no_decision.status = RunStatus::AwaitingDecision;
                no_decision.artifacts[0].status = ArtifactStatus::AwaitingDecision;
                no_decision.artifacts[0].decision = None;
                assert_status(&change, &no_decision, PublicationStatus::AwaitingDecision);

                let mut failed = no_boundary.clone();
                failed.artifacts[0].tiers[0].status = TierStatus::Failed;
                assert_status(&change, &failed, PublicationStatus::Blocked);

                let mut paused = no_boundary.clone();
                paused.status = RunStatus::Paused;
                paused.artifacts[0].status = ArtifactStatus::Paused;
                assert_status(&change, &paused, PublicationStatus::Blocked);

                let mut review = no_boundary.clone();
                review.status = RunStatus::Failed;
                review.artifacts[0].status = ArtifactStatus::NeedsReview;
                assert_status(&change, &review, PublicationStatus::Blocked);

                let mut rejected = ready.clone();
                rejected.artifacts[0].status = ArtifactStatus::Rejected;
                rejected.artifacts[0].decision.as_mut().unwrap().decision = Decision::Rejected;
                rejected.artifacts[0]
                    .decision
                    .as_mut()
                    .unwrap()
                    .assignments
                    .clear();
                assert_status(&change, &rejected, PublicationStatus::Blocked);
            }

            #[test]
            fn tc_41_requires_exact_destinations_at_the_supported_tier_for_every_kind() {
                let cases = [
                    (
                        ArtifactKind::Skill,
                        vec![TierDestination::SkillMinimum, TierDestination::SkillTarget],
                    ),
                    (ArtifactKind::Agent, vec![TierDestination::Agent]),
                    (
                        ArtifactKind::Workflow,
                        vec![
                            TierDestination::WorkflowOrchestrator,
                            TierDestination::WorkflowNode {
                                node: "builder".to_owned(),
                            },
                        ],
                    ),
                ];
                for (kind, destinations) in cases {
                    let change = change(kind);
                    let assignments = destinations
                        .iter()
                        .cloned()
                        .map(|destination| assignment(destination, Tier::T2))
                        .collect();
                    let report = report(&change, destinations, assignments);
                    let gate = evaluate_publication_gate(&change, &report).unwrap();
                    assert_eq!(gate.status, PublicationStatus::Ready);
                    assert_eq!(
                        gate.assignments,
                        report.artifacts[0].decision.as_ref().unwrap().assignments
                    );
                }

                let workflow_change = change(ArtifactKind::Workflow);
                let workflow_destinations = vec![
                    TierDestination::WorkflowOrchestrator,
                    TierDestination::WorkflowNode {
                        node: "builder".to_owned(),
                    },
                ];
                let valid = vec![
                    assignment(TierDestination::WorkflowOrchestrator, Tier::T2),
                    assignment(
                        TierDestination::WorkflowNode {
                            node: "builder".to_owned(),
                        },
                        Tier::T2,
                    ),
                ];
                let invalid = [
                    vec![valid[0].clone()],
                    vec![valid[0].clone(), valid[1].clone(), valid[1].clone()],
                    vec![
                        valid[0].clone(),
                        valid[1].clone(),
                        assignment(TierDestination::Agent, Tier::T2),
                    ],
                    vec![
                        assignment(TierDestination::WorkflowOrchestrator, Tier::T3),
                        valid[1].clone(),
                    ],
                ];
                for assignments in invalid {
                    let report =
                        report(&workflow_change, workflow_destinations.clone(), assignments);
                    let gate = evaluate_publication_gate(&workflow_change, &report).unwrap();
                    assert_eq!(gate.status, PublicationStatus::Blocked);
                    assert!(gate.assignments.is_empty());
                    assert_eq!(gate.change.incumbent_revision, "incumbent");
                }
            }

            #[test]
            fn tc_67_blocks_equal_assignments_with_wrong_destination_kind() {
                let change = change(ArtifactKind::Skill);
                let forged = report(
                    &change,
                    vec![TierDestination::Agent],
                    vec![assignment(TierDestination::Agent, Tier::T2)],
                );

                assert_status(&change, &forged, PublicationStatus::Blocked);
            }

            #[test]
            fn tc_67_validates_base_duplicate_and_empty_required_destination_lists() {
                let valid = [
                    (ArtifactKind::Skill, vec![TierDestination::SkillMinimum]),
                    (ArtifactKind::Agent, vec![TierDestination::Agent]),
                    (
                        ArtifactKind::Workflow,
                        vec![
                            TierDestination::WorkflowOrchestrator,
                            TierDestination::WorkflowNode {
                                node: "builder".to_owned(),
                            },
                        ],
                    ),
                ];
                for (kind, destinations) in valid {
                    let change = change(kind);
                    let assignments = destinations
                        .iter()
                        .cloned()
                        .map(|destination| assignment(destination, Tier::T2))
                        .collect();
                    let qualification = report(&change, destinations, assignments);
                    assert_status(&change, &qualification, PublicationStatus::Ready);
                }

                let invalid = vec![
                    (ArtifactKind::Skill, Vec::new()),
                    (
                        ArtifactKind::Skill,
                        vec![TierDestination::SkillMinimum, TierDestination::SkillMinimum],
                    ),
                    (ArtifactKind::Agent, Vec::new()),
                    (
                        ArtifactKind::Agent,
                        vec![TierDestination::Agent, TierDestination::Agent],
                    ),
                    (ArtifactKind::Workflow, Vec::new()),
                    (
                        ArtifactKind::Workflow,
                        vec![
                            TierDestination::WorkflowOrchestrator,
                            TierDestination::WorkflowNode {
                                node: "builder".to_owned(),
                            },
                            TierDestination::WorkflowNode {
                                node: "builder".to_owned(),
                            },
                        ],
                    ),
                    (
                        ArtifactKind::Workflow,
                        vec![
                            TierDestination::WorkflowOrchestrator,
                            TierDestination::WorkflowNode {
                                node: String::new(),
                            },
                        ],
                    ),
                ];
                for (kind, destinations) in invalid {
                    let change = change(kind);
                    let assignments = destinations
                        .iter()
                        .cloned()
                        .map(|destination| assignment(destination, Tier::T2))
                        .collect();
                    let qualification = report(&change, destinations, assignments);
                    assert_status(&change, &qualification, PublicationStatus::Blocked);
                }
            }

            fn assert_status(
                change: &ArtifactChange,
                report: &QualificationReport,
                expected: PublicationStatus,
            ) {
                let gate = evaluate_publication_gate(change, report).unwrap();
                assert_eq!(gate.status, expected);
                if expected != PublicationStatus::Ready {
                    assert!(gate.assignments.is_empty());
                    assert_eq!(gate.change.incumbent_revision, "incumbent");
                }
            }

            fn report(
                change: &ArtifactChange,
                required_destinations: Vec<TierDestination>,
                assignments: Vec<TierAssignment>,
            ) -> QualificationReport {
                let accepted = evidence(change, Tier::T2);
                QualificationReport {
                    run_id: RunId("run-publication".to_owned()),
                    mode: RunMode::Execute,
                    change: Some(change.clone()),
                    status: RunStatus::Completed,
                    discoveries: Vec::new(),
                    artifacts: vec![ArtifactReport {
                        artifact: change.artifact.clone(),
                        kind: change.kind,
                        required_destinations,
                        status: ArtifactStatus::Accepted,
                        review_reason: None,
                        pending_candidates: Vec::new(),
                        reference: None,
                        tiers: vec![accepted.clone()],
                        boundary: Some(QualificationBoundary {
                            failing: None,
                            accepted,
                        }),
                        decision: Some(DecisionRecord {
                            artifact: change.artifact.clone(),
                            decision: Decision::Accepted,
                            assignments,
                            reason: None,
                            decided_at: Timestamp("decision-time".to_owned()),
                        }),
                        publication_gate: None,
                    }],
                    pause: None,
                    total_usage: usage(),
                }
            }

            fn change(kind: ArtifactKind) -> ArtifactChange {
                ArtifactChange {
                    artifact: ArtifactName("artifact".to_owned()),
                    kind,
                    incumbent_revision: "incumbent".to_owned(),
                    candidate_revision: "candidate".to_owned(),
                    own_eval: OwnEvalEvidence {
                        artifact_revision: "candidate".to_owned(),
                        path: PathBuf::from("evals/results.json"),
                    },
                }
            }

            fn evidence(change: &ArtifactChange, tier: Tier) -> TierEvidence {
                TierEvidence {
                    role: EvidenceRole::Candidate,
                    tier,
                    model: ModelIdentity {
                        tier,
                        provider: "provider".to_owned(),
                        model: "model".to_owned(),
                        thinking: "thinking".to_owned(),
                    },
                    harnesses: vec![HarnessIdentity {
                        runner_version: "runner".to_owned(),
                        pi_version: "pi".to_owned(),
                        artifact_revision: change.candidate_revision.clone(),
                        tool_policy_digest: "policy".to_owned(),
                    }],
                    status: TierStatus::Accepted,
                    completed_trials: 1,
                    expected_trials: 1,
                    passed_trials: 1,
                    score: ConfidenceInterval {
                        lower: 1.0,
                        estimate: 1.0,
                        upper: 1.0,
                    },
                    candidate_usage: usage(),
                    judge_usage: usage(),
                    total_usage: usage(),
                }
            }

            fn assignment(destination: TierDestination, tier: Tier) -> TierAssignment {
                TierAssignment { destination, tier }
            }

            fn usage() -> TrialUsage {
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
