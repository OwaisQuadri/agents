macro_rules! frontier_apply_tests {
    () => {
        mod frontier_apply_tests {
            use std::cell::{Cell, RefCell};
            use std::collections::BTreeMap;
            use std::fs;
            use std::path::{Path, PathBuf};

            use sha2::{Digest, Sha256};

            use super::{
                apply_frontier_routes_at, execute_frontier_command, qualification_routes_span,
            };
            use $crate::model::{
                Decision, FrontierApplyReport, FrontierBaseline, FrontierBaselineLedger,
                FrontierConfidenceMethod, FrontierDecisionRecord, FrontierEntrant,
                FrontierEvidenceIdentity, FrontierInspection, FrontierModelProgress, FrontierPlan,
                FrontierPolicy, FrontierPoolMembership, FrontierRunConfiguration, FrontierRunId,
                FrontierRunState, FrontierRunStatus, FrontierSuite, FrontierSuiteIdentity,
                FrontierTrialSelector, ModelIdentity, OutputFormat, SkillEvalError,
                T1ScreenSnapshotIdentity, Tier, Timestamp, TrialRecord,
            };
            use $crate::ports::FrontierRuntime;
            use $crate::testing::{FakeQualificationRuntime, TemporaryRoot};

            thread_local! {
                static DISPATCH_STATE: RefCell<Option<FrontierRunState>> = const { RefCell::new(None) };
                static DISPATCH_LOADS: Cell<u32> = const { Cell::new(0) };
            }

            #[test]
            fn frontier_apply_is_idempotent_and_preserves_unrelated_bytes() {
                let fixture = Fixture::new();
                let before = fs::read(&fixture.path).unwrap();
                let before_span = qualification_routes_span(&before).unwrap();
                let first = apply_frontier_routes_at(
                    &fixture.canonical_root,
                    &digest(&before),
                    &fixture.state,
                    &fixture.baseline,
                )
                .unwrap();
                let after = fs::read(&fixture.path).unwrap();
                let after_span = qualification_routes_span(&after).unwrap();

                assert!(first.is_changed);
                assert_eq!(first.run_id, fixture.state.configuration.run_id);
                assert_eq!(&after[..after_span.start], &before[..before_span.start]);
                assert_eq!(&after[after_span.end..], &before[before_span.end..]);
                let value: serde_json::Value = serde_json::from_slice(&after).unwrap();
                assert_eq!(value["agents"]["worker"], "T3");
                assert_eq!(value["capabilities"]["special"]["pi"], "keep/model");
                assert_eq!(value["qualification_routes"]["T1"][0]["provider"], "anthropic");
                assert!(value["qualification_routes"]["T1"][0].get("tier").is_none());

                let second = apply_frontier_routes_at(
                    &fixture.canonical_root,
                    &digest(&after),
                    &fixture.state,
                    &fixture.baseline,
                )
                .unwrap();
                assert!(!second.is_changed);
                assert_eq!(second.active_routes, first.active_routes);
                assert_eq!(fs::read(&fixture.path).unwrap(), after);
            }

            #[test]
            fn frontier_apply_rejects_invalid_authority_before_write() {
                let fixture = Fixture::new();
                let before = fs::read(&fixture.path).unwrap();
                let authority = digest(&before);
                let assert_rejected = |state: &FrontierRunState, baseline: &FrontierBaseline| {
                    assert!(
                        apply_frontier_routes_at(
                            &fixture.canonical_root,
                            &authority,
                            state,
                            baseline,
                        )
                        .is_err()
                    );
                    assert_eq!(fs::read(&fixture.path).unwrap(), before);
                };

                let mut unresolved = fixture.state.clone();
                unresolved.status = FrontierRunStatus::AwaitingDecision;
                unresolved.decision = None;
                assert_rejected(&unresolved, &fixture.baseline);

                let mut rejected = fixture.state.clone();
                rejected.status = FrontierRunStatus::Rejected;
                rejected.decision.as_mut().unwrap().decision = Decision::Rejected;
                assert_rejected(&rejected, &fixture.baseline);

                let mut stale = fixture.baseline.clone();
                stale.run_id = FrontierRunId("stale-run".to_owned());
                assert_rejected(&fixture.state, &stale);

                let mut changed = fixture.baseline.clone();
                changed.run_evidence.sha256 = "0".repeat(64);
                assert_rejected(&fixture.state, &changed);

                let mut duplicate = fixture.baseline.clone();
                let mut repeated = duplicate.pools[&Tier::T1][0].clone();
                repeated.rank = 2;
                repeated.is_active = false;
                duplicate.pools.get_mut(&Tier::T1).unwrap().push(repeated);
                assert_rejected(&fixture.state, &duplicate);

                let mut foreign = fixture.baseline.clone();
                foreign.pools.get_mut(&Tier::T1).unwrap()[0].model.model = "foreign".to_owned();
                assert_rejected(&fixture.state, &foreign);

                let mut non_first_party = fixture.baseline.clone();
                non_first_party.pools.get_mut(&Tier::T1).unwrap()[0]
                    .model
                    .provider = "openrouter".to_owned();
                assert_rejected(&fixture.state, &non_first_party);

                let mut inconsistent = fixture.baseline.clone();
                inconsistent.pools.get_mut(&Tier::T1).unwrap()[0].is_active = false;
                assert_rejected(&fixture.state, &inconsistent);

                assert!(
                    apply_frontier_routes_at(
                        &fixture.canonical_root,
                        &"f".repeat(64),
                        &fixture.state,
                        &fixture.baseline,
                    )
                    .is_err()
                );
                assert_eq!(fs::read(&fixture.path).unwrap(), before);

                let mut duplicate_top_level = b"{\n  \"orchestrator\": \"duplicate\",\n".to_vec();
                duplicate_top_level.extend_from_slice(&before[2..]);
                fs::write(&fixture.path, &duplicate_top_level).unwrap();
                assert!(
                    apply_frontier_routes_at(
                        &fixture.canonical_root,
                        &digest(&duplicate_top_level),
                        &fixture.state,
                        &fixture.baseline,
                    )
                    .is_err()
                );
                assert_eq!(fs::read(&fixture.path).unwrap(), duplicate_top_level);
            }

            #[test]
            fn frontier_apply_dispatches_rejected_state_without_render_or_write() {
                let mut state = Fixture::state();
                state.status = FrontierRunStatus::Rejected;
                state.decision = Some(FrontierDecisionRecord {
                    decision: Decision::Rejected,
                    reason: "owner rejected".to_owned(),
                    decided_at: timestamp(),
                });
                DISPATCH_STATE.with(|slot| *slot.borrow_mut() = Some(state));
                DISPATCH_LOADS.with(|loads| loads.set(0));
                let root = TemporaryRoot::new("frontier-apply-dispatch");
                let mut runtime = FakeQualificationRuntime::new(&root);
                let mut output = Vec::new();

                let error = execute_frontier_command(
                    &$crate::model::CliCommand::FrontierApply {
                        run_id: FrontierRunId("run-accepted".to_owned()),
                    },
                    OutputFormat::Text,
                    &mut runtime,
                    &mut output,
                )
                .unwrap_err();

                assert!(matches!(error, SkillEvalError::InvalidConfiguration(message) if message.contains("accepted owner decision")));
                assert_eq!(DISPATCH_LOADS.with(Cell::get), 1);
                assert!(output.is_empty());
            }

            impl FrontierRuntime for FakeQualificationRuntime {
                fn load_frontier_plan(
                    &self,
                    _path: &Path,
                ) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError> {
                    panic!("rejected dispatch loaded a plan")
                }

                fn next_frontier_run_id(&mut self) -> Result<FrontierRunId, SkillEvalError> {
                    panic!("rejected dispatch allocated a run")
                }

                fn create_frontier(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    panic!("rejected dispatch created a run")
                }

                fn load_frontier(
                    &self,
                    run_id: &FrontierRunId,
                ) -> Result<FrontierRunState, SkillEvalError> {
                    DISPATCH_LOADS.with(|loads| loads.set(loads.get() + 1));
                    DISPATCH_STATE.with(|slot| {
                        slot.borrow()
                            .clone()
                            .filter(|state| state.configuration.run_id == *run_id)
                            .ok_or_else(|| SkillEvalError::NotFound("frontier".to_owned()))
                    })
                }

                fn save_frontier(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    panic!("rejected dispatch saved a run")
                }

                fn save_frontier_trial(
                    &mut self,
                    _run_id: &FrontierRunId,
                    _trial: &TrialRecord,
                ) -> Result<(), SkillEvalError> {
                    panic!("rejected dispatch saved a trial")
                }

                fn inspect_frontier(
                    &self,
                    _selector: &FrontierTrialSelector,
                ) -> Result<FrontierInspection, SkillEvalError> {
                    panic!("rejected dispatch inspected evidence")
                }

                fn load_frontier_baselines(
                    &self,
                    _path: &Path,
                ) -> Result<FrontierBaselineLedger, SkillEvalError> {
                    panic!("rejected dispatch loaded a baseline")
                }

                fn accept_frontier_baseline(
                    &mut self,
                    _state: &FrontierRunState,
                    _path: &Path,
                    _ledger: &FrontierBaselineLedger,
                ) -> Result<(), SkillEvalError> {
                    panic!("rejected dispatch accepted a baseline")
                }

                fn apply_frontier_routes(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<FrontierApplyReport, SkillEvalError> {
                    panic!("rejected dispatch wrote routes")
                }
            }

            struct Fixture {
                _root: TemporaryRoot,
                canonical_root: PathBuf,
                path: PathBuf,
                state: FrontierRunState,
                baseline: FrontierBaseline,
            }

            impl Fixture {
                fn new() -> Self {
                    let root = TemporaryRoot::new("frontier-apply-routes");
                    fs::create_dir(root.path().join("config")).unwrap();
                    let path = root.path().join("config/model-tiers.json");
                    fs::write(&path, routing_fixture()).unwrap();
                    let canonical_root = fs::canonicalize(root.path()).unwrap();
                    let state = Self::state();
                    let baseline = baseline(&state);
                    Self {
                        _root: root,
                        canonical_root,
                        path,
                        state,
                        baseline,
                    }
                }

                fn state() -> FrontierRunState {
                    let routes = tier_routes();
                    let entrants = routes
                        .values()
                        .map(|route| FrontierEntrant {
                            provider: route.provider.clone(),
                            model: route.model.clone(),
                            entry_tier: route.tier,
                            thinking_levels: vec![route.thinking.clone()],
                            catalog_observed_at: timestamp(),
                        })
                        .collect::<Vec<_>>();
                    let models = routes
                        .values()
                        .map(|route| FrontierModelProgress {
                            provider: route.provider.clone(),
                            model: route.model.clone(),
                            entry_tier: route.tier,
                            selected_routes: vec![route.clone()],
                            next_tier: None,
                            next_thinking_index: None,
                            is_exhausted: true,
                        })
                        .collect();
                    FrontierRunState {
                        configuration: FrontierRunConfiguration {
                            run_id: FrontierRunId("run-accepted".to_owned()),
                            created_at: timestamp(),
                            plan_path: PathBuf::from("plan.json"),
                            plan_sha256: "1".repeat(64),
                            plan: FrontierPlan {
                                version: 1,
                                suite: FrontierSuiteIdentity {
                                    path: PathBuf::from("suite.json"),
                                    sha256: "2".repeat(64),
                                    version: 1,
                                },
                                capabilities: T1ScreenSnapshotIdentity {
                                    path: PathBuf::from("capabilities.json"),
                                    sha256: "3".repeat(64),
                                    version: 1,
                                    observed_at_unix_seconds: 1,
                                    pi_version: "pi-test".to_owned(),
                                },
                                entrants,
                                judge: ModelIdentity {
                                    tier: Tier::T5,
                                    provider: "anthropic".to_owned(),
                                    model: "judge".to_owned(),
                                    thinking: "high".to_owned(),
                                },
                                policy: policy(),
                            },
                        },
                        status: FrontierRunStatus::Accepted,
                        models,
                        cells: Vec::new(),
                        infrastructure_events: Vec::new(),
                        pause: None,
                        decision: Some(FrontierDecisionRecord {
                            decision: Decision::Accepted,
                            reason: "owner approved".to_owned(),
                            decided_at: timestamp(),
                        }),
                        spent_millionths_of_dollar: 0,
                    }
                }
            }

            fn baseline(state: &FrontierRunState) -> FrontierBaseline {
                let pools = tier_routes()
                    .into_iter()
                    .map(|(tier, model)| {
                        (
                            tier,
                            vec![FrontierPoolMembership {
                                model,
                                rank: 1,
                                is_active: true,
                            }],
                        )
                    })
                    .collect();
                let mut bytes = serde_json::to_vec_pretty(state).unwrap();
                bytes.push(b'\n');
                FrontierBaseline {
                    accepted_at: timestamp(),
                    run_id: state.configuration.run_id.clone(),
                    run_evidence: FrontierEvidenceIdentity {
                        path: PathBuf::from(
                            ".map/skill-eval/frontier/run-accepted/state.json",
                        ),
                        sha256: digest(&bytes),
                    },
                    previous_entry_sha256: None,
                    pools,
                    capabilities: Vec::new(),
                }
            }

            fn tier_routes() -> BTreeMap<Tier, ModelIdentity> {
                [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5]
                    .into_iter()
                    .enumerate()
                    .map(|(index, tier)| {
                        (
                            tier,
                            ModelIdentity {
                                tier,
                                provider: if index.is_multiple_of(2) {
                                    "anthropic".to_owned()
                                } else {
                                    "openai-codex".to_owned()
                                },
                                model: format!("model-{index}"),
                                thinking: "low".to_owned(),
                            },
                        )
                    })
                    .collect()
            }

            fn policy() -> FrontierPolicy {
                FrontierPolicy {
                    screening_trials_per_case: 1,
                    confirmation_trials_per_case: 3,
                    maximum_trials_per_case: 5,
                    minimum_trial_score: 8,
                    minimum_weighted_pass_basis_points: 8_500,
                    minimum_lower_bound_basis_points: 8_000,
                    confidence_level_basis_points: 9_500,
                    confidence_method: FrontierConfidenceMethod::StratifiedBootstrap,
                    confidence_resamples: 100,
                    maximum_infrastructure_attempts: 2,
                    maximum_catalog_age_seconds: 3_600,
                    active_pool_size: 5,
                    maximum_trial_cost_millionths_of_dollar: 1,
                    spending_limit_millionths_of_dollar: 1,
                    is_provider_limit_enforced: true,
                    is_first_party_only: true,
                }
            }

            fn timestamp() -> Timestamp {
                Timestamp("2026-08-28T12:00:00-0400".to_owned())
            }

            fn digest(bytes: &[u8]) -> String {
                Sha256::digest(bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect()
            }

            fn routing_fixture() -> &'static [u8] {
                br#"{
  "tiers": {"T1": {"pi": "keep/model", "fallbacks": [], "thinking": "low"}},
  "qualification_routes": {},
  "orchestrator": "T3",
  "judge": "T5",
  "capabilities": {"special": {"pi": "keep/model", "fallbacks": []}},
  "unranked_controls": {},

  "agents": {"worker": "T3"},
  "untiered": {"delegate": "keep"}
}
"#
            }
        }
    };
}
