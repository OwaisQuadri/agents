#[macro_export]
macro_rules! frontier_runtime_tests {
    () => {
        mod frontier_runtime_adapter_tests {
            use std::cell::{Cell, RefCell};
            use std::collections::BTreeMap;
            use std::fs;
            use std::io::{self, Write};
            use std::path::{Path, PathBuf};
            use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
            use std::sync::{Arc, Barrier};

            use super::{
                FileSuiteRuntime, RenderFrontierProgress, apply_current_frontier_suite,
                load_frontier_plan_files, next_frontier_run_id, proposal_artifact_revisions,
                run_bounded_frontier_jobs, sha256_digest, validate_proposal_sources,
            };
            use crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, CaseId, FrontierCaseGroup,
                FrontierCaseKey, FrontierCaseReference, FrontierConfidenceMethod, FrontierPlan,
                FrontierPolicy, FrontierRunConfiguration, FrontierRunId, FrontierRunState,
                FrontierRunStatus, FrontierSuite, FrontierSuiteConstructionPlan,
                FrontierSuiteConstructionPolicy, FrontierSuiteProposal,
                FrontierSuiteProposalStatus, FrontierTierSuite, ModelIdentity, OutputFormat, RunId,
                SkillEvalError, T1ScreenSnapshotIdentity, Tier, Timestamp,
            };
            use crate::ports::{
                ArtifactSource, FrontierProgressSink, FrontierSuiteRuntime, RunIdSource,
            };

            static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

            #[test]
            fn frontier_workers_run_exactly_four_jobs_at_a_time() {
                let active = Arc::new(AtomicUsize::new(0));
                let maximum = Arc::new(AtomicUsize::new(0));
                let barrier = Arc::new(Barrier::new(4));
                let outcomes = run_bounded_frontier_jobs((0..8).collect(), {
                    let active = active.clone();
                    let maximum = maximum.clone();
                    let barrier = barrier.clone();
                    move |job| {
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        maximum.fetch_max(current, Ordering::SeqCst);
                        barrier.wait();
                        active.fetch_sub(1, Ordering::SeqCst);
                        job
                    }
                });

                assert_eq!(
                    outcomes.into_iter().collect::<Result<Vec<_>, _>>(),
                    Ok((0..8).collect())
                );
                assert_eq!(maximum.load(Ordering::SeqCst), 4);
            }

            struct MockSource {
                revisions: BTreeMap<PathBuf, String>,
                calls: RefCell<Vec<PathBuf>>,
                is_failing: bool,
            }

            impl MockSource {
                fn current(path: &str, revision: &str) -> Self {
                    Self {
                        revisions: BTreeMap::from([(PathBuf::from(path), revision.to_owned())]),
                        calls: RefCell::new(Vec::new()),
                        is_failing: false,
                    }
                }
            }

            impl ArtifactSource for MockSource {
                fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
                    self.calls.borrow_mut().push(root.to_path_buf());
                    if self.is_failing {
                        return Err(SkillEvalError::NotFound("source failed".to_owned()));
                    }
                    let revision = self.revisions.get(root).ok_or_else(|| {
                        SkillEvalError::NotFound(format!("missing {}", root.display()))
                    })?;
                    Ok(ArtifactDefinition {
                        name: ArtifactName("fixture".to_owned()),
                        kind: ArtifactKind::Skill,
                        root: root.to_path_buf(),
                        revision: revision.clone(),
                        required_destinations: Vec::new(),
                        current_tiers: Vec::new(),
                        cases: Vec::new(),
                    })
                }
            }

            struct FixtureRoot(PathBuf);

            impl FixtureRoot {
                fn new(label: &str) -> Self {
                    let sequence = ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                    let root = std::env::temp_dir().join(format!(
                        "skill-eval-frontier-runtime-{label}-{}-{sequence}",
                        std::process::id()
                    ));
                    fs::create_dir_all(&root).unwrap();
                    Self(fs::canonicalize(root).unwrap())
                }
            }

            impl Drop for FixtureRoot {
                fn drop(&mut self) {
                    fs::remove_dir_all(&self.0).unwrap();
                }
            }

            fn case(path: &str, revision: &str, id: &str) -> FrontierCaseReference {
                FrontierCaseReference {
                    artifact_path: PathBuf::from(path),
                    artifact_revision: revision.to_owned(),
                    case: CaseId(id.to_owned()),
                    group: FrontierCaseGroup::Normal,
                    is_confirmation: false,
                }
            }

            fn key(path: &str, revision: &str, id: &str) -> FrontierCaseKey {
                FrontierCaseKey {
                    artifact_path: PathBuf::from(path),
                    artifact_revision: revision.to_owned(),
                    case: CaseId(id.to_owned()),
                }
            }

            fn proposal(revision: &str) -> FrontierSuiteProposal {
                FrontierSuiteProposal {
                    version: 1,
                    inventory_sha256: "a".repeat(64),
                    review_set_sha256: "b".repeat(64),
                    policy: FrontierSuiteConstructionPolicy {
                        required_tiers: vec![Tier::T1],
                        minimum_unique_cases_per_tier: 1,
                        minimum_reviewers_per_case: 2,
                        group_weights_basis_points: BTreeMap::from([
                            (FrontierCaseGroup::Normal, 10_000),
                            (FrontierCaseGroup::Edge, 0),
                            (FrontierCaseGroup::Adversarial, 0),
                            (FrontierCaseGroup::Critical, 0),
                        ]),
                        is_unanimous_eligibility_required: true,
                        is_cross_tier_reuse_allowed: false,
                        is_calibration_anchor_counted_toward_minimum: false,
                    },
                    proposed_tiers: BTreeMap::from([(
                        Tier::T1,
                        FrontierTierSuite {
                            group_weights_basis_points: BTreeMap::from([
                                (FrontierCaseGroup::Normal, 10_000),
                                (FrontierCaseGroup::Edge, 0),
                                (FrontierCaseGroup::Adversarial, 0),
                                (FrontierCaseGroup::Critical, 0),
                            ]),
                            cases: vec![case("skills/fixture", revision, "case-a")],
                        },
                    )]),
                    calibration_anchors: vec![key("skills/fixture", revision, "anchor")],
                    holdout_cases: vec![key("skills/fixture", revision, "holdout")],
                    tier_capacity: BTreeMap::new(),
                    status: FrontierSuiteProposalStatus::Blocked,
                }
            }

            fn construction_plan() -> FrontierSuiteConstructionPlan {
                FrontierSuiteConstructionPlan {
                    version: 1,
                    artifact_roots: vec![PathBuf::from("skills/fixture")],
                    policy: FrontierSuiteConstructionPolicy {
                        required_tiers: vec![Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5],
                        minimum_unique_cases_per_tier: 30,
                        minimum_reviewers_per_case: 2,
                        group_weights_basis_points: BTreeMap::from([
                            (FrontierCaseGroup::Normal, 4_000),
                            (FrontierCaseGroup::Edge, 2_000),
                            (FrontierCaseGroup::Adversarial, 2_000),
                            (FrontierCaseGroup::Critical, 2_000),
                        ]),
                        is_unanimous_eligibility_required: true,
                        is_cross_tier_reuse_allowed: false,
                        is_calibration_anchor_counted_toward_minimum: false,
                    },
                }
            }

            #[test]
            fn file_only_suite_runtime_delegates_store_reads_without_source_calls() {
                let root = FixtureRoot::new("suite-delegation");
                fs::create_dir_all(root.0.join("plans")).unwrap();
                fs::write(
                    root.0.join("plans/construction.json"),
                    serde_json::to_vec_pretty(&construction_plan()).unwrap(),
                )
                .unwrap();
                let source = MockSource::current("skills/fixture", "current");
                let runtime = FileSuiteRuntime {
                    source,
                    store: crate::frontier_store::FileFrontierStore::new(&root.0).unwrap(),
                };
                assert_eq!(
                    runtime
                        .load_frontier_suite_construction_plan(Path::new("plans/construction.json"))
                        .unwrap(),
                    construction_plan()
                );
                assert!(runtime.source.calls.borrow().is_empty());
            }

            #[test]
            fn proposal_sources_load_each_unique_artifact_once() {
                let source = MockSource::current("skills/fixture", "current");
                validate_proposal_sources(&source, &proposal("current")).unwrap();
                assert_eq!(
                    source.calls.borrow().as_slice(),
                    [PathBuf::from("skills/fixture")]
                );
            }

            #[test]
            fn proposal_sources_reject_stale_revision_without_retry() {
                let source = MockSource::current("skills/fixture", "new");
                let error = validate_proposal_sources(&source, &proposal("old")).unwrap_err();
                assert!(format!("{error:?}").contains("changed from old to new"));
                assert_eq!(source.calls.borrow().len(), 1);
            }

            #[test]
            fn proposal_sources_reject_conflicting_frozen_revisions_before_load() {
                let mut proposal = proposal("one");
                proposal
                    .holdout_cases
                    .push(key("skills/fixture", "two", "other"));
                let source = MockSource::current("skills/fixture", "one");
                let error = proposal_artifact_revisions(&proposal).unwrap_err();
                assert!(format!("{error:?}").contains("conflicting revisions"));
                assert!(source.calls.borrow().is_empty());
            }

            #[test]
            fn stale_source_prevents_suite_replacement() {
                let root = FixtureRoot::new("stale");
                let destination = root.0.join("suite.json");
                fs::write(&destination, b"original\n").unwrap();
                let source = MockSource::current("skills/fixture", "new");
                let mut runtime = FileSuiteRuntime {
                    source,
                    store: crate::frontier_store::FileFrontierStore::new(&root.0).unwrap(),
                };
                let result = runtime.apply_frontier_suite_proposal(
                    &proposal("old"),
                    Path::new("suite.json"),
                    &Timestamp("2026-01-01T00:00:00+0000".to_owned()),
                );
                assert!(result.is_err());
                assert_eq!(fs::read(destination).unwrap(), b"original\n");
                assert_eq!(runtime.source.calls.borrow().len(), 1);
            }

            #[test]
            fn source_errors_propagate_without_retry_or_store_write() {
                let root = FixtureRoot::new("source-error");
                let source = MockSource {
                    revisions: BTreeMap::new(),
                    calls: RefCell::new(Vec::new()),
                    is_failing: true,
                };
                let mut store = crate::frontier_store::FileFrontierStore::new(&root.0).unwrap();
                let error = apply_current_frontier_suite(
                    &source,
                    &mut store,
                    &proposal("old"),
                    Path::new("suite.json"),
                    &Timestamp("2026-01-01T00:00:00+0000".to_owned()),
                )
                .unwrap_err();
                assert!(matches!(error, SkillEvalError::NotFound(_)));
                assert_eq!(source.calls.borrow().len(), 1);
                assert!(!root.0.join("suite.json").exists());
            }

            struct FakeRunIds {
                calls: Cell<u8>,
                result: Result<RunId, SkillEvalError>,
            }

            impl RunIdSource for FakeRunIds {
                fn next(&mut self) -> Result<RunId, SkillEvalError> {
                    self.calls.set(self.calls.get() + 1);
                    self.result
                        .as_ref()
                        .map(Clone::clone)
                        .map_err(|_| SkillEvalError::NotFound("run id failed".to_owned()))
                }
            }

            #[test]
            fn frozen_plan_loader_accepts_an_aged_capability_snapshot() {
                let fixture = FixtureRoot::new("aged-capability");
                let suite = FrontierSuite {
                    version: 1,
                    tiers: BTreeMap::new(),
                };
                let suite_bytes = serde_json::to_vec(&suite).unwrap();
                fs::write(fixture.0.join("suite.json"), &suite_bytes).unwrap();
                let capability_bytes = b"{}";
                fs::write(fixture.0.join("capabilities.json"), capability_bytes).unwrap();
                let mut plan = state().configuration.plan;
                plan.suite.sha256 = sha256_digest(&suite_bytes);
                plan.capabilities.sha256 = sha256_digest(capability_bytes);
                plan.capabilities.observed_at_unix_seconds = 1;
                fs::write(
                    fixture.0.join("plan.json"),
                    serde_json::to_vec(&plan).unwrap(),
                )
                .unwrap();
                let source = MockSource {
                    revisions: BTreeMap::new(),
                    calls: RefCell::new(Vec::new()),
                    is_failing: false,
                };

                let loaded =
                    load_frontier_plan_files(&fixture.0, &source, Path::new("plan.json")).unwrap();

                assert_eq!(loaded, (plan, suite));
                assert!(source.calls.borrow().is_empty());
            }

            #[test]
            fn frontier_run_id_preserves_source_identity_and_errors_without_retry() {
                let mut source = FakeRunIds {
                    calls: Cell::new(0),
                    result: Ok(RunId("run-exact".to_owned())),
                };
                assert_eq!(
                    next_frontier_run_id(&mut source).unwrap(),
                    FrontierRunId("frontier-run-exact".to_owned())
                );
                assert_eq!(source.calls.get(), 1);

                let mut failing = FakeRunIds {
                    calls: Cell::new(0),
                    result: Err(SkillEvalError::NotFound("failure".to_owned())),
                };
                assert!(next_frontier_run_id(&mut failing).is_err());
                assert_eq!(failing.calls.get(), 1);
            }

            fn state() -> FrontierRunState {
                FrontierRunState {
                    configuration: FrontierRunConfiguration {
                        run_id: FrontierRunId("frontier-test".to_owned()),
                        created_at: Timestamp("2026-01-01T00:00:00+0000".to_owned()),
                        plan_path: PathBuf::from("plan.json"),
                        plan_sha256: "c".repeat(64),
                        plan: FrontierPlan {
                            version: 1,
                            suite: crate::model::FrontierSuiteIdentity {
                                path: PathBuf::from("suite.json"),
                                sha256: "d".repeat(64),
                                version: 1,
                            },
                            capabilities: T1ScreenSnapshotIdentity {
                                path: PathBuf::from("capabilities.json"),
                                sha256: "e".repeat(64),
                                version: 1,
                                observed_at_unix_seconds: 1,
                                pi_version: "pi".to_owned(),
                            },
                            entrants: Vec::new(),
                            judge: ModelIdentity {
                                tier: Tier::T5,
                                provider: "anthropic".to_owned(),
                                model: "judge".to_owned(),
                                thinking: "high".to_owned(),
                            },
                            policy: FrontierPolicy {
                                screening_trials_per_case: 1,
                                confirmation_trials_per_case: 3,
                                maximum_trials_per_case: 5,
                                minimum_trial_score: 8,
                                minimum_weighted_pass_basis_points: 8_500,
                                minimum_lower_bound_basis_points: 8_000,
                                confidence_level_basis_points: 9_500,
                                confidence_method: FrontierConfidenceMethod::StratifiedBootstrap,
                                confidence_resamples: 1_000,
                                maximum_infrastructure_attempts: 2,
                                maximum_catalog_age_seconds: 3_600,
                                active_pool_size: 1,
                                maximum_trial_cost_millionths_of_dollar: 1,
                                spending_limit_millionths_of_dollar: 10,
                                is_provider_limit_enforced: true,
                                is_first_party_only: true,
                            },
                        },
                    },
                    status: FrontierRunStatus::Running,
                    models: Vec::new(),
                    cells: Vec::new(),
                    infrastructure_events: Vec::new(),
                    pause: None,
                    decision: None,
                    spent_millionths_of_dollar: 7,
                }
            }

            #[test]
            fn frontier_progress_output_is_deterministic() {
                let mut output = Vec::new();
                RenderFrontierProgress {
                    format: OutputFormat::Text,
                    output: &mut output,
                }
                .emit_frontier(&state())
                .unwrap();
                assert_eq!(
                    String::from_utf8(output).unwrap(),
                    "frontier frontier-test: Running, spent 7 millionths\n"
                );
            }

            struct FailingWriter;

            impl Write for FailingWriter {
                fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                    Err(io::Error::other("writer failed"))
                }

                fn flush(&mut self) -> io::Result<()> {
                    Ok(())
                }
            }

            #[test]
            fn frontier_progress_propagates_writer_errors() {
                let error = RenderFrontierProgress {
                    format: OutputFormat::JsonLines,
                    output: &mut FailingWriter,
                }
                .emit_frontier(&state())
                .unwrap_err();
                assert!(matches!(error, SkillEvalError::Io { .. }));
            }
        }
    };
}
