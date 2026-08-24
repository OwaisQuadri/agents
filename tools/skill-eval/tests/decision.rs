#[macro_export]
macro_rules! decision_tests {
    () => {
        mod decision {
            use std::cell::Cell;
            use std::path::PathBuf;

            use $crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, ConfidenceInterval,
                Decision, EvidenceRole, HarnessIdentity, ModelIdentity, ParentResponsibility,
                QualificationBoundary, QualificationPolicy, RunConfiguration, RunEvent, RunId,
                RunMode, SkillEvalError, Tier, TierAssignment, TierDestination, TierEvidence,
                TierStatus, Timestamp, TrialRecord, TrialSelector, TrialUsage,
            };
            use $crate::ports::{Clock, RunStore};

            use super::{build_report, record_decision, routing_decision};

            #[test]
            fn accept_and_reject_append_one_immutable_owner_decision() {
                let mut accepted = Store::new(
                    ArtifactKind::Skill,
                    skill_required_destinations(),
                    skill_destinations(),
                    true,
                );
                let record = record_decision(
                    &run_id(),
                    &artifact_name(),
                    Decision::Accepted,
                    skill_assignments(Tier::T2),
                    Some("owner selected the supported route".to_owned()),
                    &mut accepted,
                    &TestClock,
                )
                .unwrap();

                assert_eq!(record.decided_at, Timestamp("decision-time".to_owned()));
                assert_eq!(accepted.append_count, 1);
                assert_eq!(accepted.events.len(), 4);
                assert!(matches!(
                    accepted.events.last(),
                    Some(RunEvent::DecisionRecorded { decision, .. }) if decision == &record
                ));

                let mut rejected = Store::new(
                    ArtifactKind::Skill,
                    skill_required_destinations(),
                    skill_destinations(),
                    true,
                );
                let record = record_decision(
                    &run_id(),
                    &artifact_name(),
                    Decision::Rejected,
                    Vec::new(),
                    Some("keep the incumbent".to_owned()),
                    &mut rejected,
                    &TestClock,
                )
                .unwrap();

                assert_eq!(record.reason.as_deref(), Some("keep the incumbent"));
                assert_eq!(rejected.append_count, 1);
                assert_eq!(rejected.replay_count.get(), 1);
            }

            #[test]
            fn invalid_accept_assignment_shapes_never_append() {
                let cases = [
                    vec![assignment(TierDestination::SkillMinimum, Tier::T2)],
                    vec![
                        assignment(TierDestination::SkillMinimum, Tier::T2),
                        assignment(TierDestination::SkillTarget, Tier::T2),
                        assignment(TierDestination::SkillTarget, Tier::T2),
                    ],
                    vec![
                        assignment(TierDestination::SkillMinimum, Tier::T2),
                        assignment(TierDestination::SkillTarget, Tier::T2),
                        assignment(TierDestination::Agent, Tier::T2),
                    ],
                    vec![assignment(TierDestination::Agent, Tier::T2)],
                    skill_assignments(Tier::T3),
                ];

                for assignments in cases {
                    let mut store = Store::new(
                        ArtifactKind::Skill,
                        skill_required_destinations(),
                        skill_destinations(),
                        true,
                    );
                    let result = record_decision(
                        &run_id(),
                        &artifact_name(),
                        Decision::Accepted,
                        assignments,
                        None,
                        &mut store,
                        &TestClock,
                    );
                    assert!(matches!(result, Err(SkillEvalError::InvalidArguments(_))));
                    assert_eq!(store.append_count, 0);
                    assert_eq!(store.events.len(), 3);
                }
            }

            #[test]
            fn unknown_workflow_destination_never_appends() {
                let configured = vec![
                    assignment(TierDestination::WorkflowOrchestrator, Tier::T3),
                    assignment(
                        TierDestination::WorkflowNode {
                            node: "known".to_owned(),
                        },
                        Tier::T3,
                    ),
                ];
                let mut store = Store::new(
                    ArtifactKind::Workflow,
                    vec![
                        TierDestination::WorkflowOrchestrator,
                        TierDestination::WorkflowNode {
                            node: "known".to_owned(),
                        },
                    ],
                    configured,
                    true,
                );
                let result = record_decision(
                    &run_id(),
                    &artifact_name(),
                    Decision::Accepted,
                    vec![
                        assignment(TierDestination::WorkflowOrchestrator, Tier::T2),
                        assignment(
                            TierDestination::WorkflowNode {
                                node: "unknown".to_owned(),
                            },
                            Tier::T2,
                        ),
                    ],
                    None,
                    &mut store,
                    &TestClock,
                );

                assert!(matches!(result, Err(SkillEvalError::InvalidArguments(_))));
                assert_eq!(store.append_count, 0);
            }

            #[test]
            fn invalid_rejections_never_append() {
                for (assignments, reason) in [
                    (Vec::new(), None),
                    (Vec::new(), Some("   ".to_owned())),
                    (
                        vec![assignment(TierDestination::SkillMinimum, Tier::T2)],
                        Some("no".to_owned()),
                    ),
                ] {
                    let mut store = Store::new(
                        ArtifactKind::Skill,
                        skill_required_destinations(),
                        skill_destinations(),
                        true,
                    );
                    let result = record_decision(
                        &run_id(),
                        &artifact_name(),
                        Decision::Rejected,
                        assignments,
                        reason,
                        &mut store,
                        &TestClock,
                    );
                    assert!(matches!(result, Err(SkillEvalError::InvalidArguments(_))));
                    assert_eq!(store.append_count, 0);
                }
            }

            #[test]
            fn duplicate_decision_and_wrong_state_never_append() {
                let mut duplicate = Store::new(
                    ArtifactKind::Skill,
                    skill_required_destinations(),
                    skill_destinations(),
                    true,
                );
                record_decision(
                    &run_id(),
                    &artifact_name(),
                    Decision::Rejected,
                    Vec::new(),
                    Some("no".to_owned()),
                    &mut duplicate,
                    &TestClock,
                )
                .unwrap();
                let result = record_decision(
                    &run_id(),
                    &artifact_name(),
                    Decision::Rejected,
                    Vec::new(),
                    Some("again".to_owned()),
                    &mut duplicate,
                    &TestClock,
                );
                assert!(matches!(result, Err(SkillEvalError::InvalidArguments(_))));
                assert_eq!(duplicate.append_count, 1);

                let mut wrong_state = Store::new(
                    ArtifactKind::Skill,
                    skill_required_destinations(),
                    skill_destinations(),
                    false,
                );
                let result = record_decision(
                    &run_id(),
                    &artifact_name(),
                    Decision::Rejected,
                    Vec::new(),
                    Some("no".to_owned()),
                    &mut wrong_state,
                    &TestClock,
                );
                assert!(matches!(result, Err(SkillEvalError::InvalidArguments(_))));
                assert_eq!(wrong_state.append_count, 0);
            }

            #[test]
            fn safe_child_route_retains_parent_responsibilities() {
                let mut store = Store::new(
                    ArtifactKind::Skill,
                    skill_required_destinations(),
                    skill_destinations(),
                    true,
                );
                record_decision(
                    &run_id(),
                    &artifact_name(),
                    Decision::Accepted,
                    skill_assignments(Tier::T2),
                    None,
                    &mut store,
                    &TestClock,
                )
                .unwrap();
                let report = build_report(&run_id(), &store).unwrap();

                let route = routing_decision(&report, &artifact_name()).unwrap().unwrap();

                assert_eq!(route.target_tier, Tier::T2);
                assert_eq!(
                    route.parent_responsibilities,
                    vec![
                        ParentResponsibility::HumanDecision,
                        ParentResponsibility::IrreversibleAction,
                        ParentResponsibility::FinalVerification,
                    ]
                );

                for kind in [ArtifactKind::Agent, ArtifactKind::Workflow] {
                    let store = Store::new(kind, base_destinations(kind), destinations(kind), true);
                    let report = build_report(&run_id(), &store).unwrap();
                    assert_eq!(routing_decision(&report, &artifact_name()).unwrap(), None);
                }

                let store = Store::new(
                    ArtifactKind::Skill,
                    skill_required_destinations(),
                    skill_destinations(),
                    true,
                );
                let report = build_report(&run_id(), &store).unwrap();
                assert_eq!(routing_decision(&report, &artifact_name()).unwrap(), None);
            }

            struct TestClock;

            impl Clock for TestClock {
                fn now(&self) -> Timestamp {
                    Timestamp("decision-time".to_owned())
                }
            }

            struct Store {
                events: Vec<RunEvent>,
                replay_count: Cell<usize>,
                append_count: usize,
            }

            impl Store {
                fn new(
                    kind: ArtifactKind,
                    required_destinations: Vec<TierDestination>,
                    current_tiers: Vec<TierAssignment>,
                    is_awaiting_decision: bool,
                ) -> Self {
                    let mut events = vec![RunEvent::RunStarted {
                        at: Timestamp("start".to_owned()),
                        configuration: RunConfiguration {
                            run_id: run_id(),
                            mode: RunMode::Execute,
                            artifacts: vec![ArtifactDefinition {
                                name: artifact_name(),
                                kind,
                                root: PathBuf::from("artifact"),
                                revision: "revision".to_owned(),
                                required_destinations,
                                current_tiers,
                                cases: Vec::new(),
                            }],
                            change: None,
                            // TODO(AGNT-0032.T82): Mark this decision run as artifact qualification.
                            policy: QualificationPolicy {
                                candidate_tiers: vec![Tier::T2],
                                reference_tier: Tier::T4,
                                judge_tier: Tier::T5,
                                repeats_per_case: 1,
                                minimum_score: 8,
                                noninferiority_margin: 0.1,
                                confidence_level: 0.95,
                            },
                            created_at: Timestamp("start".to_owned()),
                        },
                    }];
                    if is_awaiting_decision {
                        let accepted = evidence();
                        events.push(RunEvent::TierEvaluated {
                            at: Timestamp("evidence".to_owned()),
                            artifact: artifact_name(),
                            evidence: accepted.clone(),
                        });
                        events.push(RunEvent::BoundaryFound {
                            at: Timestamp("boundary".to_owned()),
                            artifact: artifact_name(),
                            boundary: QualificationBoundary {
                                failing: None,
                                accepted,
                            },
                        });
                    }
                    Self {
                        events,
                        replay_count: Cell::new(0),
                        append_count: 0,
                    }
                }
            }

            impl RunStore for Store {
                fn append(
                    &mut self,
                    requested_run_id: &RunId,
                    event: &RunEvent,
                ) -> Result<(), SkillEvalError> {
                    assert_eq!(requested_run_id, &run_id());
                    self.append_count += 1;
                    self.events.push(event.clone());
                    Ok(())
                }

                fn replay(
                    &self,
                    requested_run_id: &RunId,
                    visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
                ) -> Result<(), SkillEvalError> {
                    assert_eq!(requested_run_id, &run_id());
                    self.replay_count.set(self.replay_count.get() + 1);
                    for event in self.events.clone() {
                        visitor(event)?;
                    }
                    Ok(())
                }

                fn find_trial(
                    &self,
                    _selector: &TrialSelector,
                ) -> Result<TrialRecord, SkillEvalError> {
                    panic!("decision tests do not inspect trials")
                }
            }

            fn evidence() -> TierEvidence {
                TierEvidence {
                    role: EvidenceRole::Candidate,
                    tier: Tier::T2,
                    model: ModelIdentity {
                        tier: Tier::T2,
                        provider: "provider".to_owned(),
                        model: "model".to_owned(),
                        thinking: "thinking".to_owned(),
                    },
                    harnesses: vec![HarnessIdentity {
                        runner_version: "runner".to_owned(),
                        pi_version: "pi".to_owned(),
                        artifact_revision: "revision".to_owned(),
                        tool_policy_digest: "tools".to_owned(),
                    }],
                    status: TierStatus::Accepted,
                    completed_trials: 1,
                    expected_trials: 1,
                    passed_trials: 1,
                    score: ConfidenceInterval {
                        lower: 8.0,
                        estimate: 9.0,
                        upper: 10.0,
                    },
                    candidate_usage: usage(),
                    judge_usage: usage(),
                    total_usage: usage(),
                }
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

            fn run_id() -> RunId {
                RunId("decision-run".to_owned())
            }

            fn artifact_name() -> ArtifactName {
                ArtifactName("artifact".to_owned())
            }

            fn assignment(destination: TierDestination, tier: Tier) -> TierAssignment {
                TierAssignment { destination, tier }
            }

            fn skill_assignments(tier: Tier) -> Vec<TierAssignment> {
                vec![
                    assignment(TierDestination::SkillMinimum, tier),
                    assignment(TierDestination::SkillTarget, tier),
                ]
            }

            fn skill_destinations() -> Vec<TierAssignment> {
                skill_assignments(Tier::T3)
            }

            fn skill_required_destinations() -> Vec<TierDestination> {
                vec![
                    TierDestination::SkillMinimum,
                    TierDestination::SkillTarget,
                ]
            }

            fn base_destinations(kind: ArtifactKind) -> Vec<TierDestination> {
                vec![match kind {
                    ArtifactKind::Skill => TierDestination::SkillMinimum,
                    ArtifactKind::Agent => TierDestination::Agent,
                    ArtifactKind::Workflow => TierDestination::WorkflowOrchestrator,
                }]
            }

            fn destinations(kind: ArtifactKind) -> Vec<TierAssignment> {
                match kind {
                    ArtifactKind::Skill => skill_destinations(),
                    ArtifactKind::Agent => {
                        vec![assignment(TierDestination::Agent, Tier::T3)]
                    }
                    ArtifactKind::Workflow => vec![assignment(
                        TierDestination::WorkflowOrchestrator,
                        Tier::T3,
                    )],
                }
            }
        }
    };
}
