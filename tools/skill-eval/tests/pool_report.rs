// TODO(AGNT-0032.T107): Prove per-model thinking progress and report evidence.
#[macro_export]
macro_rules! pool_report_tests {
    () => {
        mod pool_report_command_line {
            use std::cell::Cell;
            use std::collections::BTreeMap;
            use std::ffi::OsString;
            use std::path::PathBuf;

            use super::{
                execute_command, parse_arguments, render_event, render_pool_report,
                resolve_exact_candidate,
            };
            use $crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, CliCommand, CliRequest,
                ConfidenceInterval, ModelIdentity, OutputFormat, PoolChildRun, PoolChildStatus,
                PoolEntrant, PoolEntrantEvidence, PoolPauseReason, PoolPolicy,
                PoolRunConfiguration, PoolRunId, PoolRunState, PoolRunStatus, PoolStage,
                RankedPool, RunEvent, RunId, SkillEvalError, Tier, Timestamp, TrialUsage,
            };
            use $crate::models::ConfiguredModelResolver;
            use $crate::ports::PoolStore;
            use $crate::testing::{FakeQualificationRuntime, TemporaryRoot};

            struct CountingStore {
                state: PoolRunState,
                reads: Cell<usize>,
                writes: usize,
            }

            impl PoolStore for CountingStore {
                fn create_pool(&mut self, _state: &PoolRunState) -> Result<(), SkillEvalError> {
                    self.writes += 1;
                    Ok(())
                }

                fn load_pool(&self, _run_id: &PoolRunId) -> Result<PoolRunState, SkillEvalError> {
                    self.reads.set(self.reads.get() + 1);
                    Ok(self.state.clone())
                }

                fn save_pool(&mut self, _state: &PoolRunState) -> Result<(), SkillEvalError> {
                    self.writes += 1;
                    Ok(())
                }
            }

            #[test]
            fn pool_report_service_reads_once_without_writing() {
                let expected = complete_state();
                let store = CountingStore {
                    state: expected.clone(),
                    reads: Cell::new(0),
                    writes: 0,
                };

                let report = $crate::service::build_pool_report(
                    &PoolRunId("pool-report".to_owned()),
                    &store,
                )
                .unwrap();

                assert_eq!(report, expected);
                assert_eq!(store.reads.get(), 1);
                assert_eq!(store.writes, 0);
            }

            #[test]
            fn pool_text_report_exposes_complete_decision_evidence() {
                let state = complete_state();
                let mut output = Vec::new();

                render_pool_report(&state, OutputFormat::Text, &mut output).unwrap();

                let text = String::from_utf8(output).unwrap();
                for expected in [
                    "pool pool-report: AwaitingDecision",
                    "selected tiers: T2",
                    "floors: score >= 8, reliability >= 95.00%",
                    "spending: $1.250000 spent / $10.000000 limit",
                    "control excluded: control/control-exact",
                    "exact candidate host anthropic/claude-first-party",
                    "catalog 2026-08-24T11:00:00-0400",
                    "exact candidate openrouter/anthropic/claude-proxy",
                    "judge host openai-codex/judge-exact",
                    "candidate usage 100 input, 50 output, $0.400000, 1200 ms",
                    "judge usage 30 input, 20 output, $0.100000, 300 ms",
                    "failures 1 (25.00%)",
                    "child child-calibration-0: T2 entrant 1 Calibration Completed",
                    "T2 calibration: 2/3 passing; full qualification: 2/2 passing; complete: true",
                    "T2 promoted pair: 1.",
                    "T2 ranked order: 1.",
                    "owner state: result-ready",
                ] {
                    assert!(text.contains(expected), "missing {expected:?} in {text}");
                }
                assert!(!text.contains("control/control-exact (T1; off); 1."));
            }

            #[test]
            fn pool_json_line_preserves_the_complete_state() {
                let state = complete_state();
                let mut output = Vec::new();

                render_pool_report(&state, OutputFormat::JsonLines, &mut output).unwrap();

                assert_eq!(output.iter().filter(|byte| **byte == b'\n').count(), 1);
                let decoded: PoolRunState = serde_json::from_slice(&output).unwrap();
                assert_eq!(decoded, state);
                assert_eq!(decoded.pools[0].calibration.len(), 3);
                assert_eq!(decoded.pools[0].qualification.len(), 2);
            }

            #[test]
            fn paused_and_incomplete_reports_include_resume_and_owner_state() {
                let mut state = complete_state();
                state.status = PoolRunStatus::Paused;
                state.pools[0].is_complete = false;
                state.pools[0].qualification.clear();
                state.pools[0].ranked.clear();
                state.pause = Some(PoolPauseReason::SpendingLimit {
                    spent_millionths_of_dollar: 1_250_000,
                    limit_millionths_of_dollar: 10_000_000,
                });
                let mut output = Vec::new();

                render_pool_report(&state, OutputFormat::Text, &mut output).unwrap();

                let text = String::from_utf8(output).unwrap();
                assert!(text.contains("pause reason: SpendingLimit"));
                assert!(text.contains("resume: skill-eval pool-resume --run pool-report"));
                assert!(text.contains("owner state: not-result-ready"));
                assert!(text.contains("full qualification: 0/0 passing; complete: false"));
            }

            #[test]
            fn concrete_runtime_exact_candidate_delegation_preserves_identity() {
                let root = TemporaryRoot::new("pool-cli-models");
                let configuration = root.path().join("model-tiers.json");
                std::fs::write(
                    &configuration,
                    include_str!("../../../config/model-tiers.json"),
                )
                .unwrap();
                let resolver = ConfiguredModelResolver::load(
                    &configuration,
                    include_str!("../tests/fixtures/models/catalog-all.txt"),
                )
                .unwrap();
                let requested = candidate(Tier::T5, "anthropic", "claude-opus-5");

                let effective = resolve_exact_candidate(&resolver, &requested).unwrap();

                assert_eq!(effective, requested);
            }

            #[test]
            fn pool_start_and_report_dispatch_and_write_the_identifier_after_creation() {
                let root = TemporaryRoot::new("pool-cli-start");
                let mut runtime = FakeQualificationRuntime::new(&root);
                let run_id_path = root.path().join("pool-id");
                let mut request = parse_arguments(&os_arguments(&[
                    "pool-qualify",
                    "--plan",
                    "plan.json",
                    "--artifact",
                    "tools/exam",
                    "--tiers",
                    "T2",
                    "--dry-run",
                    "--run-id-file",
                    run_id_path.to_str().unwrap(),
                    "--format",
                    "jsonl",
                ]))
                .unwrap();
                let CliCommand::PoolQualify {
                    request: pool_request,
                } = &mut request.command
                else {
                    unreachable!();
                };
                pool_request.artifact_roots = vec![runtime.artifact().root.clone()];
                let mut output = Vec::new();

                execute_command(request, &mut runtime, &mut output).unwrap();

                assert_eq!(
                    std::fs::read_to_string(&run_id_path).unwrap(),
                    "pool-test\n"
                );
                assert!(
                    std::fs::read_dir(root.path())
                        .unwrap()
                        .filter_map(Result::ok)
                        .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp"))
                );
                let state = runtime
                    .load_pool(&PoolRunId("pool-test".to_owned()))
                    .unwrap();
                assert_eq!(state.status, PoolRunStatus::Pending);
                assert_eq!(state.selected_tiers, vec![Tier::T2]);

                let mut report_output = Vec::new();
                execute_command(
                    CliRequest {
                        runs_root: root.path().join("runs"),
                        output_format: OutputFormat::JsonLines,
                        command: CliCommand::PoolReport {
                            run_id: PoolRunId("pool-test".to_owned()),
                        },
                    },
                    &mut runtime,
                    &mut report_output,
                )
                .unwrap();
                let final_line = report_output.split(|byte| *byte == b'\n').next().unwrap();
                let reported: PoolRunState = serde_json::from_slice(final_line).unwrap();
                assert_eq!(reported, state);
            }

            #[test]
            fn pool_run_id_file_is_not_written_before_pool_creation() {
                let root = TemporaryRoot::new("pool-cli-run-id-failure");
                let mut runtime = FakeQualificationRuntime::new(&root);
                let run_id_path = root.path().join("failed-pool-id");
                let request = parse_arguments(&os_arguments(&[
                    "pool-qualify",
                    "--plan",
                    "plan.json",
                    "--artifact",
                    "missing/artifact",
                    "--dry-run",
                    "--run-id-file",
                    run_id_path.to_str().unwrap(),
                ]))
                .unwrap();

                assert!(execute_command(request, &mut runtime, &mut Vec::new()).is_err());
                assert!(!run_id_path.exists());
            }

            #[test]
            fn pool_resume_dispatch_preserves_a_spending_pause() {
                let root = TemporaryRoot::new("pool-cli-resume");
                let mut runtime = FakeQualificationRuntime::new(&root);
                let start = CliRequest {
                    runs_root: root.path().join("runs"),
                    output_format: OutputFormat::Text,
                    command: CliCommand::PoolQualify {
                        request: $crate::model::PoolQualifyRequest {
                            plan_path: PathBuf::from("plan.json"),
                            artifact_roots: vec![runtime.artifact().root.clone()],
                            selected_tiers: vec![Tier::T2],
                            is_dry_run: true,
                        },
                    },
                };
                execute_command(start, &mut runtime, &mut Vec::new()).unwrap();
                let mut state = runtime
                    .load_pool(&PoolRunId("pool-test".to_owned()))
                    .unwrap();
                state.status = PoolRunStatus::Paused;
                state.spent_millionths_of_dollar = 10_000_000;
                state.pause = Some(PoolPauseReason::SpendingLimit {
                    spent_millionths_of_dollar: 10_000_000,
                    limit_millionths_of_dollar: 10_000_000,
                });
                runtime.save_pool(&state).unwrap();
                let mut output = Vec::new();

                execute_command(
                    CliRequest {
                        runs_root: root.path().join("runs"),
                        output_format: OutputFormat::Text,
                        command: CliCommand::PoolResume {
                            run_id: PoolRunId("pool-test".to_owned()),
                        },
                    },
                    &mut runtime,
                    &mut output,
                )
                .unwrap();

                let text = String::from_utf8(output).unwrap();
                assert!(text.contains("pool pool-test: Paused"));
                assert!(text.contains("resume: skill-eval pool-resume --run pool-test"));
            }

            #[test]
            fn ordinary_text_event_output_is_unchanged() {
                let event = RunEvent::PoolChildCompleted {
                    at: Timestamp("2026-08-24T12:00:00-0400".to_owned()),
                    artifact: ArtifactName("exam".to_owned()),
                    tier: Tier::T2,
                };
                let mut output = Vec::new();

                render_event(&event, OutputFormat::Text, &mut output).unwrap();

                assert_eq!(output, b"exam T2 pool child complete\n");
            }

            fn complete_state() -> PoolRunState {
                let first_party = candidate(Tier::T2, "anthropic", "claude-first-party");
                let proxy = candidate(Tier::T2, "openrouter", "anthropic/claude-proxy");
                let third = candidate(Tier::T2, "google", "gemini-exact");
                let entrants = vec![
                    entrant(first_party.clone(), "2026-08-24T11:00:00-0400"),
                    entrant(proxy.clone(), "2026-08-24T11:01:00-0400"),
                    entrant(third.clone(), "2026-08-24T11:02:00-0400"),
                ];
                let mut configured = BTreeMap::new();
                configured.insert(Tier::T2, entrants);
                let calibration = vec![
                    evidence(PoolStage::Calibration, first_party.clone(), true, 0),
                    evidence(PoolStage::Calibration, proxy.clone(), true, 1),
                    evidence(PoolStage::Calibration, third, false, 2),
                ];
                let qualification = vec![
                    evidence(PoolStage::Qualification, first_party.clone(), true, 0),
                    evidence(PoolStage::Qualification, proxy.clone(), true, 0),
                ];
                PoolRunState {
                    configuration: PoolRunConfiguration {
                        run_id: PoolRunId("pool-report".to_owned()),
                        created_at: Timestamp("2026-08-24T12:00:00-0400".to_owned()),
                        artifacts: vec![ArtifactDefinition {
                            name: ArtifactName("model-calibration".to_owned()),
                            kind: ArtifactKind::Skill,
                            root: PathBuf::from("tools/exam"),
                            revision: "revision-1".to_owned(),
                            required_destinations: Vec::new(),
                            current_tiers: Vec::new(),
                            cases: Vec::new(),
                        }],
                        entrants: configured,
                        control: candidate(Tier::T1, "control", "control-exact"),
                        policy: PoolPolicy {
                            calibration_repeats_per_case: 1,
                            qualification_repeats_per_case: 3,
                            promotion_count: 2,
                            minimum_score: 8,
                            minimum_reliability_basis_points: 9_500,
                            maximum_catalog_age_seconds: 3_600,
                            spending_limit_millionths_of_dollar: 10_000_000,
                            is_provider_limit_enforced: true,
                        },
                    },
                    selected_tiers: vec![Tier::T2],
                    status: PoolRunStatus::AwaitingDecision,
                    child_runs: vec![
                        child(0, PoolStage::Calibration, PoolChildStatus::Completed),
                        child(0, PoolStage::Qualification, PoolChildStatus::Completed),
                        child(1, PoolStage::Calibration, PoolChildStatus::Completed),
                        child(1, PoolStage::Qualification, PoolChildStatus::Completed),
                        child(2, PoolStage::Calibration, PoolChildStatus::Completed),
                        child(2, PoolStage::Qualification, PoolChildStatus::Skipped),
                    ],
                    pools: vec![RankedPool {
                        tier: Tier::T2,
                        calibration,
                        promoted: vec![first_party.clone(), proxy.clone()],
                        qualification,
                        ranked: vec![proxy, first_party],
                        is_complete: true,
                    }],
                    pause: None,
                    spent_millionths_of_dollar: 1_250_000,
                }
            }

            fn entrant(model: ModelIdentity, observed: &str) -> PoolEntrant {
                PoolEntrant {
                    model,
                    catalog_observed_at: Timestamp(observed.to_owned()),
                }
            }

            fn child(index: u8, stage: PoolStage, status: PoolChildStatus) -> PoolChildRun {
                PoolChildRun {
                    tier: Tier::T2,
                    entrant_index: index,
                    stage,
                    run_id: RunId(format!(
                        "child-{}-{index}",
                        match stage {
                            PoolStage::Calibration => "calibration",
                            PoolStage::Qualification => "qualification",
                        }
                    )),
                    status,
                }
            }

            fn evidence(
                stage: PoolStage,
                model: ModelIdentity,
                is_passing: bool,
                failed_trials: u32,
            ) -> PoolEntrantEvidence {
                let candidate_usage = usage(100, 50, 400_000, 1_200);
                let judge_usage = usage(30, 20, 100_000, 300);
                PoolEntrantEvidence {
                    stage,
                    requested_model: model.clone(),
                    effective_model: model,
                    judge_model: candidate(Tier::T5, "openai-codex", "judge-exact"),
                    harnesses: Vec::new(),
                    is_passing,
                    completed_trials: 4,
                    expected_trials: 4,
                    failed_trials,
                    catastrophic_trials: 0,
                    score: ConfidenceInterval {
                        lower: 7.5,
                        estimate: 8.5,
                        upper: 9.0,
                    },
                    total_usage: usage(130, 70, 500_000, 1_500),
                    candidate_usage,
                    judge_usage,
                }
            }

            fn usage(input: u64, output: u64, cost: u64, elapsed: u64) -> TrialUsage {
                TrialUsage {
                    input_tokens: input,
                    output_tokens: output,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    turns: 1,
                    tool_calls: 0,
                    elapsed_milliseconds: elapsed,
                    cost_millionths_of_dollar: cost,
                }
            }

            fn candidate(tier: Tier, provider: &str, model: &str) -> ModelIdentity {
                ModelIdentity {
                    tier,
                    provider: provider.to_owned(),
                    model: model.to_owned(),
                    thinking: "off".to_owned(),
                }
            }

            fn os_arguments(values: &[&str]) -> Vec<OsString> {
                values.iter().map(OsString::from).collect()
            }
        }
    };
}
