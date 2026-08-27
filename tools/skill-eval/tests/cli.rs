#[macro_export]
macro_rules! cli_tests {
    () => {
        mod command_line {
            use std::ffi::OsString;
            use std::path::{Path, PathBuf};
            #[cfg(unix)]
            use std::os::unix::fs::{PermissionsExt, symlink};
            #[cfg(unix)]
            use std::os::unix::net::UnixListener;
            #[cfg(unix)]
            use std::process::Command;
            #[cfg(unix)]
            use std::sync::atomic::{AtomicU64, Ordering};

            use super::{
                MODELS_RPC_ARGUMENTS, MODELS_RPC_REQUEST, PathRunIdSource, parse_arguments,
                parse_available_models_response, pi_available_models_command, render_event,
                render_t1_screen_campaign_cap_extension, render_t1_screen_report,
                run_t1_screen_campaign_extend_cap_at, run_t1_screen_campaign_retire_run_at,
                sha256_digest,
            };
            #[cfg(unix)]
            use super::{
                candidate_environment_manifest_at, run_t1_screen_fail_route_at, zero_t1_usage,
            };
            #[cfg(unix)]
            use $crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, CandidateEnvironmentEntry,
                CaseDefinition, CaseDrive, CaseId, ConfidenceInterval, ExecutionDefinition,
                HarnessIdentity, ModelIdentity, PauseReason, PoolEntrantEvidence, PoolStage,
                QualificationPolicy, T1ScreenAttemptReport, T1ScreenCallRange,
                T1ScreenCandidateEnvironment, T1ScreenCandidatePrice, T1ScreenChildStatus,
                T1ScreenEligibleRow, T1ScreenModelOutcome, T1ScreenModelState,
                T1ScreenPauseReason, T1ScreenPolicy, T1ScreenReport, T1ScreenRouteFailure,
                T1ScreenRouteFailureRequest, T1ScreenRunConfiguration, T1ScreenRunState,
                T1ScreenSnapshotIdentity, Tier, TierDestination, TrialKey, TrialUsage,
            };
            #[cfg(unix)]
            use $crate::ports::{Clock, RunStore};
            #[cfg(unix)]
            use $crate::service::t1_environment_difference;
            #[cfg(unix)]
            use $crate::store::FileRunStore;
            #[cfg(unix)]
            use $crate::t1_screen_campaign_store::T1_SCREEN_CAMPAIGN_APPROVED_TOTAL;
            #[cfg(unix)]
            use $crate::t1_screen_store::{
                FileT1ScreenStore, candidate_environment_manifest_digest,
                preallocate_t1_screen_children, t1_screen_classification_digest,
            };
            use $crate::model::{
                CliCommand, OutputFormat, QualificationPurpose, RunConfiguration, RunEvent, RunId,
                RunMode, T1ScreenCampaignCapExtensionRequest, T1ScreenCampaignId,
                T1ScreenCampaignRunEntry, T1ScreenCampaignRunRetirementRequest,
                T1ScreenCampaignState, T1ScreenCampaignStatus, T1ScreenFormat, T1ScreenRunId,
                T1ScreenRunStatus, Timestamp,
            };
            #[cfg(unix)]
            use $crate::model::SkillEvalError;
            use $crate::ports::RunIdSource;
            use $crate::t1_screen_campaign_store::FileT1ScreenCampaignStore;

            fn arguments(values: &[&str]) -> Vec<OsString> {
                values.iter().map(OsString::from).collect()
            }

            #[cfg(unix)]
            static CANDIDATE_FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

            #[cfg(unix)]
            struct CandidateFixture {
                root: PathBuf,
                agent_root: PathBuf,
                repository: PathBuf,
                extensions: PathBuf,
            }

            #[cfg(unix)]
            impl CandidateFixture {
                fn new() -> Self {
                    let sequence = CANDIDATE_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                    let root = std::env::temp_dir().join(format!(
                        "skill-eval-candidate-environment-{}-{sequence}",
                        std::process::id()
                    ));
                    let agent_root = root.join("home/.pi/agent");
                    let extensions = agent_root.join("extensions");
                    let repository = root.join("source");
                    std::fs::create_dir_all(&extensions).unwrap();
                    std::fs::create_dir_all(repository.join("bundle/bin")).unwrap();
                    std::fs::write(agent_root.join("settings.json"), b"settings-v1").unwrap();
                    std::fs::write(agent_root.join("models.json"), b"models-v1").unwrap();
                    std::fs::write(repository.join("extension.ts"), b"entry-v1").unwrap();
                    std::fs::write(repository.join("replacement.ts"), b"entry-v1").unwrap();
                    std::fs::write(repository.join("bundle/index.ts"), b"bundle-entry-v1").unwrap();
                    std::fs::write(repository.join("bundle/companion.json"), b"companion-v1")
                        .unwrap();
                    std::fs::write(
                        repository.join("bundle/package.json"),
                        br#"{"dependencies":{"linkedom":"1.0.0"}}"#,
                    )
                    .unwrap();
                    std::fs::write(
                        repository.join("bundle/package-lock.json"),
                        br#"{"lockfileVersion":3,"packages":{"":{"dependencies":{"linkedom":"1.0.0"}}}}"#,
                    )
                    .unwrap();
                    let binary = repository.join("bundle/bin/tool");
                    std::fs::write(&binary, b"binary-v1").unwrap();
                    let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
                    permissions.set_mode(0o755);
                    std::fs::set_permissions(&binary, permissions).unwrap();
                    std::fs::write(repository.join("unrelated.txt"), b"unrelated-v1").unwrap();
                    std::fs::write(repository.join("installer-manifest.json"), b"manifest-v1")
                        .unwrap();
                    run_git(&repository, &["init"]);
                    run_git(&repository, &["add", "--all"]);
                    run_git(
                        &repository,
                        &[
                            "-c",
                            "user.name=Skill Eval Test",
                            "-c",
                            "user.email=skill-eval@example.invalid",
                            "commit",
                            "-m",
                            "fixture",
                        ],
                    );
                    symlink(repository.join("extension.ts"), extensions.join("standalone.ts"))
                        .unwrap();
                    symlink(repository.join("bundle"), extensions.join("bundle")).unwrap();
                    Self {
                        root,
                        agent_root,
                        repository,
                        extensions,
                    }
                }

                fn manifest(&self) -> Vec<$crate::model::CandidateEnvironmentEntry> {
                    candidate_environment_manifest_at(&self.agent_root).unwrap()
                }

                fn digest(&self) -> String {
                    candidate_environment_manifest_digest(&self.manifest()).unwrap()
                }
            }

            #[cfg(unix)]
            impl Drop for CandidateFixture {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.root);
                }
            }

            #[cfg(unix)]
            struct RouteFailureChildIds(u64);

            #[cfg(unix)]
            impl RunIdSource for RouteFailureChildIds {
                fn next(&mut self) -> Result<RunId, SkillEvalError> {
                    let id = self.0;
                    self.0 += 1;
                    Ok(RunId(format!("route-child-{id}")))
                }
            }

            #[cfg(unix)]
            struct RouteFailureClock;

            #[cfg(unix)]
            impl Clock for RouteFailureClock {
                fn now(&self) -> Timestamp {
                    Timestamp("2026-08-26T04:00:00-0400".to_owned())
                }
            }

            #[cfg(unix)]
            fn matrix_evidence(
                model: ModelIdentity,
                judge: ModelIdentity,
                is_passing: bool,
            ) -> PoolEntrantEvidence {
                let usage = TrialUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    turns: 0,
                    tool_calls: 0,
                    elapsed_milliseconds: 0,
                    cost_millionths_of_dollar: 0,
                };
                PoolEntrantEvidence {
                    stage: PoolStage::Calibration,
                    requested_model: model.clone(),
                    effective_model: model,
                    judge_model: judge,
                    harnesses: Vec::new(),
                    is_passing,
                    completed_trials: 5,
                    expected_trials: 5,
                    failed_trials: if is_passing { 0 } else { 5 },
                    catastrophic_trials: 0,
                    score: ConfidenceInterval {
                        lower: 0.0,
                        estimate: if is_passing { 1.0 } else { 0.0 },
                        upper: 1.0,
                    },
                    candidate_usage: usage.clone(),
                    judge_usage: usage.clone(),
                    total_usage: usage,
                }
            }

            #[cfg(unix)]
            struct RouteFailureCliFixture {
                root: PathBuf,
                runs_root: PathBuf,
                paused: T1ScreenRunState,
                request: T1ScreenRouteFailureRequest,
                campaign_aggregate: u64,
            }

            #[cfg(unix)]
            impl RouteFailureCliFixture {
                fn new(label: &str) -> Self {
                    let sequence = CANDIDATE_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                    let root = std::env::temp_dir().join(format!(
                        "skill-eval-route-failure-cli-{label}-{}-{sequence}",
                        std::process::id()
                    ));
                    std::fs::create_dir(&root).unwrap();
                    let root = root.canonicalize().unwrap();
                    let runs_root = root.join("runs");
                    let eligible = vec![
                        T1ScreenEligibleRow {
                            provider: "fixture".to_owned(),
                            model: "alpha".to_owned(),
                            supported_pi_thinking_levels: vec!["off".to_owned()],
                            is_preview: false,
                        },
                        T1ScreenEligibleRow {
                            provider: "fixture".to_owned(),
                            model: "beta".to_owned(),
                            supported_pi_thinking_levels: vec!["high".to_owned()],
                            is_preview: false,
                        },
                    ];
                    let mut child_ids = RouteFailureChildIds(0);
                    let children = preallocate_t1_screen_children(&eligible, &mut child_ids).unwrap();
                    let failed_child = children[0].clone();
                    let exam = ArtifactDefinition {
                        name: ArtifactName("route-exam".to_owned()),
                        kind: ArtifactKind::Skill,
                        root: root.clone(),
                        revision: "route-exam-r1".to_owned(),
                        required_destinations: vec![TierDestination::SkillMinimum],
                        current_tiers: Vec::new(),
                        cases: (0..5)
                            .map(|index| CaseDefinition {
                                id: CaseId(format!("case-{index}")),
                                input: "input".to_owned(),
                                expect: "expect".to_owned(),
                                source: "fixture".to_owned(),
                                is_holdout: false,
                                support_files: Vec::new(),
                                execution: ExecutionDefinition {
                                    drive: CaseDrive::Response,
                                    allowed_tools: Vec::new(),
                                    timeout_seconds: 10,
                                },
                            })
                            .collect(),
                    };
                    let harness = HarnessIdentity {
                        runner_version: "runner-1".to_owned(),
                        pi_version: "pi-1".to_owned(),
                        artifact_revision: exam.revision.clone(),
                        tool_policy_digest: "tools-1".to_owned(),
                    };
                    let manifest = vec![CandidateEnvironmentEntry {
                        key: "pi-agent/settings.json".to_owned(),
                        sha256: "a".repeat(64),
                    }];
                    let initial = T1ScreenRunState {
                        configuration: T1ScreenRunConfiguration {
                            run_id: T1ScreenRunId("route-parent".to_owned()),
                            campaign_id: T1ScreenCampaignId("route-campaign".to_owned()),
                            created_at: Timestamp("2026-08-26T03:30:00-0400".to_owned()),
                            capability_snapshot: T1ScreenSnapshotIdentity {
                                path: root.clone(),
                                sha256: "b".repeat(64),
                                version: 1,
                                observed_at_unix_seconds: 1,
                                pi_version: "pi-1".to_owned(),
                            },
                            classification_sha256: t1_screen_classification_digest(&eligible, &[])
                                .unwrap(),
                            eligible,
                            excluded: Vec::new(),
                            exam,
                            judge: ModelIdentity {
                                tier: Tier::T5,
                                provider: "judge".to_owned(),
                                model: "fixed".to_owned(),
                                thinking: "high".to_owned(),
                            },
                            candidate_environment: T1ScreenCandidateEnvironment {
                                harnesses: vec![harness.clone(); 5],
                                digest: candidate_environment_manifest_digest(&manifest).unwrap(),
                                manifest,
                            },
                            policy: T1ScreenPolicy {
                                minimum_score: 8,
                                calibration_minimum_reliability_basis_points: 8_000,
                                maximum_catastrophic_trials: 0,
                                repeats_per_case: 1,
                                candidate_timeout_seconds: None,
                            },
                            is_complete_thinking_coverage: true,
                            candidate_calls: T1ScreenCallRange {
                                minimum: 10,
                                maximum: 10,
                            },
                            judge_calls: T1ScreenCallRange {
                                minimum: 10,
                                maximum: 10,
                            },
                            candidate_price: T1ScreenCandidatePrice {
                                input_per_million_tokens: 0,
                                output_per_million_tokens: 0,
                            },
                            owner_approved_judge_cap_millionths_of_dollar:
                                T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
                            provider_enforced_judge_cap_millionths_of_dollar:
                                T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
                        },
                        cap_extensions: Vec::new(),
                        route_failures: Vec::new(),
                        status: T1ScreenRunStatus::Pending,
                        child_runs: children,
                        models: vec![
                            T1ScreenModelState {
                                provider: "fixture".to_owned(),
                                model: "alpha".to_owned(),
                                attempts: Vec::new(),
                                outcome: None,
                            },
                            T1ScreenModelState {
                                provider: "fixture".to_owned(),
                                model: "beta".to_owned(),
                                attempts: Vec::new(),
                                outcome: None,
                            },
                        ],
                        candidate_usage: zero_t1_usage(),
                        judge_usage: zero_t1_usage(),
                        spent_judge_millionths_of_dollar: 0,
                        pause: None,
                    };
                    let mut parent_store = FileT1ScreenStore::new(&root).unwrap();
                    let mut campaign_store = FileT1ScreenCampaignStore::open(&root).unwrap();
                    campaign_store
                        .create(&T1ScreenCampaignState {
                            campaign_id: initial.configuration.campaign_id.clone(),
                            created_at: Timestamp("2026-08-26T03:00:00-0400".to_owned()),
                            approved_judge_total_millionths_of_dollar:
                                T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
                            cap_extensions: Vec::new(),
                            retirements: Vec::new(),
                            aggregate_judge_spent_millionths_of_dollar: 0,
                            runs: Vec::new(),
                            active_run_id: None,
                            owner_reason: "Owner approved the campaign".to_owned(),
                            status: T1ScreenCampaignStatus::Open,
                        })
                        .unwrap();
                    parent_store.create(&initial).unwrap();
                    let mut running = initial;
                    running.status = T1ScreenRunStatus::Running;
                    running.child_runs[0].status = T1ScreenChildStatus::Running;
                    parent_store.save(&running).unwrap();
                    campaign_store.register_active_run(&running).unwrap();

                    let mut child_store = FileRunStore::new(&runs_root).unwrap();
                    child_store
                        .append(
                            &failed_child.run_id,
                            &RunEvent::RunStarted {
                                at: Timestamp("2026-08-26T03:31:00-0400".to_owned()),
                                configuration: RunConfiguration {
                                    run_id: failed_child.run_id.clone(),
                                    mode: RunMode::Execute,
                                    artifacts: Vec::new(),
                                    change: None,
                                    policy: QualificationPolicy {
                                        purpose: QualificationPurpose::Artifact,
                                        candidate_tiers: vec![Tier::T1],
                                        reference_tier: Tier::T4,
                                        judge_tier: Tier::T5,
                                        repeats_per_case: 1,
                                        minimum_score: 8,
                                        noninferiority_margin: 0.1,
                                        confidence_level: 0.95,
                                    },
                                    qualification_routes: Default::default(),
                                    created_at: Timestamp(
                                        "2026-08-26T03:31:00-0400".to_owned(),
                                    ),
                                },
                            },
                        )
                        .unwrap();
                    child_store
                        .append(
                            &failed_child.run_id,
                            &RunEvent::TrialStarted {
                                at: Timestamp("2026-08-26T03:32:00-0400".to_owned()),
                                key: TrialKey {
                                    artifact: ArtifactName("route-exam".to_owned()),
                                    tier: Tier::T1,
                                    route_index: 0,
                                    case: CaseId("case-0".to_owned()),
                                    attempt: 1,
                                },
                                models: vec![failed_child.model.clone()],
                                harness,
                            },
                        )
                        .unwrap();
                    let paused_message = "exact saved infrastructure failure";
                    child_store
                        .append(
                            &failed_child.run_id,
                            &RunEvent::RunPaused {
                                at: Timestamp("2026-08-26T03:33:00-0400".to_owned()),
                                reason: PauseReason::Infrastructure {
                                    message: paused_message.to_owned(),
                                },
                            },
                        )
                        .unwrap();

                    let mut paused = running;
                    paused.status = T1ScreenRunStatus::Paused;
                    paused.child_runs[0].status = T1ScreenChildStatus::Paused;
                    paused.pause = Some(T1ScreenPauseReason::Infrastructure {
                        message: paused_message.to_owned(),
                    });
                    parent_store.save(&paused).unwrap();
                    let campaign = campaign_store.reconcile_active_run(&paused).unwrap();
                    assert_eq!(campaign.status, T1ScreenCampaignStatus::Paused);
                    let request = T1ScreenRouteFailureRequest {
                        run_id: paused.configuration.run_id.clone(),
                        child_run_id: failed_child.run_id,
                        owner_reason: "Owner accepted this exact route failure".to_owned(),
                    };
                    Self {
                        root,
                        runs_root,
                        paused,
                        request,
                        campaign_aggregate: campaign
                            .aggregate_judge_spent_millionths_of_dollar,
                    }
                }

                fn assert_saved(&self) -> T1ScreenRunState {
                    let saved = FileT1ScreenStore::open(&self.root)
                        .unwrap()
                        .load(&self.request.run_id)
                        .unwrap();
                    let failed_model = self.paused.child_runs[0].model.clone();
                    assert_eq!(
                        saved.route_failures,
                        [T1ScreenRouteFailure {
                            timestamp: Timestamp("2026-08-26T04:00:00-0400".to_owned()),
                            child_run_id: self.request.child_run_id.clone(),
                            model: failed_model.clone(),
                            paused_message_sha256: sha256_digest(
                                b"exact saved infrastructure failure"
                            ),
                            owner_reason: self.request.owner_reason.clone(),
                        }]
                    );
                    assert_eq!(
                        saved.models[0].outcome,
                        Some(T1ScreenModelOutcome::InfrastructureFailed {
                            model: failed_model,
                            child_run_id: self.request.child_run_id.clone(),
                        })
                    );
                    assert!(saved.models[1].outcome.is_none());
                    assert_eq!(saved.child_runs[0].status, T1ScreenChildStatus::Failed);
                    assert_eq!(saved.child_runs[1].status, T1ScreenChildStatus::Pending);
                    assert_eq!(saved.status, T1ScreenRunStatus::Running);
                    assert!(saved.pause.is_none());
                    assert_eq!(saved.candidate_usage, self.paused.candidate_usage);
                    assert_eq!(saved.judge_usage, self.paused.judge_usage);
                    assert_eq!(
                        saved.spent_judge_millionths_of_dollar,
                        self.paused.spent_judge_millionths_of_dollar
                    );
                    let campaign = FileT1ScreenCampaignStore::open(&self.root)
                        .unwrap()
                        .load(&saved.configuration.campaign_id)
                        .unwrap();
                    assert_eq!(campaign.runs.len(), 1);
                    assert_eq!(campaign.status, T1ScreenCampaignStatus::Open);
                    assert_eq!(campaign.active_run_id.as_ref(), Some(&self.request.run_id));
                    assert_eq!(
                        campaign.aggregate_judge_spent_millionths_of_dollar,
                        self.campaign_aggregate
                    );
                    assert_eq!(campaign.runs[0].run_id, self.request.run_id);
                    assert_eq!(campaign.runs[0].observed_status, T1ScreenRunStatus::Running);
                    assert_eq!(
                        campaign.runs[0].judge_spend_millionths_of_dollar,
                        self.paused.spent_judge_millionths_of_dollar
                    );
                    assert_eq!(
                        campaign.runs[0].candidate_cost_millionths_of_dollar,
                        self.paused.candidate_usage.cost_millionths_of_dollar
                    );
                    saved
                }
            }

            #[cfg(unix)]
            impl Drop for RouteFailureCliFixture {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.root);
                }
            }

            #[cfg(unix)]
            fn retirement_cli_fixture(label: &str) -> (PathBuf, T1ScreenCampaignRunRetirementRequest) {
                let sequence = CANDIDATE_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let root = std::env::temp_dir().join(format!(
                    "skill-eval-campaign-retirement-cli-{label}-{}-{sequence}",
                    std::process::id()
                ));
                std::fs::create_dir(&root).unwrap();
                let root = root.canonicalize().unwrap();
                let campaign_id = T1ScreenCampaignId("campaign".to_owned());
                let run_id = T1ScreenRunId("paused-run".to_owned());
                let mut store = FileT1ScreenCampaignStore::new(&root).unwrap();
                let run_directory = root.join(".map/skill-eval/t1-screening/paused-run");
                std::fs::create_dir(&run_directory).unwrap();
                let run_path = run_directory.join("state.json");
                let run_bytes = serde_json::to_vec_pretty(&serde_json::json!({
                    "configuration": {
                        "run_id": "paused-run",
                        "campaign_id": "campaign",
                        "created_at": "2026-08-26T03:30:00-0400",
                        "candidate_environment": {
                            "manifest": [{
                                "key": "fixture",
                                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            }]
                        }
                    },
                    "status": "paused",
                    "spent_judge_millionths_of_dollar": 7,
                    "candidate_usage": {"cost_millionths_of_dollar": 0}
                }))
                .unwrap();
                std::fs::write(&run_path, &run_bytes).unwrap();
                store
                    .create(&T1ScreenCampaignState {
                        campaign_id: campaign_id.clone(),
                        created_at: Timestamp("2026-08-26T03:00:00-0400".to_owned()),
                        approved_judge_total_millionths_of_dollar: 20_000_000,
                        cap_extensions: Vec::new(),
                        retirements: Vec::new(),
                        aggregate_judge_spent_millionths_of_dollar: 7,
                        runs: vec![T1ScreenCampaignRunEntry {
                            run_id: run_id.clone(),
                            canonical_state_path: run_path,
                            state_file_sha256: sha256_digest(&run_bytes),
                            created_at: Timestamp("2026-08-26T03:30:00-0400".to_owned()),
                            observed_status: T1ScreenRunStatus::Paused,
                            judge_spend_millionths_of_dollar: 7,
                            candidate_cost_millionths_of_dollar: 0,
                            is_resumable: true,
                            superseded_reason: None,
                        }],
                        active_run_id: Some(run_id.clone()),
                        owner_reason: "Initial owner approval".to_owned(),
                        status: T1ScreenCampaignStatus::Paused,
                    })
                    .unwrap();
                (
                    root,
                    T1ScreenCampaignRunRetirementRequest {
                        campaign_id,
                        run_id,
                        owner_reason: "Owner retired the paused run".to_owned(),
                    },
                )
            }

            #[cfg(unix)]
            fn run_git(repository: &Path, arguments: &[&str]) {
                let output = Command::new("git")
                    .arg("-C")
                    .arg(repository)
                    .args(arguments)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "git {arguments:?} failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }

            #[cfg(unix)]
            fn assert_candidate_difference(
                fixture: &CandidateFixture,
                previous: &mut Vec<$crate::model::CandidateEnvironmentEntry>,
                expected: &str,
            ) {
                let current = fixture.manifest();
                assert_eq!(
                    t1_environment_difference(previous, &current).as_deref(),
                    Some(expected)
                );
                *previous = current;
            }

            #[cfg(unix)]
            #[test]
            fn candidate_environment_manifest_is_exact_deterministic_and_round_trips() {
                let fixture = CandidateFixture::new();
                let first = fixture.manifest();
                let second = fixture.manifest();
                assert_eq!(first, second);
                assert!(first.iter().all(|entry| {
                    entry.sha256.len() == 64
                        && entry
                            .sha256
                            .chars()
                            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
                }));
                assert_eq!(
                    first.iter().map(|entry| entry.key.as_str()).collect::<Vec<_>>(),
                    [
                        "extensions/bundle",
                        "extensions/bundle/canonical-path",
                        "extensions/bundle/files/bin/tool",
                        "extensions/bundle/files/companion.json",
                        "extensions/bundle/files/index.ts",
                        "extensions/bundle/files/node_modules/.package-lock.json",
                        "extensions/bundle/files/package-lock.json",
                        "extensions/bundle/files/package.json",
                        "extensions/standalone.ts",
                        "extensions/standalone.ts/canonical-path",
                        "extensions/standalone.ts/content",
                        "pi-agent/models.json",
                        "pi-agent/settings.json",
                    ]
                );
                assert_eq!(
                    first
                        .iter()
                        .find(|entry| entry.key == "pi-agent/settings.json")
                        .unwrap()
                        .sha256,
                    sha256_digest(b"settings-v1")
                );
                assert_eq!(
                    first
                        .iter()
                        .find(|entry| entry.key == "extensions/bundle/canonical-path")
                        .unwrap()
                        .sha256,
                    sha256_digest(
                        fixture
                            .repository
                            .join("bundle")
                            .canonicalize()
                            .unwrap()
                            .as_os_str()
                            .as_encoded_bytes()
                    )
                );
                let serialized = serde_json::to_vec(&first).unwrap();
                let round_trip: Vec<$crate::model::CandidateEnvironmentEntry> =
                    serde_json::from_slice(&serialized).unwrap();
                assert_eq!(round_trip, first);
                assert_eq!(
                    candidate_environment_manifest_digest(&round_trip).unwrap(),
                    fixture.digest()
                );
            }

            #[cfg(unix)]
            #[test]
            fn candidate_environment_manifest_serializes_no_file_bytes_or_credentials() {
                let fixture = CandidateFixture::new();
                let credential = "api-key-raw-secret-value";
                std::fs::write(fixture.agent_root.join("settings.json"), credential).unwrap();
                std::fs::write(
                    fixture.repository.join("bundle/bin/tool"),
                    b"raw-binary-secret-value",
                )
                .unwrap();

                let serialized = serde_json::to_string(&fixture.manifest()).unwrap();
                assert!(!serialized.contains(credential));
                assert!(!serialized.contains("raw-binary-secret-value"));
                assert!(!serialized.contains("settings-v1"));
                assert!(!serialized.contains("models-v1"));
            }

            #[cfg(unix)]
            #[test]
            fn candidate_environment_manifest_order_ignores_directory_iteration_order() {
                let fixture = CandidateFixture::new();
                let first_path = fixture.extensions.join("a-first.ts");
                let last_path = fixture.extensions.join("z-last.ts");
                std::fs::write(&last_path, b"z").unwrap();
                std::fs::write(&first_path, b"a").unwrap();
                let first = fixture.manifest();

                std::fs::remove_file(&first_path).unwrap();
                std::fs::remove_file(&last_path).unwrap();
                std::fs::write(&first_path, b"a").unwrap();
                std::fs::write(&last_path, b"z").unwrap();
                assert_eq!(fixture.manifest(), first);
            }

            #[cfg(unix)]
            #[test]
            fn candidate_environment_ignores_unrelated_repository_identity() {
                let fixture = CandidateFixture::new();
                let expected = fixture.digest();

                std::fs::write(fixture.repository.join("unrelated.txt"), b"unrelated-v2").unwrap();
                std::fs::write(
                    fixture.repository.join("installer-manifest.json"),
                    b"manifest-v2",
                )
                .unwrap();
                std::fs::write(fixture.repository.join("untracked.txt"), b"untracked").unwrap();
                assert_eq!(fixture.digest(), expected);

                run_git(&fixture.repository, &["add", "--all"]);
                run_git(
                    &fixture.repository,
                    &[
                        "-c",
                        "user.name=Skill Eval Test",
                        "-c",
                        "user.email=skill-eval@example.invalid",
                        "commit",
                        "-m",
                        "unrelated changes",
                    ],
                );
                assert_eq!(fixture.digest(), expected);
            }

            #[cfg(unix)]
            #[test]
            fn candidate_environment_tracks_exact_executable_inputs() {
                let fixture = CandidateFixture::new();
                let mut previous = fixture.manifest();

                std::fs::write(fixture.repository.join("extension.ts"), b"entry-v2").unwrap();
                assert_candidate_difference(
                    &fixture,
                    &mut previous,
                    "changed extensions/standalone.ts/content",
                );
                std::fs::write(
                    fixture.repository.join("bundle/companion.json"),
                    b"companion-v2",
                )
                .unwrap();
                assert_candidate_difference(
                    &fixture,
                    &mut previous,
                    "changed extensions/bundle/files/companion.json",
                );
                std::fs::write(fixture.repository.join("bundle/bin/tool"), b"binary-v2").unwrap();
                assert_candidate_difference(
                    &fixture,
                    &mut previous,
                    "changed extensions/bundle/files/bin/tool",
                );
                std::fs::write(fixture.agent_root.join("settings.json"), b"settings-v2").unwrap();
                assert_candidate_difference(
                    &fixture,
                    &mut previous,
                    "changed pi-agent/settings.json",
                );
                std::fs::write(fixture.agent_root.join("models.json"), b"models-v2").unwrap();
                assert_candidate_difference(
                    &fixture,
                    &mut previous,
                    "changed pi-agent/models.json",
                );
                std::fs::write(
                    fixture.repository.join("bundle/package-lock.json"),
                    br#"{"lockfileVersion":3,"packages":{"":{"dependencies":{"linkedom":"2.0.0"}}}}"#,
                )
                .unwrap();
                assert_candidate_difference(
                    &fixture,
                    &mut previous,
                    "changed extensions/bundle/files/package-lock.json",
                );

                let replacement = fixture.repository.join("replacement.ts");
                std::fs::write(
                    &replacement,
                    std::fs::read(fixture.repository.join("extension.ts")).unwrap(),
                )
                .unwrap();
                std::fs::remove_file(fixture.extensions.join("standalone.ts")).unwrap();
                symlink(&replacement, fixture.extensions.join("standalone.ts")).unwrap();
                assert_candidate_difference(
                    &fixture,
                    &mut previous,
                    "changed extensions/standalone.ts/canonical-path",
                );

                std::fs::write(fixture.extensions.join("added.ts"), b"added").unwrap();
                assert_candidate_difference(&fixture, &mut previous, "added extensions/added.ts");
                std::fs::remove_file(fixture.extensions.join("added.ts")).unwrap();
                assert_candidate_difference(
                    &fixture,
                    &mut previous,
                    "removed extensions/added.ts",
                );
                std::fs::rename(
                    fixture.extensions.join("bundle"),
                    fixture.extensions.join("renamed-bundle"),
                )
                .unwrap();
                assert_candidate_difference(
                    &fixture,
                    &mut previous,
                    "removed extensions/bundle",
                );
                std::fs::remove_file(fixture.extensions.join("standalone.ts")).unwrap();
                assert_candidate_difference(
                    &fixture,
                    &mut previous,
                    "removed extensions/standalone.ts",
                );
            }

            #[cfg(unix)]
            #[test]
            fn candidate_environment_tracks_installed_package_lock_without_package_bulk() {
                let fixture = CandidateFixture::new();
                let mut previous = fixture.manifest();
                let marker_key = "extensions/bundle/files/node_modules/.package-lock.json";
                assert_eq!(
                    previous
                        .iter()
                        .find(|entry| entry.key == marker_key)
                        .unwrap()
                        .sha256,
                    sha256_digest(b"missing")
                );

                std::fs::create_dir(fixture.repository.join("bundle/node_modules")).unwrap();
                std::fs::write(
                    fixture
                        .repository
                        .join("bundle/node_modules/.package-lock.json"),
                    br#"{"packages":{"node_modules/linkedom":{"version":"1.0.0"}}}"#,
                )
                .unwrap();
                std::fs::create_dir_all(
                    fixture
                        .repository
                        .join("bundle/node_modules/linkedom/large"),
                )
                .unwrap();
                std::fs::File::create(
                    fixture
                        .repository
                        .join("bundle/node_modules/linkedom/large/ignored.bin"),
                )
                .unwrap()
                .set_len(70 * 1024 * 1024)
                .unwrap();
                assert_candidate_difference(
                    &fixture,
                    &mut previous,
                    &format!("changed {marker_key}"),
                );
                assert_eq!(
                    previous
                        .iter()
                        .find(|entry| entry.key == marker_key)
                        .unwrap()
                        .sha256,
                    sha256_digest(br#"{"packages":{"node_modules/linkedom":{"version":"1.0.0"}}}"#)
                );

                std::fs::write(
                    fixture
                        .repository
                        .join("bundle/node_modules/.package-lock.json"),
                    br#"{"packages":{"node_modules/linkedom":{"version":"2.0.0"}}}"#,
                )
                .unwrap();
                assert_candidate_difference(
                    &fixture,
                    &mut previous,
                    &format!("changed {marker_key}"),
                );
            }

            #[cfg(unix)]
            #[test]
            fn candidate_environment_rejects_unsupported_extension_entries() {
                let fixture = CandidateFixture::new();
                let socket = PathBuf::from(format!(
                    "/tmp/skill-eval-unsupported-{}.socket",
                    std::process::id()
                ));
                let _ = std::fs::remove_file(&socket);
                let listener = UnixListener::bind(&socket).unwrap();
                symlink(&socket, fixture.extensions.join("unsupported.socket")).unwrap();

                assert!(matches!(
                    candidate_environment_manifest_at(&fixture.agent_root),
                    Err(SkillEvalError::InvalidConfiguration(message))
                        if message.contains("unsupported type")
                ));
                drop(listener);
                std::fs::remove_file(socket).unwrap();
            }

            #[cfg(unix)]
            #[test]
            fn candidate_environment_rejects_directory_cycles() {
                let fixture = CandidateFixture::new();
                symlink(
                    fixture.repository.join("bundle"),
                    fixture.repository.join("bundle/cycle"),
                )
                .unwrap();

                let error = candidate_environment_manifest_at(&fixture.agent_root).unwrap_err();
                assert!(
                    matches!(
                        &error,
                        SkillEvalError::InvalidConfiguration(message)
                            if message.contains("directory cycle")
                    ),
                    "{error:?}"
                );
            }

            #[cfg(unix)]
            #[test]
            fn candidate_environment_rejects_extension_size_overflow() {
                let fixture = CandidateFixture::new();
                let oversized = fixture.repository.join("bundle/oversized.bin");
                std::fs::File::create(&oversized)
                    .unwrap()
                    .set_len(64 * 1024 * 1024 + 1)
                    .unwrap();

                assert!(matches!(
                    candidate_environment_manifest_at(&fixture.agent_root),
                    Err(SkillEvalError::InvalidConfiguration(message))
                        if message.contains("identity size limit")
                ));
            }

            #[test]
            fn available_models_probe_uses_one_non_prompt_rpc_command() {
                let command = pi_available_models_command("pi");
                let arguments = command
                    .get_args()
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                let request: serde_json::Value =
                    serde_json::from_slice(MODELS_RPC_REQUEST).expect("request must be JSON");

                assert_eq!(arguments, MODELS_RPC_ARGUMENTS);
                assert_eq!(request["id"], "skill-eval-models");
                assert_eq!(request["type"], "get_available_models");
                assert!(request.get("message").is_none());
            }

            #[test]
            fn available_models_probe_rejects_malformed_and_duplicate_responses() {
                let valid = r#"{"id":"skill-eval-models","type":"response","command":"get_available_models","success":true,"data":{"models":[]}}"#;
                for output in [
                    "not-json\n".to_owned(),
                    format!("{valid}\n{valid}\n"),
                    valid.replace("skill-eval-models", "wrong-id"),
                    valid.replace("get_available_models", "prompt"),
                ] {
                    assert!(
                        parse_available_models_response(&output).is_err(),
                        "accepted {output:?}"
                    );
                }
            }

            #[test]
            fn available_models_probe_rejects_missing_or_invalid_model_data() {
                for data in [
                    r#"{}"#,
                    r#"{"models":null}"#,
                    r#"{"models":[{"provider":"anthropic","id":"model","reasoning":"yes"}]}"#,
                    r#"{"models":[{"provider":"anthropic","id":"model","reasoning":true,"thinkingLevelMap":{"turbo":"turbo"}}]}"#,
                ] {
                    let response = format!(
                        r#"{{"id":"skill-eval-models","type":"response","command":"get_available_models","success":true,"data":{data}}}"#
                    );
                    assert!(
                        parse_available_models_response(&response).is_err(),
                        "accepted {data}"
                    );
                }
            }

            #[test]
            fn model_capabilities_requires_one_safe_repository_output() {
                let request = parse_arguments(&arguments(&[
                    "model-capabilities",
                    "--output",
                    "snapshots/models.json",
                ]))
                .unwrap();
                assert!(matches!(
                    request.command,
                    CliCommand::ModelCapabilities { ref output }
                        if output == Path::new("snapshots/models.json")
                ));

                for case in [
                    vec!["model-capabilities"],
                    vec!["model-capabilities", "--output", ""],
                    vec!["model-capabilities", "--output", "../models.json"],
                    vec!["model-capabilities", "--output", "/tmp/models.json"],
                    vec![
                        "model-capabilities",
                        "--output",
                        "one.json",
                        "--output",
                        "two.json",
                    ],
                    vec![
                        "model-capabilities",
                        "--output",
                        "models.json",
                        "--format",
                        "jsonl",
                    ],
                    vec![
                        "model-capabilities",
                        "--output",
                        "models.json",
                        "--unknown",
                    ],
                ] {
                    assert!(parse_arguments(&arguments(&case)).is_err(), "{case:?}");
                }
            }

            #[test]
            fn t1_screen_commands_parse_complete_safe_shapes() {
                let start = parse_arguments(&arguments(&[
                    "t1-screen-start",
                    "--campaign",
                    "campaign-1",
                    "--capabilities",
                    "research/models.json",
                    "--exam",
                    "tools/exam",
                    "--judge-cap-millionths",
                    "100",
                    "--provider-cap-millionths",
                    "80",
                    "--run-id-file",
                    ".scratch/t1-id",
                    "--format",
                    "json",
                ]))
                .unwrap();
                assert!(matches!(
                    start.command,
                    CliCommand::T1ScreenStart { request, .. }
                        if request.owner_approved_judge_cap_millionths_of_dollar == 100
                            && request.provider_enforced_judge_cap_millionths_of_dollar == 80
                ));

                for command in ["t1-screen-resume", "t1-screen-report"] {
                    let request = parse_arguments(&arguments(&[
                        command,
                        "--run",
                        "t1-screen-safe_1",
                        "--format",
                        "text",
                    ]))
                    .unwrap();
                    assert!(matches!(
                        request.command,
                        CliCommand::T1ScreenResume { .. } | CliCommand::T1ScreenReport { .. }
                    ));
                }
            }

            #[test]
            fn t1_screen_parser_rejects_duplicates_unsafe_paths_and_invalid_caps() {
                let valid = [
                    "t1-screen-start",
                    "--campaign",
                    "campaign-1",
                    "--capabilities",
                    "research/models.json",
                    "--exam",
                    "tools/exam",
                    "--judge-cap-millionths",
                    "100",
                    "--provider-cap-millionths",
                    "80",
                ];
                for case in [
                    vec!["t1-screen-start"],
                    vec![
                        "t1-screen-start",
                        "--capabilities",
                        "../models.json",
                        "--exam",
                        "tools/exam",
                        "--judge-cap-millionths",
                        "100",
                        "--provider-cap-millionths",
                        "80",
                    ],
                    vec![
                        "t1-screen-start",
                        "--capabilities",
                        "research/models.json",
                        "--capabilities",
                        "research/models-2.json",
                        "--exam",
                        "tools/exam",
                        "--judge-cap-millionths",
                        "100",
                        "--provider-cap-millionths",
                        "80",
                    ],
                    vec![
                        "t1-screen-start",
                        "--capabilities",
                        "research/models.json",
                        "--exam",
                        "tools/exam",
                        "--judge-cap-millionths",
                        "0",
                        "--provider-cap-millionths",
                        "0",
                    ],
                    vec![
                        "t1-screen-start",
                        "--capabilities",
                        "research/models.json",
                        "--exam",
                        "tools/exam",
                        "--judge-cap-millionths",
                        "80",
                        "--provider-cap-millionths",
                        "100",
                    ],
                ] {
                    assert!(parse_arguments(&arguments(&case)).is_err(), "{case:?}");
                }
                assert!(parse_arguments(&arguments(&valid)).is_ok());
                for value in ["", "../screen", "screen/name", "screen name", "screen:1"] {
                    assert!(
                        parse_arguments(&arguments(&[
                            "t1-screen-report",
                            "--run",
                            value,
                        ]))
                        .is_err(),
                        "accepted {value:?}"
                    );
                }
            }

            #[test]
            fn t1_screen_fail_route_parser_is_strict() {
                for format in ["text", "json"] {
                    let parsed = parse_arguments(&arguments(&[
                        "t1-screen-fail-route",
                        "--run",
                        "parent-1",
                        "--child",
                        "child-1",
                        "--reason",
                        "Owner accepted this exact route failure",
                        "--format",
                        format,
                    ]))
                    .unwrap();
                    assert!(matches!(
                        parsed.command,
                        CliCommand::T1ScreenFailRoute { request, format: parsed_format }
                            if request.run_id == T1ScreenRunId("parent-1".to_owned())
                                && request.child_run_id == RunId("child-1".to_owned())
                                && !request.owner_reason.trim().is_empty()
                                && parsed_format
                                    == if format == "text" {
                                        T1ScreenFormat::Text
                                    } else {
                                        T1ScreenFormat::Json
                                    }
                    ));
                }
                for case in [
                    vec!["t1-screen-fail-route"],
                    vec![
                        "t1-screen-fail-route",
                        "--run",
                        "parent",
                        "--child",
                        "child",
                    ],
                    vec![
                        "t1-screen-fail-route",
                        "--run",
                        "../parent",
                        "--child",
                        "child",
                        "--reason",
                        "approved",
                    ],
                    vec![
                        "t1-screen-fail-route",
                        "--run",
                        "parent",
                        "--child",
                        "child/name",
                        "--reason",
                        "approved",
                    ],
                    vec![
                        "t1-screen-fail-route",
                        "--run",
                        "parent",
                        "--child",
                        "child",
                        "--reason",
                        "   ",
                    ],
                    vec![
                        "t1-screen-fail-route",
                        "--run",
                        "parent",
                        "--run",
                        "other",
                        "--child",
                        "child",
                        "--reason",
                        "approved",
                    ],
                    vec![
                        "t1-screen-fail-route",
                        "--run",
                        "parent",
                        "--child",
                        "child",
                        "--reason",
                        "approved",
                        "--format",
                        "jsonl",
                    ],
                    vec![
                        "t1-screen-fail-route",
                        "--run",
                        "parent",
                        "--child",
                        "child",
                        "--reason",
                        "approved",
                        "--unknown",
                    ],
                ] {
                    assert!(parse_arguments(&arguments(&case)).is_err(), "{case:?}");
                }
            }

            #[cfg(unix)]
            #[test]
            fn tier_result_matrix_leads_t1_screen_report_and_json_stays_lossless() {
                let text_fixture = RouteFailureCliFixture::new("text");
                let mut text = Vec::new();
                run_t1_screen_fail_route_at(
                    &text_fixture.root,
                    &text_fixture.request,
                    &text_fixture.runs_root,
                    &RouteFailureClock,
                    T1ScreenFormat::Text,
                    &mut text,
                )
                .unwrap();
                let text_saved = text_fixture.assert_saved();
                let text = String::from_utf8(text).unwrap();
                let failure = &text_saved.route_failures[0];
                assert!(text.starts_with(concat!(
                    "| Model | off | minimal | low | medium | high | xhigh | max |\n",
                    "| --- | --- | --- | --- | --- | --- | --- | --- |\n",
                    "| fixture/alpha |  |  |  |  |  |  |  |\n",
                    "| fixture/beta |  |  |  |  |  |  |  |\n\n",
                )));
                assert!(text.contains("T1 screen route-parent: Running"));
                assert!(text.contains(
                    "campaign route-campaign: Open; total 20000000, spent 0, remaining 20000000; active route-parent"
                ));
                assert!(text.contains(&format!(
                    "{}: child {} exact fixture/alpha (T1; off); pause sha256 {}; reason {}",
                    failure.timestamp.0,
                    failure.child_run_id.0,
                    failure.paused_message_sha256,
                    failure.owner_reason
                )));
                assert!(text.contains(
                    "model fixture/alpha: infrastructure_failed fixture/alpha (T1; off) child route-child-0"
                ));
                assert!(text.contains("model fixture/beta: pending"));
                assert!(text.contains("active child: none"));

                let json_fixture = RouteFailureCliFixture::new("json");
                let mut json = Vec::new();
                run_t1_screen_fail_route_at(
                    &json_fixture.root,
                    &json_fixture.request,
                    &json_fixture.runs_root,
                    &RouteFailureClock,
                    T1ScreenFormat::Json,
                    &mut json,
                )
                .unwrap();
                let json_saved = json_fixture.assert_saved();
                let report: T1ScreenReport = serde_json::from_slice(&json).unwrap();
                assert_eq!(report.run_id, json_fixture.request.run_id);
                assert_eq!(report.status, T1ScreenRunStatus::Running);
                assert_eq!(report.campaign_status, T1ScreenCampaignStatus::Open);
                assert_eq!(
                    report.campaign_active_run_id.as_ref(),
                    Some(&json_fixture.request.run_id)
                );
                assert_eq!(report.route_failures, json_saved.route_failures);
                assert_eq!(report.models[0].outcome, json_saved.models[0].outcome);
                assert_eq!(report.models[0].attempts.len(), 1);
                assert_eq!(
                    report.models[0].attempts[0].child_run_id,
                    json_fixture.request.child_run_id
                );
                assert_eq!(
                    report.models[0].attempts[0].status,
                    T1ScreenChildStatus::Failed
                );
                assert!(report.models[1].outcome.is_none());
                assert!(report.models[1].attempts.is_empty());
                assert!(report.active_child_run_id.is_none());

                let mut scored_report = report.clone();
                scored_report.child_runs[0].status = T1ScreenChildStatus::Completed;
                scored_report.models[0].attempts[0].status = T1ScreenChildStatus::Completed;
                scored_report.models[0].attempts[0].evidence = Some(matrix_evidence(
                    scored_report.models[0].attempts[0].model.clone(),
                    scored_report.judge.clone(),
                    true,
                ));
                scored_report.child_runs[1].status = T1ScreenChildStatus::Completed;
                let second_model = scored_report.child_runs[1].model.clone();
                scored_report.models[1].attempts.push(T1ScreenAttemptReport {
                    child_run_id: scored_report.child_runs[1].run_id.clone(),
                    model: second_model.clone(),
                    status: T1ScreenChildStatus::Completed,
                    evidence: Some(matrix_evidence(
                        second_model,
                        scored_report.judge.clone(),
                        false,
                    )),
                    cases: Vec::new(),
                });
                let mut scored_text = Vec::new();
                render_t1_screen_report(
                    &scored_report,
                    T1ScreenFormat::Text,
                    &mut scored_text,
                )
                .unwrap();
                let scored_text = String::from_utf8(scored_text).unwrap();
                assert!(scored_text.contains("| fixture/alpha | P |  |  |  |  |  |  |"));
                assert!(scored_text.contains("| fixture/beta |  |  |  |  | F |  |  |"));

                let mut scored_json = Vec::new();
                render_t1_screen_report(
                    &scored_report,
                    T1ScreenFormat::Json,
                    &mut scored_json,
                )
                .unwrap();
                assert_eq!(
                    serde_json::from_slice::<T1ScreenReport>(&scored_json).unwrap(),
                    scored_report
                );

                struct BrokenOutput;

                impl std::io::Write for BrokenOutput {
                    fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
                        Err(std::io::Error::other("injected output failure"))
                    }

                    fn flush(&mut self) -> std::io::Result<()> {
                        Ok(())
                    }
                }

                let failure_fixture = RouteFailureCliFixture::new("output-failure");
                assert!(
                    run_t1_screen_fail_route_at(
                        &failure_fixture.root,
                        &failure_fixture.request,
                        &failure_fixture.runs_root,
                        &RouteFailureClock,
                        T1ScreenFormat::Text,
                        &mut BrokenOutput,
                    )
                    .is_err()
                );
                failure_fixture.assert_saved();
            }

            #[test]
            fn t1_screen_campaign_create_parser_is_strict_and_repeatable() {
                let request = parse_arguments(&arguments(&[
                    "t1-screen-campaign-create",
                    "--campaign",
                    "campaign-1",
                    "--judge-cap-millionths",
                    "20000000",
                    "--reason",
                    "Owner approved one total",
                    "--run",
                    "run-2",
                    "--run",
                    "run-1",
                    "--format",
                    "json",
                ]))
                .unwrap();
                assert!(matches!(
                    request.command,
                    CliCommand::T1ScreenCampaignCreate { request, .. }
                        if request.campaign_id.0 == "campaign-1"
                            && request.judge_cap_millionths_of_dollar == 20_000_000
                            && request.run_ids.len() == 2
                ));

                for case in [
                    vec!["t1-screen-campaign-create"],
                    vec![
                        "t1-screen-campaign-create", "--campaign", "../unsafe",
                        "--judge-cap-millionths", "20000000", "--reason", "approved",
                        "--run", "run-1",
                    ],
                    vec![
                        "t1-screen-campaign-create", "--campaign", "safe",
                        "--campaign", "other", "--judge-cap-millionths", "20000000",
                        "--reason", "approved", "--run", "run-1",
                    ],
                    vec![
                        "t1-screen-campaign-create", "--campaign", "safe",
                        "--judge-cap-millionths", "20000000", "--reason", "approved",
                    ],
                ] {
                    assert!(parse_arguments(&arguments(&case)).is_err(), "{case:?}");
                }
            }

            #[test]
            fn t1_screen_campaign_extension_parser_is_strict() {
                let request = parse_arguments(&arguments(&[
                    "t1-screen-campaign-extend-cap",
                    "--campaign",
                    "campaign-1",
                    "--judge-cap-millionths",
                    "66038087",
                    "--reason",
                    "Owner approved the aggregate campaign total",
                    "--format",
                    "json",
                ]))
                .unwrap();
                assert!(matches!(
                    request.command,
                    CliCommand::T1ScreenCampaignExtendCap { request, format }
                        if request.campaign_id.0 == "campaign-1"
                            && request.new_approved_total_millionths_of_dollar == 66_038_087
                            && request.owner_reason == "Owner approved the aggregate campaign total"
                            && format == T1ScreenFormat::Json
                ));

                for case in [
                    vec!["t1-screen-campaign-extend-cap"],
                    vec![
                        "t1-screen-campaign-extend-cap", "--campaign", "../unsafe",
                        "--judge-cap-millionths", "66038087", "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-campaign-extend-cap", "--campaign", "safe", "--campaign",
                        "other", "--judge-cap-millionths", "66038087", "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-campaign-extend-cap", "--campaign", "safe",
                        "--judge-cap-millionths", "0", "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-campaign-extend-cap", "--campaign", "safe",
                        "--judge-cap-millionths", "66038087", "--reason", "   ",
                    ],
                    vec![
                        "t1-screen-campaign-extend-cap", "--campaign", "safe",
                        "--judge-cap-millionths", "18446744073709551616", "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-campaign-extend-cap", "--campaign", "safe",
                        "--judge-cap-millionths", "66038087", "--judge-cap-millionths",
                        "70000000", "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-campaign-extend-cap", "--campaign", "safe",
                        "--judge-cap-millionths", "66038087", "--reason", "approved",
                        "--reason", "again",
                    ],
                    vec![
                        "t1-screen-campaign-extend-cap", "--campaign", "safe",
                        "--judge-cap-millionths", "66038087", "--reason", "approved",
                        "--format", "jsonl",
                    ],
                    vec![
                        "t1-screen-campaign-extend-cap", "--campaign", "safe",
                        "--judge-cap-millionths", "66038087", "--reason", "approved",
                        "--unknown",
                    ],
                ] {
                    assert!(parse_arguments(&arguments(&case)).is_err(), "{case:?}");
                }
            }

            #[test]
            fn t1_screen_campaign_retirement_parser_is_strict() {
                let parsed = parse_arguments(&arguments(&[
                    "t1-screen-campaign-retire-run",
                    "--campaign",
                    "campaign-1",
                    "--run",
                    "paused-run",
                    "--reason",
                    "Owner retired the paused run",
                    "--format",
                    "json",
                ]))
                .unwrap();
                assert!(matches!(
                    parsed.command,
                    CliCommand::T1ScreenCampaignRetireRun { request, format }
                        if request.campaign_id.0 == "campaign-1"
                            && request.run_id.0 == "paused-run"
                            && request.owner_reason == "Owner retired the paused run"
                            && format == T1ScreenFormat::Json
                ));

                for case in [
                    vec!["t1-screen-campaign-retire-run"],
                    vec![
                        "t1-screen-campaign-retire-run", "--campaign", "../unsafe", "--run",
                        "paused-run", "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-campaign-retire-run", "--campaign", "campaign", "--run",
                        "../unsafe", "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-campaign-retire-run", "--campaign", "campaign", "--campaign",
                        "other", "--run", "paused-run", "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-campaign-retire-run", "--campaign", "campaign", "--run",
                        "paused-run", "--run", "other", "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-campaign-retire-run", "--campaign", "campaign", "--run",
                        "paused-run", "--reason", " ",
                    ],
                    vec![
                        "t1-screen-campaign-retire-run", "--campaign", "campaign", "--run",
                        "paused-run", "--reason", "approved", "--reason", "again",
                    ],
                    vec![
                        "t1-screen-campaign-retire-run", "--campaign", "campaign", "--run",
                        "paused-run", "--reason", "approved", "--format", "jsonl",
                    ],
                    vec![
                        "t1-screen-campaign-retire-run", "--campaign", "campaign", "--run",
                        "paused-run", "--reason", "approved", "--runs-root", "elsewhere",
                    ],
                    vec![
                        "t1-screen-campaign-retire-run", "--campaign", "campaign", "--run",
                        "paused-run", "--reason", "approved", "--unknown",
                    ],
                ] {
                    assert!(parse_arguments(&arguments(&case)).is_err(), "{case:?}");
                }
            }

            #[cfg(unix)]
            #[test]
            fn t1_screen_campaign_retirement_saves_before_text_and_json_output() {
                let (text_root, text_request) = retirement_cli_fixture("text");
                let mut text = Vec::new();
                run_t1_screen_campaign_retire_run_at(
                    &text_root,
                    &text_request,
                    Timestamp("2026-08-26T04:00:00-0400".to_owned()),
                    T1ScreenFormat::Text,
                    &mut text,
                )
                .unwrap();
                assert_eq!(
                    String::from_utf8(text).unwrap(),
                    "retired T1 screen campaign run paused-run; total 20000000, spent 7, remaining 19999993 millionths\n"
                );
                let text_saved = FileT1ScreenCampaignStore::open(&text_root)
                    .unwrap()
                    .load(&text_request.campaign_id)
                    .unwrap();
                assert_eq!(text_saved.retirements.len(), 1);
                assert!(text_saved.active_run_id.is_none());

                let (json_root, json_request) = retirement_cli_fixture("json");
                let mut json = Vec::new();
                run_t1_screen_campaign_retire_run_at(
                    &json_root,
                    &json_request,
                    Timestamp("2026-08-26T04:00:00-0400".to_owned()),
                    T1ScreenFormat::Json,
                    &mut json,
                )
                .unwrap();
                let rendered: T1ScreenCampaignState = serde_json::from_slice(&json).unwrap();
                let json_saved = FileT1ScreenCampaignStore::open(&json_root)
                    .unwrap()
                    .load(&json_request.campaign_id)
                    .unwrap();
                assert_eq!(rendered, json_saved);

                struct BrokenOutput;
                impl std::io::Write for BrokenOutput {
                    fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
                        Err(std::io::Error::other("injected output failure"))
                    }
                    fn flush(&mut self) -> std::io::Result<()> {
                        Ok(())
                    }
                }
                let (failure_root, failure_request) = retirement_cli_fixture("output-failure");
                assert!(run_t1_screen_campaign_retire_run_at(
                    &failure_root,
                    &failure_request,
                    Timestamp("2026-08-26T04:00:00-0400".to_owned()),
                    T1ScreenFormat::Text,
                    &mut BrokenOutput,
                )
                .is_err());
                assert_eq!(
                    FileT1ScreenCampaignStore::open(&failure_root)
                        .unwrap()
                        .load(&failure_request.campaign_id)
                        .unwrap()
                        .retirements
                        .len(),
                    1
                );

                for root in [text_root, json_root, failure_root] {
                    std::fs::remove_dir_all(root).unwrap();
                }
            }

            #[test]
            fn t1_screen_cap_extension_parser_is_strict() {
                let request = parse_arguments(&arguments(&[
                    "t1-screen-extend-cap",
                    "--run",
                    "t1-screen-safe_1",
                    "--judge-cap-millionths",
                    "20000000",
                    "--provider-cap-millionths",
                    "20000000",
                    "--reason",
                    "Owner approved the remaining judge work",
                    "--format",
                    "json",
                ]))
                .unwrap();
                assert!(matches!(
                    request.command,
                    CliCommand::T1ScreenExtendCap { request, .. }
                        if request.new_owner_cap_millionths_of_dollar == 20_000_000
                            && request.new_provider_cap_millionths_of_dollar == 20_000_000
                            && request.owner_reason == "Owner approved the remaining judge work"
                ));

                for case in [
                    vec!["t1-screen-extend-cap"],
                    vec![
                        "t1-screen-extend-cap", "--run", "../unsafe", "--judge-cap-millionths",
                        "200", "--provider-cap-millionths", "200", "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-extend-cap", "--run", "safe", "--run", "other",
                        "--judge-cap-millionths", "200", "--provider-cap-millionths", "200",
                        "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-extend-cap", "--run", "safe", "--judge-cap-millionths", "0",
                        "--provider-cap-millionths", "1", "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-extend-cap", "--run", "safe", "--judge-cap-millionths", "200",
                        "--provider-cap-millionths", "201", "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-extend-cap", "--run", "safe", "--judge-cap-millionths", "200",
                        "--provider-cap-millionths", "200", "--reason", "   ",
                    ],
                    vec![
                        "t1-screen-extend-cap", "--run", "safe", "--judge-cap-millionths",
                        "18446744073709551616", "--provider-cap-millionths", "200", "--reason",
                        "approved",
                    ],
                    vec![
                        "t1-screen-extend-cap", "--run", "safe", "--judge-cap-millionths", "200",
                        "--judge-cap-millionths", "201", "--provider-cap-millionths", "200",
                        "--reason", "approved",
                    ],
                    vec![
                        "t1-screen-extend-cap", "--run", "safe", "--judge-cap-millionths", "200",
                        "--provider-cap-millionths", "200", "--reason", "approved", "--reason",
                        "again",
                    ],
                ] {
                    assert!(parse_arguments(&arguments(&case)).is_err(), "{case:?}");
                }
            }

            #[cfg(unix)]
            #[test]
            fn t1_screen_campaign_extension_executor_saves_before_text_and_json_output() {
                let sequence = CANDIDATE_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let root = std::env::temp_dir().join(format!(
                    "skill-eval-campaign-extension-cli-{}-{sequence}",
                    std::process::id()
                ));
                std::fs::create_dir(&root).unwrap();
                let campaign_id = T1ScreenCampaignId("campaign".to_owned());
                let mut store = FileT1ScreenCampaignStore::new(&root).unwrap();
                let mut state = T1ScreenCampaignState {
                    campaign_id: campaign_id.clone(),
                    created_at: Timestamp("2026-08-26T03:00:00-0400".to_owned()),
                    approved_judge_total_millionths_of_dollar: 20_000_000,
                    cap_extensions: Vec::new(),
                    retirements: Vec::new(),
                    aggregate_judge_spent_millionths_of_dollar: 0,
                    runs: Vec::new(),
                    active_run_id: None,
                    owner_reason: "Initial owner approval".to_owned(),
                    status: T1ScreenCampaignStatus::Paused,
                };
                store.create(&state).unwrap();
                let request = T1ScreenCampaignCapExtensionRequest {
                    campaign_id: campaign_id.clone(),
                    new_approved_total_millionths_of_dollar: 66_038_087,
                    owner_reason: "Owner approved the aggregate campaign total".to_owned(),
                };
                let mut text = Vec::new();

                run_t1_screen_campaign_extend_cap_at(
                    &root,
                    &request,
                    Timestamp("2026-08-26T04:00:00-0400".to_owned()),
                    T1ScreenFormat::Text,
                    &mut text,
                )
                .unwrap();

                assert_eq!(
                    String::from_utf8(text).unwrap(),
                    "T1 screen campaign campaign: Open\njudge budget: total 66038087, spent 0, remaining 66038087 millionths\ncap extensions: 1\n"
                );
                state = store.load(&campaign_id).unwrap();
                assert_eq!(state.approved_judge_total_millionths_of_dollar, 66_038_087);
                state.status = T1ScreenCampaignStatus::Paused;
                store.save(&state).unwrap();
                let second = T1ScreenCampaignCapExtensionRequest {
                    campaign_id: campaign_id.clone(),
                    new_approved_total_millionths_of_dollar: 70_000_000,
                    owner_reason: "Owner approved a second aggregate total".to_owned(),
                };
                let mut json = Vec::new();
                run_t1_screen_campaign_extend_cap_at(
                    &root,
                    &second,
                    Timestamp("2026-08-26T05:00:00-0400".to_owned()),
                    T1ScreenFormat::Json,
                    &mut json,
                )
                .unwrap();
                let rendered: T1ScreenCampaignState = serde_json::from_slice(&json).unwrap();
                assert_eq!(rendered, store.load(&campaign_id).unwrap());
                assert_eq!(rendered.cap_extensions.len(), 2);

                let mut direct = Vec::new();
                render_t1_screen_campaign_cap_extension(
                    &rendered,
                    T1ScreenFormat::Json,
                    &mut direct,
                )
                .unwrap();
                assert_eq!(direct, json);
                std::fs::remove_dir_all(root).unwrap();
            }

            #[test]
            fn qualify_preserves_planned_flags() {
                let request = parse_arguments(&arguments(&[
                    "qualify",
                    "--skill",
                    "skills/create-pr",
                    "--dry-run",
                    "--start-tier",
                    "T2",
                    "--reference-tier",
                    "T4",
                    "--run-id-file",
                    ".map/run-id",
                    "--format",
                    "jsonl",
                ]))
                .unwrap();

                assert_eq!(request.output_format, OutputFormat::JsonLines);
                let CliCommand::Qualify { request } = request.command else {
                    panic!("expected qualify request");
                };
                assert!(request.is_dry_run);
                assert_eq!(request.artifact_roots.len(), 1);
                assert_eq!(request.policy.purpose, QualificationPurpose::Artifact);
                assert_eq!(request.policy.reference_tier, $crate::model::Tier::T4);
                assert_eq!(request.policy.candidate_tiers[0], $crate::model::Tier::T2);
            }

            #[test]
            fn pool_qualify_defaults_to_all_tiers() {
                let request = parse_arguments(&arguments(&[
                    "pool-qualify",
                    "--plan",
                    ".map/AGNT-0032/model-pool-plan.json",
                    "--artifact",
                    "tools/skill-eval/tests/fixtures/model-calibration",
                    "--dry-run",
                ]))
                .unwrap();

                let CliCommand::PoolQualify { request } = request.command else {
                    panic!("expected pool qualification request");
                };
                assert!(request.is_dry_run);
                assert_eq!(
                    request.selected_tiers,
                    vec![
                        $crate::model::Tier::T1,
                        $crate::model::Tier::T2,
                        $crate::model::Tier::T3,
                        $crate::model::Tier::T4,
                        $crate::model::Tier::T5,
                    ]
                );
            }

            #[test]
            fn pool_qualify_preserves_repeatable_tiers() {
                let request = parse_arguments(&arguments(&[
                    "pool-qualify",
                    "--plan",
                    ".map/AGNT-0032/model-pool-plan.json",
                    "--artifact",
                    "tools/skill-eval/tests/fixtures/model-calibration",
                    "--tiers",
                    "T2",
                    "--tiers",
                    "T4",
                    "--format",
                    "jsonl",
                ]))
                .unwrap();

                assert_eq!(request.output_format, OutputFormat::JsonLines);
                let CliCommand::PoolQualify { request } = request.command else {
                    panic!("expected pool qualification request");
                };
                assert_eq!(
                    request.selected_tiers,
                    vec![$crate::model::Tier::T2, $crate::model::Tier::T4]
                );
            }

            #[test]
            fn pool_report_and_resume_require_one_safe_identifier() {
                for command in ["pool-report", "pool-resume"] {
                    let request = parse_arguments(&arguments(&[command, "--run", "pool-1"]))
                        .expect("safe pool identifier must parse");
                    assert!(matches!(
                        request.command,
                        CliCommand::PoolReport { .. } | CliCommand::PoolResume { .. }
                    ));
                    for value in ["", "../pool", "pool/name", "pool name", "pool:1"] {
                        assert!(
                            parse_arguments(&arguments(&[command, "--run", value])).is_err(),
                            "{command} accepted {value:?}"
                        );
                    }
                }
            }

            #[test]
            fn pool_replacement_requires_parent_and_entrant_index() {
                let request = parse_arguments(&arguments(&[
                    "pool-replacement",
                    "--run",
                    "pool-1",
                    "--entrant-index",
                    "2",
                ]))
                .unwrap();

                assert!(matches!(
                    request.command,
                    CliCommand::PoolReplacement {
                        entrant_index: 2,
                        ..
                    }
                ));
                assert!(
                    parse_arguments(&arguments(&["pool-replacement", "--run", "pool-1",])).is_err()
                );
            }

            #[test]
            fn pool_parser_rejects_missing_duplicate_unsafe_and_unknown_inputs() {
                let cases = [
                    vec!["pool-qualify", "--artifact", "tools/exam"],
                    vec!["pool-qualify", "--plan", "plan.json"],
                    vec![
                        "pool-qualify",
                        "--plan",
                        "../plan.json",
                        "--artifact",
                        "tools/exam",
                    ],
                    vec!["pool-qualify", "--plan", "plan.json", "--artifact", ""],
                    vec![
                        "pool-qualify",
                        "--plan",
                        "plan.json",
                        "--artifact",
                        "tools/exam",
                        "--artifact",
                        "tools/exam",
                    ],
                    vec![
                        "pool-qualify",
                        "--plan",
                        "plan.json",
                        "--artifact",
                        "tools/exam",
                        "--tiers",
                        "T2",
                        "--tiers",
                        "T2",
                    ],
                    vec![
                        "pool-qualify",
                        "--plan",
                        "plan.json",
                        "--artifact",
                        "tools/exam",
                        "--unknown",
                    ],
                    vec![
                        "pool-qualify",
                        "--plan",
                        "one.json",
                        "--plan",
                        "two.json",
                        "--artifact",
                        "tools/exam",
                    ],
                    vec![
                        "pool-qualify",
                        "--plan",
                        "plan.json",
                        "--artifact",
                        "tools/exam",
                        "--dry-run",
                        "--dry-run",
                    ],
                ];

                for case in cases {
                    assert!(parse_arguments(&arguments(&case)).is_err(), "{case:?}");
                }
            }

            #[test]
            fn every_command_has_a_complete_request_shape() {
                let cases = [
                    vec!["model-capabilities", "--output", "models.json"],
                    vec!["report", "--run", "run-1"],
                    vec![
                        "inspect",
                        "--run",
                        "run-1",
                        "--skill",
                        "skill",
                        "--tier",
                        "T2",
                        "--route-index",
                        "0",
                        "--case",
                        "c1",
                        "--trial",
                        "2",
                    ],
                    vec!["resume", "--run", "run-1"],
                    vec![
                        "decide",
                        "--run",
                        "run-1",
                        "--artifact",
                        "skill",
                        "--accept",
                        "--assign",
                        "skill_minimum=T2",
                    ],
                    vec!["apply", "--run", "run-1", "--artifact", "skill"],
                    vec![
                        "audit-briefs",
                        "--skill",
                        "skills/create-pr",
                        "--out",
                        ".map/audits",
                    ],
                    vec!["judge", "--prompt", "grade this", "--timeout", "30"],
                    vec![
                        "pool-qualify",
                        "--plan",
                        "plan.json",
                        "--artifact",
                        "tools/exam",
                    ],
                    vec!["pool-report", "--run", "pool-1"],
                    vec!["pool-resume", "--run", "pool-1"],
                    vec![
                        "pool-replacement",
                        "--run",
                        "pool-1",
                        "--entrant-index",
                        "2",
                    ],
                ];

                for case in cases {
                    parse_arguments(&arguments(&case)).unwrap();
                }
            }

            #[test]
            fn decide_never_infers_an_owner_choice() {
                let result = parse_arguments(&arguments(&[
                    "decide",
                    "--run",
                    "run-1",
                    "--artifact",
                    "create-pr",
                    "--assign",
                    "skill_minimum=T2",
                ]));

                assert!(result.is_err());
            }

            #[test]
            fn reject_requires_a_reason() {
                let result = parse_arguments(&arguments(&[
                    "decide",
                    "--run",
                    "run-1",
                    "--artifact",
                    "create-pr",
                    "--reject",
                ]));

                assert!(result.is_err());
            }

            #[test]
            fn duplicate_run_flags_are_rejected() {
                let cases = [
                    vec!["report", "--run", "run-1", "--run", "run-2"],
                    vec!["resume", "--run", "run-1", "--run", "run-2"],
                    vec![
                        "apply",
                        "--run",
                        "run-1",
                        "--run",
                        "run-2",
                        "--artifact",
                        "skill",
                    ],
                ];

                for case in cases {
                    assert!(parse_arguments(&arguments(&case)).is_err(), "{case:?}");
                }
            }

            #[test]
            fn duplicate_format_flags_are_rejected() {
                let cases = [
                    vec![
                        "report", "--format", "text", "--run", "run-1", "--format", "jsonl",
                    ],
                    vec![
                        "judge", "--prompt", "grade", "--format", "jsonl", "--format", "text",
                    ],
                ];

                for case in cases {
                    assert!(parse_arguments(&arguments(&case)).is_err(), "{case:?}");
                }
            }

            #[test]
            fn duplicate_dry_run_flags_are_rejected() {
                let cases = [
                    vec![
                        "qualify",
                        "--skill",
                        "skills/create-pr",
                        "--dry-run",
                        "--dry-run",
                    ],
                    vec![
                        "qualify",
                        "--dry-run",
                        "--skill",
                        "skills/create-pr",
                        "--dry-run",
                    ],
                ];

                for case in cases {
                    assert!(parse_arguments(&arguments(&case)).is_err(), "{case:?}");
                }
            }

            #[test]
            fn duplicate_singleton_value_flags_are_rejected() {
                let cases = [
                    vec![
                        "inspect",
                        "--run",
                        "run-1",
                        "--artifact",
                        "one",
                        "--skill",
                        "two",
                        "--tier",
                        "T2",
                        "--case",
                        "c1",
                        "--trial",
                        "1",
                    ],
                    vec![
                        "inspect",
                        "--run",
                        "run-1",
                        "--artifact",
                        "one",
                        "--tier",
                        "T2",
                        "--tier",
                        "T3",
                        "--case",
                        "c1",
                        "--trial",
                        "1",
                    ],
                    vec![
                        "inspect",
                        "--run",
                        "run-1",
                        "--artifact",
                        "one",
                        "--tier",
                        "T2",
                        "--case",
                        "c1",
                        "--case",
                        "c2",
                        "--trial",
                        "1",
                    ],
                    vec![
                        "inspect",
                        "--run",
                        "run-1",
                        "--artifact",
                        "one",
                        "--tier",
                        "T2",
                        "--case",
                        "c1",
                        "--trial",
                        "1",
                        "--attempt",
                        "2",
                    ],
                    vec![
                        "decide",
                        "--run",
                        "run-1",
                        "--artifact",
                        "one",
                        "--reject",
                        "--reason",
                        "first",
                        "--reason",
                        "second",
                    ],
                    vec![
                        "audit-briefs",
                        "--skill",
                        "skills/create-pr",
                        "--out",
                        ".map/one",
                        "--out",
                        ".map/two",
                    ],
                    vec!["judge", "--prompt", "one", "--prompt", "two"],
                    vec!["judge", "--prompt", "one", "--prompt-file", "prompt.txt"],
                    vec![
                        "judge",
                        "--prompt",
                        "one",
                        "--timeout",
                        "10",
                        "--timeout",
                        "20",
                    ],
                    vec![
                        "qualify",
                        "--skill",
                        "skills/create-pr",
                        "--start-tier",
                        "T1",
                        "--start-tier",
                        "T2",
                    ],
                    vec![
                        "qualify",
                        "--skill",
                        "skills/create-pr",
                        "--reference-tier",
                        "T3",
                        "--reference-tier",
                        "T4",
                    ],
                    vec![
                        "qualify",
                        "--skill",
                        "skills/create-pr",
                        "--run-id-file",
                        ".map/one",
                        "--run-id-file",
                        ".map/two",
                    ],
                    vec![
                        "qualify",
                        "--skill",
                        "skills/create-pr",
                        "--trials",
                        "1",
                        "--trials",
                        "2",
                    ],
                    vec![
                        "qualify",
                        "--skill",
                        "skills/create-pr",
                        "--minimum-score",
                        "7",
                        "--minimum-score",
                        "8",
                    ],
                    vec![
                        "qualify",
                        "--skill",
                        "skills/create-pr",
                        "--noninferiority-margin",
                        "0.5",
                        "--noninferiority-margin",
                        "1.0",
                    ],
                    vec![
                        "qualify",
                        "--skill",
                        "skills/create-pr",
                        "--confidence",
                        "0.9",
                        "--confidence",
                        "0.95",
                    ],
                    vec![
                        "qualify",
                        "--skill",
                        "skills/create-pr",
                        "--change-artifact",
                        "one",
                        "--change-artifact",
                        "two",
                    ],
                    vec![
                        "qualify",
                        "--skill",
                        "skills/create-pr",
                        "--incumbent-revision",
                        "one",
                        "--incumbent-revision",
                        "two",
                    ],
                    vec![
                        "report",
                        "--run",
                        "run-1",
                        "--runs-root",
                        ".map/one",
                        "--runs-root",
                        ".map/two",
                    ],
                ];

                for case in cases {
                    assert!(parse_arguments(&arguments(&case)).is_err(), "{case:?}");
                }
            }

            #[test]
            fn repeatable_flags_keep_distinct_values() {
                parse_arguments(&arguments(&[
                    "qualify",
                    "--skill",
                    "skills/create-pr",
                    "--artifact",
                    "skills/rust-style",
                    "--dry-run",
                ]))
                .unwrap();
                parse_arguments(&arguments(&[
                    "decide",
                    "--run",
                    "run-1",
                    "--artifact",
                    "create-pr",
                    "--accept",
                    "--assign",
                    "skill_minimum=T2",
                    "--assign",
                    "skill_target=T3",
                ]))
                .unwrap();
            }

            #[test]
            fn jsonl_alias_is_not_accepted() {
                let result = parse_arguments(&arguments(&["report", "--run", "run-1", "--jsonl"]));

                assert!(result.is_err());
            }

            #[test]
            fn structured_event_is_one_complete_line() {
                let event = RunEvent::RunStarted {
                    at: Timestamp("2026-01-01T00:00:00+0000".to_owned()),
                    configuration: RunConfiguration {
                        run_id: RunId("run-1".to_owned()),
                        mode: RunMode::DryRun,
                        artifacts: Vec::new(),
                        change: None,
                        policy: $crate::model::QualificationPolicy {
                            purpose: QualificationPurpose::Artifact,
                            candidate_tiers: vec![$crate::model::Tier::T2],
                            reference_tier: $crate::model::Tier::T4,
                            judge_tier: $crate::model::Tier::T5,
                            repeats_per_case: 3,
                            minimum_score: 8,
                            noninferiority_margin: 1.0,
                            confidence_level: 0.95,
                        },
                        qualification_routes: Default::default(),
                        created_at: Timestamp("2026-01-01T00:00:00+0000".to_owned()),
                    },
                };
                let mut output = Vec::new();

                render_event(&event, OutputFormat::JsonLines, &mut output).unwrap();

                assert_eq!(output.iter().filter(|byte| **byte == b'\n').count(), 1);
                let json = serde_json::from_slice::<serde_json::Value>(&output).unwrap();
                assert_eq!(json["configuration"]["policy"]["purpose"], "artifact");
            }

            #[test]
            fn concrete_run_ids_are_unique_safe_components() {
                let root = std::env::temp_dir().join(format!(
                    "skill-eval-cli-run-ids-{}-{:?}",
                    std::process::id(),
                    std::thread::current().id()
                ));
                let mut source = PathRunIdSource::new(&root).unwrap();

                let first = source.next().unwrap();
                let second = source.next().unwrap();

                assert_ne!(first, second);
                for run_id in [first, second] {
                    assert_eq!(Path::new(&run_id.0).components().count(), 1);
                    assert!(!run_id.0.contains(['/', '\\']));
                }
                std::fs::remove_dir_all(root).unwrap();
            }

            #[test]
            fn unsafe_run_id_fails_closed() {
                let result = parse_arguments(&arguments(&["report", "--run", "../run"]));
                assert!(result.is_err());
            }
        }
    };
}
