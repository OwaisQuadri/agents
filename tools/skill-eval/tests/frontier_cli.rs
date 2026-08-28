#[macro_export]
macro_rules! frontier_cli_tests {
    () => {
        mod frontier_command_tests {
            use std::cell::RefCell;
            use std::ffi::OsString;
            use std::fs;
            use std::path::{Path, PathBuf};
            use std::sync::atomic::{AtomicU64, Ordering};

            use super::{execute_frontier_command, parse_arguments};
            use $crate::model::{
                ArtifactDefinition, CandidateArtifact, CaseDefinition, CaseId, CheckResult,
                CliCommand, Decision, ExecutionDefinition, FrontierApplyReport,
                FrontierBaselineLedger, FrontierDecisionRequest, FrontierInspection, FrontierPlan,
                FrontierRunId, FrontierRunState, FrontierSuite, HarnessIdentity, JudgeInput,
                JudgeResult, ModelIdentity, OutputFormat, PromptJudgeRequest, PromptJudgeResult,
                RunEvent, RunId, SkillEvalError, Tier, TierAssignment, Timestamp, TrialKey,
                TrialRecord, TrialSelector,
            };
            use $crate::ports::{
                ArtifactSource, CandidateRunner, Clock, FrontierRuntime, HarnessResolver, Judge,
                ModelResolver, QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
            };

            static PATH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

            fn arguments(values: &[&str]) -> Vec<OsString> {
                values.iter().map(OsString::from).collect()
            }

            fn unique_repository_path(label: &str) -> PathBuf {
                let sequence = PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                PathBuf::from(format!(
                    "skill-eval-frontier-cli-{label}-{}-{sequence}",
                    std::process::id()
                ))
            }

            #[test]
            fn parses_every_frontier_command_into_exact_typed_values() {
                let preview = parse_arguments(&arguments(&[
                    "frontier-preview",
                    "--format",
                    "jsonl",
                    "--plan",
                    "plans/frontier.json",
                    "--runs-root",
                    "work/runs",
                ]))
                .unwrap();
                assert_eq!(preview.output_format, OutputFormat::JsonLines);
                assert_eq!(preview.runs_root, PathBuf::from("work/runs"));
                assert_eq!(
                    preview.command,
                    CliCommand::FrontierPreview {
                        plan_path: PathBuf::from("plans/frontier.json"),
                    }
                );

                assert_eq!(
                    parse_arguments(&arguments(&[
                        "frontier-start",
                        "--plan",
                        "plans/frontier.json",
                        "--run-id-file",
                        "work/run-id",
                    ]))
                    .unwrap()
                    .command,
                    CliCommand::FrontierStart {
                        plan_path: PathBuf::from("plans/frontier.json"),
                    }
                );
                assert_eq!(
                    parse_arguments(&arguments(&["frontier-resume", "--run", "frontier-run-1"]))
                        .unwrap()
                        .command,
                    CliCommand::FrontierResume {
                        run_id: FrontierRunId("frontier-run-1".to_owned()),
                    }
                );
                assert_eq!(
                    parse_arguments(&arguments(&[
                        "frontier-report",
                        "--run",
                        "frontier-run-1",
                        "--baseline",
                        "config/model-frontier-baseline.json",
                    ]))
                    .unwrap()
                    .command,
                    CliCommand::FrontierReport {
                        run_id: FrontierRunId("frontier-run-1".to_owned()),
                        baseline_path: Some(PathBuf::from(
                            "config/model-frontier-baseline.json"
                        )),
                    }
                );

                let inspect = parse_arguments(&arguments(&[
                    "frontier-inspect",
                    "--run",
                    "frontier-run-1",
                    "--provider",
                    "anthropic",
                    "--model",
                    "claude-sonnet-4.5",
                    "--tier",
                    "t3",
                    "--thinking",
                    "high",
                    "--artifact",
                    "agent-author",
                    "--case",
                    "case-17",
                    "--attempt",
                    "3",
                ]))
                .unwrap();
                let CliCommand::FrontierInspect { selector } = inspect.command else {
                    panic!("expected frontier inspection");
                };
                assert_eq!(selector.run_id.0, "frontier-run-1");
                assert_eq!(selector.provider, "anthropic");
                assert_eq!(selector.model, "claude-sonnet-4.5");
                assert_eq!(selector.tier, Tier::T3);
                assert_eq!(selector.thinking, "high");
                assert_eq!(selector.artifact.0, "agent-author");
                assert_eq!(selector.case, CaseId("case-17".to_owned()));
                assert_eq!(selector.attempt, 3);

                assert_eq!(
                    parse_arguments(&arguments(&[
                        "frontier-decide",
                        "--run",
                        "frontier-run-1",
                        "--accept",
                        "--reason",
                        "  owner approved  ",
                    ]))
                    .unwrap()
                    .command,
                    CliCommand::FrontierDecide {
                        request: FrontierDecisionRequest {
                            run_id: FrontierRunId("frontier-run-1".to_owned()),
                            decision: Decision::Accepted,
                            reason: "owner approved".to_owned(),
                        },
                    }
                );
                assert_eq!(
                    parse_arguments(&arguments(&[
                        "frontier-apply",
                        "--run",
                        "frontier-run-1",
                    ]))
                    .unwrap()
                    .command,
                    CliCommand::FrontierApply {
                        run_id: FrontierRunId("frontier-run-1".to_owned()),
                    }
                );
            }

            #[test]
            fn malformed_frontier_arguments_fail_before_dispatch() {
                let cases = [
                    vec!["frontier-preview"],
                    vec!["frontier-preview", "--plan", "/tmp/plan.json"],
                    vec!["frontier-preview", "--plan", "plans/frontier;touch.json"],
                    vec!["frontier-preview", "--plan", "plans/frontier\n.json"],
                    vec!["frontier-start", "--plan", "../plan.json"],
                    vec![
                        "frontier-start",
                        "--plan",
                        "plan.json",
                        "--plan",
                        "other.json",
                    ],
                    vec!["frontier-resume", "--run", ""],
                    vec!["frontier-resume", "--run", "run;touch"],
                    vec!["frontier-report", "--run", "run", "--baseline", "../base.json"],
                    vec!["frontier-report", "--run", "run", "--baseline", ""],
                    vec!["frontier-report", "--run", "run", "--baseline", "base|cat.json"],
                    vec![
                        "frontier-inspect",
                        "--run",
                        "run",
                        "--provider",
                        "anthropic",
                    ],
                    vec![
                        "frontier-inspect",
                        "--run",
                        "run",
                        "--provider",
                        "anthropic",
                        "--model",
                        "model",
                        "--tier",
                        "t6",
                        "--thinking",
                        "high",
                        "--artifact",
                        "skill",
                        "--case",
                        "case",
                        "--attempt",
                        "1",
                    ],
                    vec![
                        "frontier-inspect",
                        "--run",
                        "run",
                        "--provider",
                        "anthropic",
                        "--model",
                        "model",
                        "--tier",
                        "t1",
                        "--thinking",
                        "high",
                        "--artifact",
                        "skill",
                        "--case",
                        "case",
                        "--attempt",
                        "0",
                    ],
                    vec![
                        "frontier-decide",
                        "--run",
                        "run",
                        "--accept",
                        "--reject",
                        "--reason",
                        "no",
                    ],
                    vec!["frontier-decide", "--run", "run", "--accept", "--reason", "   "],
                    vec!["frontier-decide", "--run", "run", "--reason", "missing decision"],
                    vec!["frontier-apply", "--run", "run", "--format", "json"],
                    vec!["frontier-apply", "--run", "run", "--unknown", "value"],
                    vec!["frontier-apply", "--run", "run", "extra"],
                ];
                for case in cases {
                    let error = parse_arguments(&arguments(&case)).unwrap_err();
                    assert!(
                        matches!(error, SkillEvalError::InvalidArguments(_)),
                        "accepted malformed arguments: {case:?}"
                    );
                }
            }

            #[test]
            fn unsafe_run_id_files_fail_before_runtime_or_output() {
                let absolute = std::env::temp_dir().join(format!(
                    "skill-eval-frontier-cli-absolute-{}",
                    std::process::id()
                ));
                let cases = [
                    absolute.to_string_lossy().into_owned(),
                    "../run-id".to_owned(),
                    "run-id\nfile".to_owned(),
                    "run-id;touch".to_owned(),
                    "   ".to_owned(),
                    String::new(),
                ];

                for path in cases {
                    let is_path_present = Path::new(&path).exists();
                    let mut runtime = NoCallRuntime::default();
                    let mut output = Vec::new();
                    let parsed = parse_arguments(&arguments(&[
                        "frontier-start",
                        "--plan",
                        "plan.json",
                        "--run-id-file",
                        &path,
                    ]));
                    let error = match parsed {
                        Ok(request) => execute_frontier_command(
                            &request.command,
                            request.output_format,
                            &mut runtime,
                            &mut output,
                        )
                        .unwrap_err(),
                        Err(error) => error,
                    };

                    assert!(matches!(error, SkillEvalError::InvalidArguments(_)));
                    assert!(runtime.log.borrow().is_empty());
                    assert!(output.is_empty());
                    assert_eq!(Path::new(&path).exists(), is_path_present);
                }
            }

            #[test]
            fn dispatch_reaches_only_the_first_required_service_call() {
                let cases = [
                    (
                        CliCommand::FrontierPreview {
                            plan_path: PathBuf::from("plan.json"),
                        },
                        "load_plan:plan.json",
                    ),
                    (
                        CliCommand::FrontierStart {
                            plan_path: PathBuf::from("plan.json"),
                        },
                        "load_plan:plan.json",
                    ),
                    (
                        CliCommand::FrontierResume {
                            run_id: FrontierRunId("run-1".to_owned()),
                        },
                        "load_run:run-1",
                    ),
                    (
                        CliCommand::FrontierReport {
                            run_id: FrontierRunId("run-1".to_owned()),
                            baseline_path: None,
                        },
                        "load_run:run-1",
                    ),
                    (
                        CliCommand::FrontierInspect {
                            selector: selector(),
                        },
                        "load_run:run-1",
                    ),
                    (
                        CliCommand::FrontierDecide {
                            request: FrontierDecisionRequest {
                                run_id: FrontierRunId("run-1".to_owned()),
                                decision: Decision::Rejected,
                                reason: "not approved".to_owned(),
                            },
                        },
                        "load_run:run-1",
                    ),
                ];
                for (command, expected) in cases {
                    let mut runtime = NoCallRuntime::default();
                    let error = execute_frontier_command(
                        &command,
                        OutputFormat::Text,
                        &mut runtime,
                        &mut Vec::new(),
                    )
                    .unwrap_err();
                    assert!(matches!(error, SkillEvalError::NotFound(_)));
                    assert_eq!(runtime.log.into_inner(), [expected]);
                }
            }

            #[test]
            fn start_preserves_an_existing_run_id_file_before_service_dispatch() {
                let path = unique_repository_path("existing");
                fs::write(&path, b"existing\n").unwrap();
                let request = parse_arguments(&arguments(&[
                    "frontier-start",
                    "--plan",
                    "plan.json",
                    "--run-id-file",
                    path.to_str().unwrap(),
                ]))
                .unwrap();
                let mut runtime = NoCallRuntime::default();

                let error = execute_frontier_command(
                    &request.command,
                    request.output_format,
                    &mut runtime,
                    &mut Vec::new(),
                )
                .unwrap_err();

                assert!(matches!(error, SkillEvalError::InvalidArguments(_)));
                assert_eq!(fs::read(&path).unwrap(), b"existing\n");
                assert!(runtime.log.borrow().is_empty());
                fs::remove_file(path).unwrap();
            }

            #[test]
            fn apply_dispatches_and_non_frontier_rejections_make_only_expected_runtime_calls() {
                let mut runtime = NoCallRuntime::default();
                let error = execute_frontier_command(
                    &CliCommand::FrontierApply {
                        run_id: FrontierRunId("run-1".to_owned()),
                    },
                    OutputFormat::Text,
                    &mut runtime,
                    &mut Vec::new(),
                )
                .unwrap_err();
                assert!(matches!(error, SkillEvalError::NotFound(message) if message == "run"));
                assert_eq!(runtime.log.borrow().as_slice(), ["load_run:run-1"]);
                runtime.log.borrow_mut().clear();

                let error = execute_frontier_command(
                    &CliCommand::FrontierSuiteCheck {
                        proposal_path: PathBuf::from("proposal.json"),
                    },
                    OutputFormat::Text,
                    &mut runtime,
                    &mut Vec::new(),
                )
                .unwrap_err();
                assert!(matches!(error, SkillEvalError::InvalidArguments(_)));
                assert!(runtime.log.borrow().is_empty());
            }

            fn selector() -> $crate::model::FrontierTrialSelector {
                $crate::model::FrontierTrialSelector {
                    run_id: FrontierRunId("run-1".to_owned()),
                    provider: "anthropic".to_owned(),
                    model: "model".to_owned(),
                    tier: Tier::T1,
                    thinking: "high".to_owned(),
                    artifact: $crate::model::ArtifactName("skill".to_owned()),
                    case: CaseId("case".to_owned()),
                    attempt: 1,
                }
            }

            #[derive(Default)]
            struct NoCallRuntime {
                log: RefCell<Vec<String>>,
            }

            impl QualificationRuntime for NoCallRuntime {}

            impl FrontierRuntime for NoCallRuntime {
                fn load_frontier_plan(
                    &self,
                    path: &Path,
                ) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError> {
                    self.log
                        .borrow_mut()
                        .push(format!("load_plan:{}", path.display()));
                    Err(SkillEvalError::NotFound("plan".to_owned()))
                }

                fn next_frontier_run_id(&mut self) -> Result<FrontierRunId, SkillEvalError> {
                    panic!("unexpected run identifier allocation")
                }

                fn create_frontier(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    panic!("unexpected frontier create")
                }

                fn load_frontier(
                    &self,
                    run_id: &FrontierRunId,
                ) -> Result<FrontierRunState, SkillEvalError> {
                    self.log
                        .borrow_mut()
                        .push(format!("load_run:{}", run_id.0));
                    Err(SkillEvalError::NotFound("run".to_owned()))
                }

                fn save_frontier(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    panic!("unexpected frontier save")
                }

                fn save_frontier_trial(
                    &mut self,
                    _run_id: &FrontierRunId,
                    _trial: &TrialRecord,
                ) -> Result<(), SkillEvalError> {
                    panic!("unexpected frontier trial save")
                }

                fn inspect_frontier(
                    &self,
                    _selector: &$crate::model::FrontierTrialSelector,
                ) -> Result<FrontierInspection, SkillEvalError> {
                    panic!("unexpected frontier inspection")
                }

                fn load_frontier_baselines(
                    &self,
                    _path: &Path,
                ) -> Result<FrontierBaselineLedger, SkillEvalError> {
                    panic!("unexpected baseline load")
                }

                fn accept_frontier_baseline(
                    &mut self,
                    _state: &FrontierRunState,
                    _path: &Path,
                    _ledger: &FrontierBaselineLedger,
                ) -> Result<(), SkillEvalError> {
                    panic!("unexpected baseline write")
                }

                fn apply_frontier_routes(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<FrontierApplyReport, SkillEvalError> {
                    panic!("unexpected route publication")
                }
            }

            impl ArtifactSource for NoCallRuntime {
                fn load(&self, _root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
                    panic!("unexpected artifact load")
                }
            }

            impl ModelResolver for NoCallRuntime {
                fn candidates(&self, _tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    panic!("unexpected candidate resolution")
                }

                fn qualification_routes(
                    &self,
                    _tier: Tier,
                ) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    panic!("unexpected route resolution")
                }

                fn exact_candidate(
                    &self,
                    _requested: &ModelIdentity,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    panic!("unexpected exact candidate resolution")
                }

                fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
                    panic!("unexpected judge tier resolution")
                }

                fn judge(
                    &self,
                    _judge_tier: Tier,
                    _candidate: Option<&ModelIdentity>,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    panic!("unexpected judge resolution")
                }
            }

            impl HarnessResolver for NoCallRuntime {
                fn identity(
                    &self,
                    _artifact: &ArtifactDefinition,
                    _execution: &ExecutionDefinition,
                ) -> Result<HarnessIdentity, SkillEvalError> {
                    panic!("unexpected harness resolution")
                }
            }

            impl RunIdSource for NoCallRuntime {
                fn next(&mut self) -> Result<RunId, SkillEvalError> {
                    panic!("unexpected ordinary run identifier allocation")
                }
            }

            impl CandidateRunner for NoCallRuntime {
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
                    panic!("unexpected candidate execution")
                }
            }

            impl Verifier for NoCallRuntime {
                fn verify(
                    &mut self,
                    _case: &CaseDefinition,
                    _candidate: &CandidateArtifact,
                ) -> Result<Vec<CheckResult>, SkillEvalError> {
                    panic!("unexpected verification")
                }
            }

            impl Judge for NoCallRuntime {
                fn grade(
                    &mut self,
                    _model: &ModelIdentity,
                    _input: &JudgeInput,
                ) -> Result<JudgeResult, SkillEvalError> {
                    panic!("unexpected judge execution")
                }

                fn grade_prompt(
                    &mut self,
                    _model: &ModelIdentity,
                    _request: &PromptJudgeRequest,
                ) -> Result<PromptJudgeResult, SkillEvalError> {
                    panic!("unexpected prompt judge execution")
                }
            }

            impl RunStore for NoCallRuntime {
                fn append(
                    &mut self,
                    _run_id: &RunId,
                    _event: &RunEvent,
                ) -> Result<(), SkillEvalError> {
                    panic!("unexpected ordinary event write")
                }

                fn replay(
                    &self,
                    _run_id: &RunId,
                    _visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
                ) -> Result<(), SkillEvalError> {
                    panic!("unexpected ordinary event read")
                }

                fn find_trial(
                    &self,
                    _selector: &TrialSelector,
                ) -> Result<TrialRecord, SkillEvalError> {
                    panic!("unexpected ordinary trial read")
                }
            }

            impl Clock for NoCallRuntime {
                fn now(&self) -> Timestamp {
                    panic!("unexpected clock read")
                }
            }

            impl TierWriter for NoCallRuntime {
                fn write(
                    &mut self,
                    _artifact: &ArtifactDefinition,
                    _assignments: &[TierAssignment],
                ) -> Result<(), SkillEvalError> {
                    panic!("unexpected tier write")
                }
            }
        }
    };
}
