#[macro_export]
macro_rules! frontier_render_tests {
    () => {
        mod frontier_render_tests {
            use std::collections::BTreeMap;
            use std::path::PathBuf;

            use super::{
                render_frontier_apply, render_frontier_inspection, render_frontier_preview,
                render_frontier_report,
            };
            use $crate::model::{
                ArtifactName, CaseId, CheckResult, CheckStatus, FrontierApplyReport,
                FrontierBaselineChange, FrontierCellEvidence, FrontierCellStatus,
                FrontierInspection, FrontierModelReport, FrontierPoolMembership,
                FrontierPreviewReport, FrontierReport, FrontierRunId, FrontierRunStatus,
                FrontierScore, HarnessIdentity, ModelIdentity, OutputFormat, PoolPauseReason,
                T1ScreenCallRange, Tier, TrialKey, TrialRecord, TrialUsage, TrialVerdict,
            };

            #[test]
            fn frontier_preview_text_is_capacity_first_and_golden() {
                let report = preview();
                let output = render_text(|output| {
                    render_frontier_preview(&report, OutputFormat::Text, output)
                });

                assert_eq!(
                    output,
                    concat!(
                        "suite capacity:\n",
                        "  T1: 30 cases\n",
                        "  T2: 31 cases\n",
                        "  T3: 32 cases\n",
                        "  T4: 33 cases\n",
                        "  T5: 34 cases\n",
                        "guards: capacity=passed; owner_approval_required=true\n",
                        "plan sha256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
                        "routes: 12\n",
                        "candidate calls: minimum 150, maximum 750\n",
                        "judge calls: minimum 150, maximum 750\n",
                        "maximum spending: 9000000 millionths of a dollar\n",
                    )
                );

                let mut authorized = report;
                authorized.is_owner_approval_required = false;
                let output = render_text(|output| {
                    render_frontier_preview(&authorized, OutputFormat::Text, output)
                });
                assert!(output.contains("owner_approval_required=false"));
            }

            #[test]
            fn frontier_report_text_has_one_exact_matrix_and_full_detail() {
                let report = report();
                let output = render_text(|output| {
                    render_frontier_report(&report, OutputFormat::Text, output)
                });

                assert!(output.starts_with(concat!(
                    "| Model | off | minimal | low | medium | high | xhigh | max |\n",
                    "| --- | --- | --- | --- | --- | --- | --- | --- |\n",
                    "| anthropic/alpha | F2 | F3 | P5 | N/A | N/A | N/A | N/A |\n",
                )));
                assert_eq!(output.matches("| Model |").count(), 1);
                assert!(output.contains("infrastructure/pause: Infrastructure { message: \"provider unavailable\" }"));
                assert!(output.contains("weighted 9100; lower 8800; critical 2/2; coverage=true"));
                assert!(output.contains("pool memberships: T1 rank 1 active=true"));
                let matrix = output.lines().take(3).collect::<Vec<_>>().join("\n");
                assert!(!matrix.contains("provider unavailable"));
            }

            #[test]
            fn quota_skipped_model_is_marked_in_the_matrix_and_detail() {
                let mut report = report();
                let skipped = FrontierCellEvidence {
                    model: route(Tier::T3, "off"),
                    status: FrontierCellStatus::Skipped,
                    completed_trials: 0,
                    expected_trials: 0,
                    failed_trials: 0,
                    score: None,
                    total_usage: zero_usage(),
                };
                let model = &mut report.models[0];
                model.cells = vec![skipped];
                model.highest_passing_tier = None;
                model.selected_routes.clear();
                model.pool_memberships.clear();
                model.total_usage = zero_usage();
                report.status = FrontierRunStatus::AwaitingDecision;
                report.pause = None;

                let output = render_text(|output| {
                    render_frontier_report(&report, OutputFormat::Text, output)
                });

                assert!(output.contains("| anthropic/alpha | Q |  |  | N/A | N/A | N/A | N/A |"));
                assert!(output.contains("set aside after quota: T3 (off)"));
            }

            #[test]
            fn frontier_inspection_text_preserves_the_full_typed_evidence() {
                let inspection = FrontierInspection::Trial { trial: trial() };
                let output = render_text(|output| {
                    render_frontier_inspection(&inspection, OutputFormat::Text, output)
                });
                let encoded = output.strip_prefix("frontier inspection:\n").unwrap();
                let decoded: FrontierInspection = serde_json::from_str(encoded).unwrap();

                assert_eq!(decoded, inspection);
                assert!(output.contains("\"tool_policy_digest\": \"policy-digest\""));
                assert!(output.contains("\"cache_write_tokens\": 4"));
                assert!(output.contains("\"failure_mode\": \"missed edge\""));
            }

            #[test]
            fn frontier_apply_text_lists_active_routes_and_change_status() {
                let mut changed = apply();
                let output = render_text(|output| {
                    render_frontier_apply(&changed, OutputFormat::Text, output)
                });
                assert_eq!(
                    output,
                    concat!(
                        "frontier apply frontier-1:\n",
                        "  T1: anthropic/alpha (T1; off)\n",
                        "  T2: anthropic/alpha (T2; minimal)\n",
                        "  T3: anthropic/alpha (T3; low)\n",
                        "  T4: anthropic/alpha (T4; medium)\n",
                        "  T5: anthropic/alpha (T5; high)\n",
                        "status: changed\n",
                    )
                );

                changed.is_changed = false;
                let output = render_text(|output| {
                    render_frontier_apply(&changed, OutputFormat::Text, output)
                });
                assert!(output.ends_with("status: no-op\n"));
            }

            #[test]
            fn every_frontier_jsonl_renderer_writes_one_full_value_and_newline() {
                assert_json_line(&preview(), |value, output| {
                    render_frontier_preview(value, OutputFormat::JsonLines, output)
                });
                assert_json_line(&report(), |value, output| {
                    render_frontier_report(value, OutputFormat::JsonLines, output)
                });
                let inspection = FrontierInspection::Trial { trial: trial() };
                assert_json_line(&inspection, |value, output| {
                    render_frontier_inspection(value, OutputFormat::JsonLines, output)
                });
                assert_json_line(&apply(), |value, output| {
                    render_frontier_apply(value, OutputFormat::JsonLines, output)
                });
            }

            #[test]
            fn malformed_or_forged_frontier_values_fail_before_output() {
                let mut malformed_preview = preview();
                malformed_preview.tier_case_counts.remove(&Tier::T5);
                assert_rejected(|output| {
                    render_frontier_preview(&malformed_preview, OutputFormat::Text, output)
                });

                let mut malformed_report = report();
                malformed_report.models[0].selected_routes.clear();
                assert_rejected(|output| {
                    render_frontier_report(&malformed_report, OutputFormat::JsonLines, output)
                });

                let mut malformed_inspection = FrontierInspection::Trial { trial: trial() };
                let FrontierInspection::Trial { trial } = &mut malformed_inspection else {
                    unreachable!()
                };
                trial.key.tier = Tier::T2;
                assert_rejected(|output| {
                    render_frontier_inspection(
                        &malformed_inspection,
                        OutputFormat::Text,
                        output,
                    )
                });

                let mut forged_apply = apply();
                forged_apply.active_routes.get_mut(&Tier::T3).unwrap()[0].tier = Tier::T2;
                assert_rejected(|output| {
                    render_frontier_apply(&forged_apply, OutputFormat::JsonLines, output)
                });
            }

            fn preview() -> FrontierPreviewReport {
                FrontierPreviewReport {
                    plan_sha256: "a".repeat(64),
                    tier_case_counts: [
                        (Tier::T1, 30),
                        (Tier::T2, 31),
                        (Tier::T3, 32),
                        (Tier::T4, 33),
                        (Tier::T5, 34),
                    ]
                    .into_iter()
                    .collect(),
                    route_count: 12,
                    candidate_calls: T1ScreenCallRange {
                        minimum: 150,
                        maximum: 750,
                    },
                    judge_calls: T1ScreenCallRange {
                        minimum: 150,
                        maximum: 750,
                    },
                    maximum_spending_millionths_of_dollar: 9_000_000,
                    is_owner_approval_required: true,
                }
            }

            fn report() -> FrontierReport {
                let t1_off = cell(Tier::T1, "off", FrontierCellStatus::Passed, 9_100);
                let t2_off = cell(Tier::T2, "off", FrontierCellStatus::Failed, 7_900);
                let t2_minimal = cell(Tier::T2, "minimal", FrontierCellStatus::Passed, 9_100);
                let t3_minimal = cell(Tier::T3, "minimal", FrontierCellStatus::Failed, 7_900);
                let t3_low = cell(Tier::T3, "low", FrontierCellStatus::Passed, 9_100);
                let t4_low = cell(Tier::T4, "low", FrontierCellStatus::Passed, 9_100);
                let t5_low = cell(Tier::T5, "low", FrontierCellStatus::Passed, 9_100);
                let mut total_usage = usage();
                total_usage.input_tokens *= 7;
                total_usage.output_tokens *= 7;
                total_usage.cache_read_tokens *= 7;
                total_usage.cache_write_tokens *= 7;
                total_usage.turns *= 7;
                total_usage.tool_calls *= 7;
                total_usage.elapsed_milliseconds *= 7;
                total_usage.cost_millionths_of_dollar *= 7;
                FrontierReport {
                    run_id: FrontierRunId("frontier-1".to_owned()),
                    status: FrontierRunStatus::Paused,
                    models: vec![FrontierModelReport {
                        provider: "anthropic".to_owned(),
                        model: "alpha".to_owned(),
                        supported_thinking_levels: vec![
                            "off".to_owned(),
                            "minimal".to_owned(),
                            "low".to_owned(),
                        ],
                        cells: vec![
                            t1_off.clone(),
                            t2_off,
                            t2_minimal.clone(),
                            t3_minimal,
                            t3_low.clone(),
                            t4_low.clone(),
                            t5_low.clone(),
                        ],
                        highest_passing_tier: Some(Tier::T5),
                        selected_routes: vec![
                            t1_off.model.clone(),
                            t2_minimal.model,
                            t3_low.model,
                            t4_low.model,
                            t5_low.model,
                        ],
                        pool_memberships: BTreeMap::from([(
                            Tier::T1,
                            FrontierPoolMembership {
                                model: t1_off.model,
                                rank: 1,
                                is_active: true,
                            },
                        )]),
                        baseline_change: FrontierBaselineChange::Better,
                        total_usage,
                    }],
                    pause: Some(PoolPauseReason::Infrastructure {
                        message: "provider unavailable".to_owned(),
                    }),
                    decision: None,
                    spent_millionths_of_dollar: 57,
                }
            }

            fn cell(
                tier: Tier,
                thinking: &str,
                status: FrontierCellStatus,
                weighted_pass_basis_points: u16,
            ) -> FrontierCellEvidence {
                FrontierCellEvidence {
                    model: route(tier, thinking),
                    status,
                    completed_trials: 2,
                    expected_trials: 2,
                    failed_trials: u32::from(status != FrontierCellStatus::Passed),
                    score: Some(FrontierScore {
                        weighted_pass_basis_points,
                        lower_bound_basis_points: 8_800,
                        critical_passed_trials: 2,
                        critical_expected_trials: 2,
                        is_group_coverage_complete: true,
                    }),
                    total_usage: usage(),
                }
            }

            fn apply() -> FrontierApplyReport {
                FrontierApplyReport {
                    run_id: FrontierRunId("frontier-1".to_owned()),
                    active_routes: [
                        (Tier::T1, vec![route(Tier::T1, "off")]),
                        (Tier::T2, vec![route(Tier::T2, "minimal")]),
                        (Tier::T3, vec![route(Tier::T3, "low")]),
                        (Tier::T4, vec![route(Tier::T4, "medium")]),
                        (Tier::T5, vec![route(Tier::T5, "high")]),
                    ]
                    .into_iter()
                    .collect(),
                    is_changed: true,
                }
            }

            fn trial() -> TrialRecord {
                TrialRecord {
                    key: TrialKey {
                        artifact: ArtifactName("agent-author".to_owned()),
                        tier: Tier::T1,
                        route_index: 2,
                        case: CaseId("case-7".to_owned()),
                        attempt: 2,
                    },
                    model: route(Tier::T1, "off"),
                    harness: HarnessIdentity {
                        runner_version: "1.0".to_owned(),
                        pi_version: "0.84".to_owned(),
                        artifact_revision: "artifact-revision".to_owned(),
                        tool_policy_digest: "policy-digest".to_owned(),
                    },
                    artifact_path: PathBuf::from("agents/agent-author"),
                    transcript_path: PathBuf::from("runs/transcript.jsonl"),
                    candidate_usage: usage(),
                    judge_model: ModelIdentity {
                        provider: "openai-codex".to_owned(),
                        model: "judge".to_owned(),
                        tier: Tier::T5,
                        thinking: "high".to_owned(),
                    },
                    judge_usage: usage(),
                    verdict: TrialVerdict {
                        score: 7,
                        is_catastrophic: false,
                        failure_mode: Some("missed edge".to_owned()),
                        checks: vec![CheckResult {
                            name: "fixture".to_owned(),
                            status: CheckStatus::Passed,
                            detail: Some("exact".to_owned()),
                        }],
                    },
                }
            }

            fn route(tier: Tier, thinking: &str) -> ModelIdentity {
                ModelIdentity {
                    provider: "anthropic".to_owned(),
                    model: "alpha".to_owned(),
                    tier,
                    thinking: thinking.to_owned(),
                }
            }

            fn zero_usage() -> TrialUsage {
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

            fn usage() -> TrialUsage {
                TrialUsage {
                    input_tokens: 11,
                    output_tokens: 13,
                    cache_read_tokens: 3,
                    cache_write_tokens: 4,
                    turns: 2,
                    tool_calls: 5,
                    elapsed_milliseconds: 17,
                    cost_millionths_of_dollar: 19,
                }
            }

            fn render_text(
                render: impl FnOnce(&mut Vec<u8>) -> Result<(), $crate::model::SkillEvalError>,
            ) -> String {
                let mut output = Vec::new();
                render(&mut output).unwrap();
                String::from_utf8(output).unwrap()
            }

            fn assert_json_line<T>(
                expected: &T,
                render: impl FnOnce(
                    &T,
                    &mut Vec<u8>,
                ) -> Result<(), $crate::model::SkillEvalError>,
            ) where
                T: serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
            {
                let mut output = Vec::new();
                render(expected, &mut output).unwrap();
                assert_eq!(output.last(), Some(&b'\n'));
                assert_eq!(output.iter().filter(|byte| **byte == b'\n').count(), 1);
                let actual: T = serde_json::from_slice(&output).unwrap();
                assert_eq!(&actual, expected);
            }

            fn assert_rejected(
                render: impl FnOnce(&mut Vec<u8>) -> Result<(), $crate::model::SkillEvalError>,
            ) {
                let mut output = Vec::new();
                let error = render(&mut output).unwrap_err();
                assert!(matches!(
                    error,
                    $crate::model::SkillEvalError::InvalidConfiguration(message)
                        if message.starts_with("malformed frontier render state:")
                ));
                assert!(output.is_empty());
            }
        }
    };
}
