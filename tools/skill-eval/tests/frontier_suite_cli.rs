#[macro_export]
macro_rules! frontier_suite_cli_tests {
    () => {
mod frontier_suite_command_tests {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use sha2::{Digest, Sha256};

    use super::{
        execute_frontier_suite_command, parse_arguments, render_frontier_suite_inventory,
        render_frontier_suite_proposal, render_frontier_suite_publication,
    };
    use $crate::frontier_source::build_frontier_suite_proposal;
    use $crate::model::{
        ArtifactDefinition, ArtifactKind, ArtifactName, CaseDefinition, CaseDrive, CaseId,
        CliCommand, CommandDefinition, ExecutionDefinition, FrontierCaseGroup,
        FrontierCaseInventoryEntry, FrontierCaseKey, FrontierCaseReviewDecision,
        FrontierCaseReviewRecord, FrontierSuiteConstructionPlan, FrontierSuiteConstructionPolicy,
        FrontierSuiteInventory, FrontierSuiteProposal, FrontierSuiteProposalStatus,
        FrontierSuitePublication, FrontierSuiteReviewSet, OutputFormat, SkillEvalError, Tier,
        Timestamp,
    };
    use $crate::ports::{ArtifactSource, Clock, FrontierSuiteRuntime};
    use $crate::service::{
        apply_frontier_suite, check_frontier_suite, inventory_frontier_suite,
        propose_frontier_suite,
    };

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_all_frontier_suite_commands_and_global_formats() {
        let inventory = parse_arguments(&arguments(&[
            "frontier-suite-inventory",
            "--plan",
            "plans/suite.json",
            "--output",
            "work/inventory.json",
            "--format",
            "jsonl",
        ]))
        .unwrap();
        assert_eq!(inventory.output_format, OutputFormat::JsonLines);
        assert_eq!(
            inventory.command,
            CliCommand::FrontierSuiteInventory {
                plan_path: PathBuf::from("plans/suite.json"),
                output: PathBuf::from("work/inventory.json"),
            }
        );

        let propose = parse_arguments(&arguments(&[
            "frontier-suite-propose",
            "--reviews",
            "work/reviews.json",
            "--inventory",
            "work/inventory.json",
            "--plan",
            "plans/suite.json",
            "--output",
            "work/proposal.json",
        ]))
        .unwrap();
        assert_eq!(
            propose.command,
            CliCommand::FrontierSuitePropose {
                plan_path: PathBuf::from("plans/suite.json"),
                inventory_path: PathBuf::from("work/inventory.json"),
                review_set_path: PathBuf::from("work/reviews.json"),
                output: PathBuf::from("work/proposal.json"),
            }
        );

        let check = parse_arguments(&arguments(&[
            "frontier-suite-check",
            "--format",
            "text",
            "--proposal",
            "work/proposal.json",
        ]))
        .unwrap();
        assert_eq!(
            check.command,
            CliCommand::FrontierSuiteCheck {
                proposal_path: PathBuf::from("work/proposal.json"),
            }
        );

        let apply = parse_arguments(&arguments(&[
            "frontier-suite-apply",
            "--proposal",
            "work/proposal.json",
            "--output",
            "config/model-frontier-suite.json",
        ]))
        .unwrap();
        assert_eq!(
            apply.command,
            CliCommand::FrontierSuiteApply {
                proposal_path: PathBuf::from("work/proposal.json"),
                output: PathBuf::from("config/model-frontier-suite.json"),
            }
        );
    }

    #[test]
    fn rejects_malformed_frontier_suite_arguments() {
        let cases = [
            vec!["frontier-suite-inventory"],
            vec![
                "frontier-suite-inventory",
                "--plan",
                "plan.json",
                "--plan",
                "other.json",
                "--output",
                "out.json",
            ],
            vec![
                "frontier-suite-propose",
                "--plan",
                "plan.json",
                "--inventory",
                "inventory.json",
                "--reviews",
                "",
                "--output",
                "proposal.json",
            ],
            vec!["frontier-suite-check", "--proposal", "/tmp/proposal.json"],
            vec!["frontier-suite-check", "--proposal", "../proposal.json"],
            vec![
                "frontier-suite-apply",
                "--proposal",
                "proposal.json",
                "--output",
                "suite.json",
                "extra",
            ],
            vec![
                "frontier-suite-apply",
                "--proposal",
                "proposal.json",
                "--unknown",
                "value",
                "--output",
                "suite.json",
            ],
            vec![
                "frontier-suite-check",
                "--proposal",
                "proposal.json",
                "--format",
                "yaml",
            ],
        ];
        for case in cases {
            let error = parse_arguments(&arguments(&case)).unwrap_err();
            assert!(
                matches!(error, SkillEvalError::InvalidArguments(_)),
                "{case:?}"
            );
        }
    }

    #[test]
    fn service_paths_call_runtime_in_exact_order() {
        let mut runtime = FakeSuiteRuntime::new();
        let inventory = inventory_frontier_suite(
            Path::new("plan.json"),
            Path::new("inventory.json"),
            &mut runtime,
        )
        .unwrap();
        assert_eq!(inventory.cases.len(), 150);
        assert_eq!(
            runtime.take_log(),
            [
                "load_plan:plan.json",
                "load_artifact:skills/a",
                "now",
                "save_inventory:inventory.json",
            ]
        );

        let proposal = propose_frontier_suite(
            Path::new("plan.json"),
            Path::new("inventory.json"),
            Path::new("reviews.json"),
            Path::new("proposal.json"),
            &mut runtime,
        )
        .unwrap();
        assert_eq!(proposal.status, FrontierSuiteProposalStatus::Ready);
        assert_eq!(
            runtime.take_log(),
            [
                "load_plan:plan.json",
                "load_inventory:inventory.json",
                "load_reviews:reviews.json",
                "save_proposal:proposal.json",
            ]
        );

        check_frontier_suite(Path::new("proposal.json"), &runtime).unwrap();
        assert_eq!(runtime.take_log(), ["load_proposal:proposal.json"]);

        apply_frontier_suite(
            Path::new("proposal.json"),
            Path::new("suite.json"),
            &mut runtime,
        )
        .unwrap();
        assert_eq!(
            runtime.take_log(),
            ["load_proposal:proposal.json", "now", "apply:suite.json"]
        );
        assert_eq!(runtime.publication_writes.get(), 1);
    }

    #[test]
    fn dispatches_each_suite_command_once_and_rejects_non_suite_commands() {
        let mut runtime = FakeSuiteRuntime::new();
        let commands = [
            CliCommand::FrontierSuiteInventory {
                plan_path: PathBuf::from("plan.json"),
                output: PathBuf::from("inventory.json"),
            },
            CliCommand::FrontierSuitePropose {
                plan_path: PathBuf::from("plan.json"),
                inventory_path: PathBuf::from("inventory.json"),
                review_set_path: PathBuf::from("reviews.json"),
                output: PathBuf::from("proposal.json"),
            },
            CliCommand::FrontierSuiteCheck {
                proposal_path: PathBuf::from("proposal.json"),
            },
            CliCommand::FrontierSuiteApply {
                proposal_path: PathBuf::from("proposal.json"),
                output: PathBuf::from("suite.json"),
            },
        ];
        for command in commands {
            let mut output = Vec::new();
            execute_frontier_suite_command(
                &command,
                OutputFormat::JsonLines,
                &mut runtime,
                &mut output,
            )
            .unwrap();
            assert_eq!(output.iter().filter(|byte| **byte == b'\n').count(), 1);
        }
        assert_eq!(runtime.inventory_writes.get(), 1);
        assert_eq!(runtime.proposal_writes.get(), 1);
        assert_eq!(runtime.publication_writes.get(), 1);

        runtime.take_log();
        let error = execute_frontier_suite_command(
            &CliCommand::Report {
                run_id: $crate::model::RunId("run-1".to_owned()),
            },
            OutputFormat::Text,
            &mut runtime,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(
            matches!(error, SkillEvalError::InvalidArguments(message) if message == "command is not a complete-bank suite command")
        );
        assert!(runtime.take_log().is_empty());
    }

    #[test]
    fn renders_deterministic_text_and_single_value_json_lines() {
        let runtime = FakeSuiteRuntime::new();
        let mut first = Vec::new();
        let mut second = Vec::new();
        render_frontier_suite_inventory(&runtime.inventory, OutputFormat::Text, &mut first)
            .unwrap();
        render_frontier_suite_inventory(&runtime.inventory, OutputFormat::Text, &mut second)
            .unwrap();
        assert_eq!(first, second);
        let inventory_text = String::from_utf8(first).unwrap();
        assert!(
            inventory_text.starts_with(
                "version: 1\ngenerated at: 2026-08-27T12:00:00-0400\ncase count: 150\n"
            )
        );
        assert!(inventory_text.contains("case: skills/a@revision-1 case-000"));

        let mut proposal_json = Vec::new();
        render_frontier_suite_proposal(
            &runtime.proposal,
            OutputFormat::JsonLines,
            &mut proposal_json,
        )
        .unwrap();
        let mut expected = serde_json::to_vec(&runtime.proposal).unwrap();
        expected.push(b'\n');
        assert_eq!(proposal_json, expected);

        let mut publication_text = Vec::new();
        render_frontier_suite_publication(
            &runtime.publication(),
            OutputFormat::Text,
            &mut publication_text,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(publication_text).unwrap(),
            format!(
                "proposal digest: {}\nsuite path: suite.json\nsuite digest: {}\npublished at: 2026-08-27T12:00:00-0400\n",
                "a".repeat(64),
                "b".repeat(64)
            )
        );
    }

    #[test]
    fn blocked_text_reports_capacity_before_suite_detail() {
        let runtime = FakeSuiteRuntime::new();
        let proposal = runtime.blocked_proposal();
        let mut output = Vec::new();
        render_frontier_suite_proposal(&proposal, OutputFormat::Text, &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.starts_with("status: Blocked\n"));
        assert!(text.contains(
            "T5: accepted 8, required 30, shortfall 22, duplicates 0, rejects 0, complete false"
        ));
        let capacity = text.find("T5: accepted 8").unwrap();
        let weights = text.find("weights:").unwrap();
        let cases = text.find("case T1:").unwrap();
        assert!(capacity < weights && weights < cases);
    }

    #[test]
    fn blocked_apply_returns_before_publication_write() {
        let mut runtime = FakeSuiteRuntime::new();
        runtime.loaded_proposal = runtime.blocked_proposal();
        let error = apply_frontier_suite(
            Path::new("proposal.json"),
            Path::new("suite.json"),
            &mut runtime,
        )
        .unwrap_err();
        assert!(
            matches!(error, SkillEvalError::InvalidArguments(message) if message == "frontier proposal is blocked")
        );
        assert_eq!(runtime.publication_writes.get(), 0);
        assert_eq!(
            runtime.take_log(),
            ["load_proposal:proposal.json", "now", "apply:suite.json"]
        );
    }

    #[test]
    fn broken_writers_return_errors() {
        let runtime = FakeSuiteRuntime::new();
        for result in [
            render_frontier_suite_inventory(
                &runtime.inventory,
                OutputFormat::Text,
                &mut BrokenWriter,
            ),
            render_frontier_suite_proposal(
                &runtime.proposal,
                OutputFormat::JsonLines,
                &mut BrokenWriter,
            ),
            render_frontier_suite_publication(
                &runtime.publication(),
                OutputFormat::Text,
                &mut BrokenWriter,
            ),
        ] {
            assert!(result.is_err());
        }
    }

    struct BrokenWriter;

    impl Write for BrokenWriter {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("broken writer"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("broken writer"))
        }
    }

    struct FakeSuiteRuntime {
        plan: FrontierSuiteConstructionPlan,
        artifact: ArtifactDefinition,
        inventory: FrontierSuiteInventory,
        reviews: FrontierSuiteReviewSet,
        proposal: FrontierSuiteProposal,
        loaded_proposal: FrontierSuiteProposal,
        log: RefCell<Vec<String>>,
        inventory_writes: Cell<u32>,
        proposal_writes: Cell<u32>,
        publication_writes: Cell<u32>,
    }

    impl FakeSuiteRuntime {
        fn new() -> Self {
            let plan = plan();
            let artifact = artifact(150);
            let inventory = inventory(150);
            let reviews = reviews(&inventory);
            let proposal = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();
            Self {
                plan,
                artifact,
                inventory,
                reviews,
                loaded_proposal: proposal.clone(),
                proposal,
                log: RefCell::new(Vec::new()),
                inventory_writes: Cell::new(0),
                proposal_writes: Cell::new(0),
                publication_writes: Cell::new(0),
            }
        }

        fn take_log(&self) -> Vec<String> {
            std::mem::take(&mut *self.log.borrow_mut())
        }

        fn blocked_proposal(&self) -> FrontierSuiteProposal {
            let inventory = inventory(128);
            let reviews = reviews(&inventory);
            build_frontier_suite_proposal(&self.plan, &inventory, &reviews).unwrap()
        }

        fn publication(&self) -> FrontierSuitePublication {
            FrontierSuitePublication {
                proposal_sha256: "a".repeat(64),
                suite_path: PathBuf::from("suite.json"),
                suite_sha256: "b".repeat(64),
                published_at: timestamp(),
            }
        }

        fn record(&self, value: impl Into<String>) {
            self.log.borrow_mut().push(value.into());
        }
    }

    impl ArtifactSource for FakeSuiteRuntime {
        fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
            self.record(format!("load_artifact:{}", root.display()));
            assert_eq!(root, Path::new("skills/a"));
            Ok(self.artifact.clone())
        }
    }

    impl Clock for FakeSuiteRuntime {
        fn now(&self) -> Timestamp {
            self.record("now");
            timestamp()
        }
    }

    impl FrontierSuiteRuntime for FakeSuiteRuntime {
        fn load_frontier_suite_construction_plan(
            &self,
            path: &Path,
        ) -> Result<FrontierSuiteConstructionPlan, SkillEvalError> {
            self.record(format!("load_plan:{}", path.display()));
            Ok(self.plan.clone())
        }

        fn load_frontier_suite_inventory(
            &self,
            path: &Path,
        ) -> Result<FrontierSuiteInventory, SkillEvalError> {
            self.record(format!("load_inventory:{}", path.display()));
            Ok(self.inventory.clone())
        }

        fn load_frontier_suite_review_set(
            &self,
            path: &Path,
        ) -> Result<FrontierSuiteReviewSet, SkillEvalError> {
            self.record(format!("load_reviews:{}", path.display()));
            Ok(self.reviews.clone())
        }

        fn load_frontier_suite_proposal(
            &self,
            path: &Path,
        ) -> Result<FrontierSuiteProposal, SkillEvalError> {
            self.record(format!("load_proposal:{}", path.display()));
            Ok(self.loaded_proposal.clone())
        }

        fn save_frontier_suite_inventory(
            &mut self,
            path: &Path,
            inventory: &FrontierSuiteInventory,
        ) -> Result<(), SkillEvalError> {
            self.record(format!("save_inventory:{}", path.display()));
            assert_eq!(inventory.cases.len(), 150);
            self.inventory_writes.set(self.inventory_writes.get() + 1);
            Ok(())
        }

        fn save_frontier_suite_proposal(
            &mut self,
            path: &Path,
            proposal: &FrontierSuiteProposal,
        ) -> Result<(), SkillEvalError> {
            self.record(format!("save_proposal:{}", path.display()));
            assert_eq!(proposal.status, FrontierSuiteProposalStatus::Ready);
            self.proposal_writes.set(self.proposal_writes.get() + 1);
            Ok(())
        }

        fn apply_frontier_suite_proposal(
            &mut self,
            proposal: &FrontierSuiteProposal,
            output: &Path,
            _published_at: &Timestamp,
        ) -> Result<FrontierSuitePublication, SkillEvalError> {
            self.record(format!("apply:{}", output.display()));
            if proposal.status == FrontierSuiteProposalStatus::Blocked {
                return Err(SkillEvalError::InvalidArguments(
                    "frontier proposal is blocked".to_owned(),
                ));
            }
            self.publication_writes
                .set(self.publication_writes.get() + 1);
            Ok(self.publication())
        }
    }

    fn plan() -> FrontierSuiteConstructionPlan {
        FrontierSuiteConstructionPlan {
            version: 1,
            artifact_roots: vec![PathBuf::from("skills/a")],
            policy: FrontierSuiteConstructionPolicy {
                required_tiers: vec![Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5],
                minimum_unique_cases_per_tier: 30,
                minimum_reviewers_per_case: 2,
                group_weights_basis_points: weights(),
                is_unanimous_eligibility_required: true,
                is_cross_tier_reuse_allowed: false,
                is_calibration_anchor_counted_toward_minimum: false,
            },
        }
    }

    fn artifact(count: usize) -> ArtifactDefinition {
        ArtifactDefinition {
            name: ArtifactName("a".to_owned()),
            kind: ArtifactKind::Skill,
            root: PathBuf::from("skills/a"),
            revision: "revision-1".to_owned(),
            required_destinations: Vec::new(),
            current_tiers: Vec::new(),
            cases: (0..count)
                .map(|index| CaseDefinition {
                    id: CaseId(format!("case-{index:03}")),
                    input: "input".to_owned(),
                    expect: "expect".to_owned(),
                    source: "fixture".to_owned(),
                    is_holdout: false,
                    support_files: Vec::new(),
                    execution: ExecutionDefinition {
                        drive: drive(),
                        allowed_tools: Vec::new(),
                        timeout_seconds: 30,
                    },
                })
                .collect(),
        }
    }

    fn inventory(count: usize) -> FrontierSuiteInventory {
        FrontierSuiteInventory {
            version: 1,
            generated_at: timestamp(),
            cases: (0..count)
                .map(|index| FrontierCaseInventoryEntry {
                    key: key(index),
                    drive: drive(),
                    is_holdout: false,
                })
                .collect(),
        }
    }

    fn reviews(inventory: &FrontierSuiteInventory) -> FrontierSuiteReviewSet {
        FrontierSuiteReviewSet {
            version: 1,
            inventory_sha256: digest(inventory),
            records: inventory
                .cases
                .iter()
                .flat_map(|entry| {
                    ["panel-a", "panel-b"].map(|reviewer| FrontierCaseReviewRecord {
                        key: entry.key.clone(),
                        reviewer: reviewer.to_owned(),
                        reviewed_at: timestamp(),
                        decision: FrontierCaseReviewDecision::Eligible {
                            relative_difficulty_basis_points: u16::try_from(
                                entry.key.case.0[5..].parse::<usize>().unwrap() + 1,
                            )
                            .unwrap(),
                            group: group_for_case(&entry.key.case),
                            is_confirmation: false,
                            evidence: vec!["review evidence".to_owned()],
                        },
                    })
                })
                .collect(),
        }
    }

    fn key(index: usize) -> FrontierCaseKey {
        FrontierCaseKey {
            artifact_path: PathBuf::from("skills/a"),
            artifact_revision: "revision-1".to_owned(),
            case: CaseId(format!("case-{index:03}")),
        }
    }

    fn group_for_case(case: &CaseId) -> FrontierCaseGroup {
        match case.0[5..].parse::<usize>().unwrap() % 4 {
            0 => FrontierCaseGroup::Normal,
            1 => FrontierCaseGroup::Edge,
            2 => FrontierCaseGroup::Adversarial,
            _ => FrontierCaseGroup::Critical,
        }
    }

    fn weights() -> BTreeMap<FrontierCaseGroup, u16> {
        [
            (FrontierCaseGroup::Normal, 4_000),
            (FrontierCaseGroup::Edge, 2_000),
            (FrontierCaseGroup::Adversarial, 2_000),
            (FrontierCaseGroup::Critical, 2_000),
        ]
        .into_iter()
        .collect()
    }

    fn drive() -> CaseDrive {
        CaseDrive::ExistingHarness {
            command: CommandDefinition {
                program: "true".to_owned(),
                arguments: Vec::new(),
                working_directory: None,
            },
        }
    }

    fn digest<T: serde::Serialize>(value: &T) -> String {
        Sha256::digest(serde_json::to_vec(value).unwrap())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn timestamp() -> Timestamp {
        Timestamp("2026-08-27T12:00:00-0400".to_owned())
    }
}
    };
}
