#[macro_export]
macro_rules! report_destination_tests {
    () => {
        mod report_destinations {
            use std::path::PathBuf;

            use $crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, QualificationPolicy,
                RunConfiguration, RunEvent, RunId, RunMode, SkillEvalError, Tier, TierAssignment,
                TierDestination, Timestamp, TrialRecord, TrialSelector,
            };
            use $crate::ports::RunStore;

            use super::build_report;

            #[test]
            fn tc_48_skill_copies_required_destinations_in_declaration_order() {
                let report = report(vec![artifact(
                    "skill",
                    ArtifactKind::Skill,
                    vec![TierDestination::SkillTarget, TierDestination::SkillMinimum],
                    vec![assignment(TierDestination::SkillMinimum)],
                )])
                .unwrap();

                assert_eq!(
                    report.artifacts[0].required_destinations,
                    vec![TierDestination::SkillTarget, TierDestination::SkillMinimum,]
                );
            }

            #[test]
            fn tc_48_current_tiers_do_not_add_required_destinations() {
                let report = report(vec![artifact(
                    "skill",
                    ArtifactKind::Skill,
                    vec![TierDestination::SkillMinimum],
                    vec![
                        assignment(TierDestination::SkillMinimum),
                        assignment(TierDestination::SkillTarget),
                    ],
                )])
                .unwrap();

                assert_eq!(
                    report.artifacts[0].required_destinations,
                    vec![TierDestination::SkillMinimum]
                );
            }

            #[test]
            fn tc_48_agent_requires_exactly_agent() {
                let report = report(vec![artifact(
                    "agent",
                    ArtifactKind::Agent,
                    vec![TierDestination::Agent],
                    Vec::new(),
                )])
                .unwrap();

                assert_eq!(
                    report.artifacts[0].required_destinations,
                    vec![TierDestination::Agent]
                );
            }

            #[test]
            fn tc_48_workflow_copies_prequalification_model_nodes_in_order() {
                let report = report(vec![artifact(
                    "workflow",
                    ArtifactKind::Workflow,
                    vec![
                        node("planner"),
                        TierDestination::WorkflowOrchestrator,
                        node("builder"),
                    ],
                    vec![assignment(TierDestination::WorkflowOrchestrator)],
                )])
                .unwrap();

                assert_eq!(
                    report.artifacts[0].required_destinations,
                    vec![
                        node("planner"),
                        TierDestination::WorkflowOrchestrator,
                        node("builder"),
                    ]
                );
            }

            #[test]
            fn tc_48_rejects_duplicate_required_destinations() {
                for definition in [
                    artifact(
                        "skill",
                        ArtifactKind::Skill,
                        vec![TierDestination::SkillMinimum, TierDestination::SkillMinimum],
                        Vec::new(),
                    ),
                    artifact(
                        "agent",
                        ArtifactKind::Agent,
                        vec![TierDestination::Agent, TierDestination::Agent],
                        Vec::new(),
                    ),
                    artifact(
                        "workflow",
                        ArtifactKind::Workflow,
                        vec![
                            TierDestination::WorkflowOrchestrator,
                            TierDestination::WorkflowOrchestrator,
                        ],
                        Vec::new(),
                    ),
                ] {
                    assert_invalid(report(vec![definition]));
                }
            }

            #[test]
            fn tc_48_rejects_duplicate_workflow_node_names() {
                assert_invalid(report(vec![artifact(
                    "workflow",
                    ArtifactKind::Workflow,
                    vec![
                        TierDestination::WorkflowOrchestrator,
                        node("builder"),
                        node("builder"),
                    ],
                    Vec::new(),
                )]));
            }

            #[test]
            fn tc_48_rejects_missing_base_destinations() {
                for definition in [
                    artifact(
                        "skill",
                        ArtifactKind::Skill,
                        vec![TierDestination::SkillTarget],
                        Vec::new(),
                    ),
                    artifact("agent", ArtifactKind::Agent, Vec::new(), Vec::new()),
                    artifact(
                        "workflow",
                        ArtifactKind::Workflow,
                        vec![node("builder")],
                        Vec::new(),
                    ),
                ] {
                    assert_invalid(report(vec![definition]));
                }
            }

            #[test]
            fn tc_48_rejects_wrong_kind_required_destinations() {
                for definition in [
                    artifact(
                        "skill",
                        ArtifactKind::Skill,
                        vec![TierDestination::SkillMinimum, TierDestination::Agent],
                        Vec::new(),
                    ),
                    artifact(
                        "agent",
                        ArtifactKind::Agent,
                        vec![TierDestination::Agent, TierDestination::SkillMinimum],
                        Vec::new(),
                    ),
                    artifact(
                        "workflow",
                        ArtifactKind::Workflow,
                        vec![
                            TierDestination::WorkflowOrchestrator,
                            TierDestination::SkillTarget,
                        ],
                        Vec::new(),
                    ),
                ] {
                    assert_invalid(report(vec![definition]));
                }
            }

            #[test]
            fn tc_48_rejects_unnamed_workflow_node() {
                assert_invalid(report(vec![artifact(
                    "workflow",
                    ArtifactKind::Workflow,
                    vec![TierDestination::WorkflowOrchestrator, node("   ")],
                    Vec::new(),
                )]));
            }

            fn report(
                artifacts: Vec<ArtifactDefinition>,
            ) -> Result<$crate::model::QualificationReport, SkillEvalError> {
                let store = Store {
                    events: vec![RunEvent::RunStarted {
                        at: timestamp(),
                        configuration: RunConfiguration {
                            run_id: run_id(),
                            mode: RunMode::Execute,
                            artifacts,
                            change: None,
                            // TODO(AGNT-0032.T82): Mark this report run as artifact qualification.
                            policy: QualificationPolicy {
                                candidate_tiers: vec![Tier::T2],
                                reference_tier: Tier::T4,
                                judge_tier: Tier::T5,
                                repeats_per_case: 1,
                                minimum_score: 8,
                                noninferiority_margin: 0.1,
                                confidence_level: 0.95,
                            },
                            created_at: timestamp(),
                        },
                    }],
                };
                build_report(&run_id(), &store)
            }

            fn artifact(
                name: &str,
                kind: ArtifactKind,
                required_destinations: Vec<TierDestination>,
                current_tiers: Vec<TierAssignment>,
            ) -> ArtifactDefinition {
                ArtifactDefinition {
                    name: ArtifactName(name.to_owned()),
                    kind,
                    root: PathBuf::from(name),
                    revision: "revision".to_owned(),
                    required_destinations,
                    current_tiers,
                    cases: Vec::new(),
                }
            }

            fn assignment(destination: TierDestination) -> TierAssignment {
                TierAssignment {
                    destination,
                    tier: Tier::T3,
                }
            }

            fn node(name: &str) -> TierDestination {
                TierDestination::WorkflowNode {
                    node: name.to_owned(),
                }
            }

            fn run_id() -> RunId {
                RunId("report-destinations".to_owned())
            }

            fn timestamp() -> Timestamp {
                Timestamp("2026-08-22T05:00:00-0400".to_owned())
            }

            fn assert_invalid<T>(result: Result<T, SkillEvalError>) {
                assert!(matches!(
                    result,
                    Err(SkillEvalError::InvalidConfiguration(_))
                ));
            }

            struct Store {
                events: Vec<RunEvent>,
            }

            impl RunStore for Store {
                fn append(
                    &mut self,
                    _run_id: &RunId,
                    _event: &RunEvent,
                ) -> Result<(), SkillEvalError> {
                    panic!("report destination tests do not append events")
                }

                fn replay(
                    &self,
                    requested_run_id: &RunId,
                    visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
                ) -> Result<(), SkillEvalError> {
                    assert_eq!(requested_run_id, &run_id());
                    for event in self.events.clone() {
                        visitor(event)?;
                    }
                    Ok(())
                }

                fn find_trial(
                    &self,
                    _selector: &TrialSelector,
                ) -> Result<TrialRecord, SkillEvalError> {
                    panic!("report destination tests do not inspect trials")
                }
            }
        }
    };
}
