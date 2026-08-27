#[macro_export]
macro_rules! source_destination_tests {
    () => {
        #[test]
        fn tc_49_skill_required_destinations_are_independent_from_current_tiers() {
            let without_tiers = Fixture::new();
            without_tiers.write("SKILL.md", &skill_definition(""));
            without_tiers.write("evals/cases.jsonl", &one_case(""));

            let artifact = without_tiers.load().unwrap();

            assert_eq!(
                artifact.required_destinations,
                vec![TierDestination::SkillMinimum]
            );
            assert!(artifact.current_tiers.is_empty());

            let with_target = Fixture::new();
            with_target.write(
                "SKILL.md",
                &skill_definition("  minimum-tier: T3\n  target-tier: T2\n"),
            );
            with_target.write("evals/cases.jsonl", &one_case(""));

            let artifact = with_target.load().unwrap();

            assert_eq!(
                artifact.required_destinations,
                vec![TierDestination::SkillMinimum, TierDestination::SkillTarget]
            );
            assert_eq!(artifact.current_tiers.len(), 2);
        }

        #[test]
        fn tc_49_agent_requires_agent_without_inventing_an_assignment() {
            let fixture = Fixture::new();
            fixture.write(
                "fixture-agent.md",
                "---\nname: fixture-agent\ndescription: A complete agent description.\ntools: Read\nmodel: sonnet\n---\nbody\n",
            );
            fixture.write("evals/cases.jsonl", &one_case(""));

            let artifact = fixture.load().unwrap();

            assert_eq!(artifact.required_destinations, vec![TierDestination::Agent]);
            assert!(artifact.current_tiers.is_empty());
        }

        #[test]
        fn tc_49_workflow_accepts_model_nodes_and_preserves_destination_order() {
            let fixture = Fixture::new();
            fixture.write("SKILL.md", &skill_definition("  minimum-tier: T3\n"));
            fixture.write(
                "fixture.workflow.js",
                "export const meta = { phases: [\n  { title: 'Plan', model: 'sonnet' },\n  { title: 'Build', tier: 'T2' },\n  { title: 'Review', model: 'opus' },\n] }\n",
            );
            fixture.write("evals/cases.jsonl", &one_case(""));

            let artifact = fixture.load().unwrap();

            assert_eq!(
                artifact.required_destinations,
                vec![
                    TierDestination::WorkflowOrchestrator,
                    TierDestination::WorkflowNode {
                        node: "Plan".to_owned(),
                    },
                    TierDestination::WorkflowNode {
                        node: "Build".to_owned(),
                    },
                    TierDestination::WorkflowNode {
                        node: "Review".to_owned(),
                    },
                ]
            );
            assert_eq!(
                artifact.current_tiers,
                vec![
                    TierAssignment {
                        destination: TierDestination::WorkflowOrchestrator,
                        tier: Tier::T3,
                    },
                    TierAssignment {
                        destination: TierDestination::WorkflowNode {
                            node: "Build".to_owned(),
                        },
                        tier: Tier::T2,
                    },
                ]
            );
        }

        #[test]
        fn tc_49_workflow_rejects_invalid_destinations() {
            for (workflow, expected) in [
                (
                    "export const meta = { phases: [{ title: 'Plan', model: 'sonnet', tier: 'T2' }] }\n",
                    "both model and tier",
                ),
                (
                    "export const meta = { phases: [{ title: 'Plan', model: 'sonnet' }, { title: 'Plan', tier: 'T2' }] }\n",
                    "repeats destination",
                ),
                (
                    "export const meta = { phases: [{ model: 'sonnet' }] }\n",
                    "no named node destination",
                ),
                (
                    "export const meta = { phases: [{ title: '   ', tier: 'T2' }] }\n",
                    "no named node destination",
                ),
            ] {
                let fixture = Fixture::new();
                fixture.write("SKILL.md", &skill_definition("  minimum-tier: T3\n"));
                fixture.write("fixture.workflow.js", workflow);
                fixture.write("evals/cases.jsonl", &one_case(""));

                let message = invalid_message(fixture.load().unwrap_err());

                assert!(message.contains(expected), "{message:?} did not contain {expected:?}");
            }
        }

        #[test]
        fn tc_49_workflow_rejects_wrong_kind_destination_and_missing_floor() {
            let wrong_kind = Fixture::new();
            wrong_kind.write(
                "SKILL.md",
                &skill_definition("  minimum-tier: T3\n  target-tier: T2\n"),
            );
            wrong_kind.write(
                "fixture.workflow.js",
                "export const meta = { phases: [{ title: 'Plan', tier: 'T2' }] }\n",
            );
            wrong_kind.write("evals/cases.jsonl", &one_case(""));

            let message = invalid_message(wrong_kind.load().unwrap_err());
            assert!(message.contains("wrong artifact kind"));

            let missing_floor = Fixture::new();
            missing_floor.write("SKILL.md", &skill_definition(""));
            missing_floor.write(
                "fixture.workflow.js",
                "export const meta = { phases: [{ title: 'Plan', model: 'sonnet' }] }\n",
            );
            missing_floor.write("evals/cases.jsonl", &one_case(""));

            let message = invalid_message(missing_floor.load().unwrap_err());
            assert!(message.contains("missing orchestrator floor"));
        }

        #[test]
        fn tc_49_workflow_model_change_changes_revision() {
            let fixture = Fixture::new();
            fixture.write("SKILL.md", &skill_definition("  minimum-tier: T3\n"));
            fixture.write(
                "fixture.workflow.js",
                "export const meta = { phases: [{ title: 'Plan', model: 'sonnet' }] }\n",
            );
            fixture.write("evals/cases.jsonl", &one_case(""));
            let first = fixture.load().unwrap().revision;

            fixture.write(
                "fixture.workflow.js",
                "export const meta = { phases: [{ title: 'Plan', model: 'opus' }] }\n",
            );
            let second = fixture.load().unwrap().revision;

            assert_ne!(first, second);
            assert_eq!(second, fixture.load().unwrap().revision);
        }

        #[test]
        fn d_53_string_sentinel_loads_without_entering_normalized_case_data() {
            let fixture = Fixture::new();
            fixture.write("SKILL.md", &skill_definition(""));
            fixture.write(
                "evals/cases.jsonl",
                &one_case(",\"sentinel\":\"harness-only-value\""),
            );

            let artifact = fixture.load().unwrap();
            let normalized = serde_json::to_value(&artifact.cases[0]).unwrap();

            assert_eq!(artifact.cases.len(), 1);
            assert!(normalized.get("sentinel").is_none());
            assert!(!normalized.to_string().contains("harness-only-value"));
        }

        #[test]
        fn d_53_sentinel_change_changes_raw_case_revision() {
            let fixture = Fixture::new();
            fixture.write("SKILL.md", &skill_definition(""));
            fixture.write("evals/cases.jsonl", &one_case(",\"sentinel\":\"first\""));
            let first = fixture.load().unwrap().revision;

            fixture.write("evals/cases.jsonl", &one_case(",\"sentinel\":\"second\""));
            let second = fixture.load().unwrap().revision;

            assert_ne!(first, second);
        }

        #[test]
        fn d_53_non_string_sentinel_is_rejected() {
            for value in ["null", "true", "53", "[]", "{}"] {
                let fixture = Fixture::new();
                fixture.write("SKILL.md", &skill_definition(""));
                fixture.write(
                    "evals/cases.jsonl",
                    &one_case(&format!(",\"sentinel\":{value}")),
                );

                let message = invalid_message(fixture.load().unwrap_err());

                assert!(message.contains("invalid type"), "{message:?}");
            }
        }

        #[test]
        fn d_53_other_unknown_case_field_is_still_rejected() {
            let fixture = Fixture::new();
            fixture.write("SKILL.md", &skill_definition(""));
            fixture.write(
                "evals/cases.jsonl",
                &one_case(",\"sentinel\":\"known\",\"other\":\"unknown\""),
            );

            let message = invalid_message(fixture.load().unwrap_err());

            assert!(message.contains("unknown field `other`"), "{message:?}");
        }

        #[test]
        fn d_55_snapshot_and_checkpoints_are_discarded_from_normalized_cases() {
            let fixture = Fixture::new();
            fixture.write("SKILL.md", &skill_definition(""));
            fixture.write(
                "evals/cases.jsonl",
                &one_case(
                    ",\"snapshot\":{\"value\":\"snapshot-only-value\"},\"execution\":{\"drive\":{\"kind\":\"response\"},\"checkpoints\":[\"first-checkpoint\",\"second checkpoint\"]}",
                ),
            );

            let artifact = fixture.load().unwrap();
            let normalized = serde_json::to_value(&artifact.cases[0]).unwrap();
            let normalized_text = normalized.to_string();

            assert!(normalized.get("snapshot").is_none());
            assert!(normalized["execution"].get("checkpoints").is_none());
            assert!(!normalized_text.contains("snapshot-only-value"));
            assert!(!normalized_text.contains("first-checkpoint"));
            assert!(!normalized_text.contains("second checkpoint"));
        }

        #[test]
        fn d_55_harness_only_raw_field_changes_change_revision() {
            let fixture = Fixture::new();
            fixture.write("SKILL.md", &skill_definition(""));
            fixture.write(
                "evals/cases.jsonl",
                &one_case(
                    ",\"snapshot\":{\"value\":\"first\"},\"execution\":{\"drive\":{\"kind\":\"response\"},\"checkpoints\":[\"first\"]}",
                ),
            );
            let original = fixture.load().unwrap().revision;

            fixture.write(
                "evals/cases.jsonl",
                &one_case(
                    ",\"snapshot\":{\"value\":\"second\"},\"execution\":{\"drive\":{\"kind\":\"response\"},\"checkpoints\":[\"first\"]}",
                ),
            );
            let snapshot_revision = fixture.load().unwrap().revision;
            fixture.write(
                "evals/cases.jsonl",
                &one_case(
                    ",\"snapshot\":{\"value\":\"second\"},\"execution\":{\"drive\":{\"kind\":\"response\"},\"checkpoints\":[\"second\"]}",
                ),
            );
            let checkpoints_revision = fixture.load().unwrap().revision;

            assert_ne!(original, snapshot_revision);
            assert_ne!(snapshot_revision, checkpoints_revision);
        }

        #[test]
        fn d_55_snapshot_rejects_null_and_non_object_values() {
            for value in ["null", "true", "55", "\"snapshot\"", "[]"] {
                let fixture = Fixture::new();
                fixture.write("SKILL.md", &skill_definition(""));
                fixture.write(
                    "evals/cases.jsonl",
                    &one_case(&format!(",\"snapshot\":{value}")),
                );

                let message = invalid_message(fixture.load().unwrap_err());

                assert!(message.contains("invalid type"), "{message:?}");
            }
        }

        #[test]
        fn d_55_checkpoints_rejects_null_and_non_array_values() {
            for value in ["null", "true", "55", "\"checkpoint\"", "{}"] {
                let fixture = Fixture::new();
                fixture.write("SKILL.md", &skill_definition(""));
                fixture.write(
                    "evals/cases.jsonl",
                    &one_case(&format!(
                        ",\"execution\":{{\"drive\":{{\"kind\":\"response\"}},\"checkpoints\":{value}}}"
                    )),
                );

                let message = invalid_message(fixture.load().unwrap_err());

                assert!(message.contains("invalid type"), "{message:?}");
            }
        }

        #[test]
        fn d_55_checkpoints_rejects_non_string_entries() {
            for value in ["null", "true", "55", "{}", "[]"] {
                let fixture = Fixture::new();
                fixture.write("SKILL.md", &skill_definition(""));
                fixture.write(
                    "evals/cases.jsonl",
                    &one_case(&format!(
                        ",\"execution\":{{\"drive\":{{\"kind\":\"response\"}},\"checkpoints\":[{value}]}}"
                    )),
                );

                let message = invalid_message(fixture.load().unwrap_err());

                assert!(message.contains("invalid type"), "{message:?}");
            }
        }

        #[test]
        fn d_55_checkpoints_rejects_blank_entries() {
            for value in ["", " ", "\n", "\t"] {
                let fixture = Fixture::new();
                fixture.write("SKILL.md", &skill_definition(""));
                fixture.write(
                    "evals/cases.jsonl",
                    &one_case(&format!(
                        ",\"execution\":{{\"drive\":{{\"kind\":\"response\"}},\"checkpoints\":[{value:?}]}}"
                    )),
                );

                let message = invalid_message(fixture.load().unwrap_err());

                assert!(message.contains("empty execution checkpoint"), "{message:?}");
            }
        }

        #[test]
        fn d_55_other_unknown_fields_are_still_rejected() {
            for extra in [
                ",\"snapshot\":{},\"other\":\"unknown\"",
                ",\"execution\":{\"drive\":{\"kind\":\"response\"},\"checkpoints\":[],\"other\":\"unknown\"}",
            ] {
                let fixture = Fixture::new();
                fixture.write("SKILL.md", &skill_definition(""));
                fixture.write("evals/cases.jsonl", &one_case(extra));

                let message = invalid_message(fixture.load().unwrap_err());

                assert!(message.contains("unknown field `other`"), "{message:?}");
            }
        }
    };
}
