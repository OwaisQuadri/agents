#![expect(dead_code, reason = "the test imports private production modules")]
#![expect(
    clippy::large_enum_variant,
    reason = "the test imports frozen production model declarations"
)]

#[path = "../src/frontier_store.rs"]
mod frontier_store;
#[path = "../src/model.rs"]
mod model;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use frontier_store::{FileFrontierStore, FrontierFailurePoint};
use model::{
    ArtifactName, CaseId, Decision, FrontierBaseline, FrontierBaselineLedger, FrontierCaseGroup,
    FrontierCaseReference, FrontierConfidenceMethod, FrontierDecisionRecord, FrontierEntrant,
    FrontierEvidenceIdentity, FrontierInspection, FrontierModelProgress, FrontierPlan,
    FrontierPolicy, FrontierRunConfiguration, FrontierRunId, FrontierRunState, FrontierRunStatus,
    FrontierSuite, FrontierSuiteIdentity, FrontierTierSuite, FrontierTrialSelector,
    HarnessIdentity, ModelIdentity, T1ScreenSnapshotIdentity, Tier, Timestamp, TrialKey,
    TrialRecord, TrialUsage, TrialVerdict,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn create_is_durable_and_rejects_collision_and_escape() {
    let fixture = Fixture::new();
    let mut store = FileFrontierStore::new(&fixture.root).unwrap();
    let state = fixture.state();
    store.create_frontier(&state).unwrap();
    assert_eq!(
        store.load_frontier(&state.configuration.run_id).unwrap(),
        state
    );
    assert!(store.create_frontier(&state).is_err());

    let mut unsafe_state = fixture.state();
    unsafe_state.configuration.run_id = FrontierRunId("../escape".to_owned());
    assert!(store.create_frontier(&unsafe_state).is_err());
}

#[test]
fn save_accepts_legal_progress_and_rejects_regression_and_illegal_status() {
    let fixture = Fixture::new();
    let mut store = FileFrontierStore::new(&fixture.root).unwrap();
    let state = fixture.state();
    store.create_frontier(&state).unwrap();

    let mut running = state.clone();
    running.status = FrontierRunStatus::Running;
    running.spent_millionths_of_dollar = 10;
    store.save_frontier(&running).unwrap();
    store.save_frontier(&running).unwrap();

    let mut regressed = running.clone();
    regressed.spent_millionths_of_dollar = 9;
    assert!(store.save_frontier(&regressed).is_err());

    let mut skipped = running;
    skipped.status = FrontierRunStatus::Accepted;
    assert!(store.save_frontier(&skipped).is_err());
}

#[test]
fn trial_replay_is_idempotent_and_conflict_is_rejected() {
    let fixture = Fixture::new();
    let mut store = FileFrontierStore::new(&fixture.root).unwrap();
    let state = fixture.state();
    store.create_frontier(&state).unwrap();
    let trial = fixture.trial();
    store
        .save_frontier_trial(&state.configuration.run_id, &trial)
        .unwrap();
    store
        .save_frontier_trial(&state.configuration.run_id, &trial)
        .unwrap();

    let mut conflict = trial;
    conflict.verdict.score = 8;
    assert!(
        store
            .save_frontier_trial(&state.configuration.run_id, &conflict)
            .is_err()
    );
}

#[test]
fn inspection_uses_every_exact_selector_field() {
    let fixture = Fixture::new();
    let mut store = FileFrontierStore::new(&fixture.root).unwrap();
    let state = fixture.state();
    store.create_frontier(&state).unwrap();
    let trial = fixture.trial();
    store
        .save_frontier_trial(&state.configuration.run_id, &trial)
        .unwrap();
    let selector = fixture.selector();
    assert!(matches!(
        store.inspect_frontier(&selector).unwrap(),
        FrontierInspection::Trial { .. }
    ));

    let mut wrong = selector;
    wrong.thinking = "high".to_owned();
    assert!(store.inspect_frontier(&wrong).is_err());
    wrong.thinking = "low".to_owned();
    wrong.attempt = 2;
    assert!(store.inspect_frontier(&wrong).is_err());
}

#[test]
fn trial_rejects_out_of_schedule_and_escaping_evidence() {
    let fixture = Fixture::new();
    let mut store = FileFrontierStore::new(&fixture.root).unwrap();
    let state = fixture.state();
    store.create_frontier(&state).unwrap();
    let mut trial = fixture.trial();
    trial.key.attempt = 2;
    assert!(
        store
            .save_frontier_trial(&state.configuration.run_id, &trial)
            .is_err()
    );
    trial.key.attempt = 1;
    trial.artifact_path = PathBuf::from("../escape");
    assert!(
        store
            .save_frontier_trial(&state.configuration.run_id, &trial)
            .is_err()
    );
}

#[test]
fn baseline_loader_rejects_broken_chain_and_malformed_version() {
    let fixture = Fixture::new();
    let store = FileFrontierStore::new(&fixture.root).unwrap();
    let evidence = fixture.evidence("accepted.json", b"accepted\n");
    let first = FrontierBaseline {
        accepted_at: fixture.timestamp(),
        run_id: FrontierRunId("old-run".to_owned()),
        run_evidence: evidence.clone(),
        previous_entry_sha256: None,
        pools: BTreeMap::new(),
        capabilities: Vec::new(),
    };
    let second = FrontierBaseline {
        accepted_at: fixture.timestamp(),
        run_id: FrontierRunId("new-run".to_owned()),
        run_evidence: evidence,
        previous_entry_sha256: Some("0".repeat(64)),
        pools: BTreeMap::new(),
        capabilities: Vec::new(),
    };
    fixture.write_json(
        "baselines.json",
        &FrontierBaselineLedger {
            version: 1,
            baselines: vec![first, second],
        },
    );
    assert!(
        store
            .load_frontier_baselines(Path::new("baselines.json"))
            .is_err()
    );

    fixture.write_json(
        "baselines.json",
        &FrontierBaselineLedger {
            version: 2,
            baselines: Vec::new(),
        },
    );
    assert!(
        store
            .load_frontier_baselines(Path::new("baselines.json"))
            .is_err()
    );
}

#[test]
fn acceptance_rejects_stale_authority_and_rewritten_history() {
    let fixture = Fixture::new();
    let mut store = FileFrontierStore::new(&fixture.root).unwrap();
    let pending = fixture.state();
    store.create_frontier(&pending).unwrap();
    let accepted = fixture.accepted_state(&pending);
    let ledger = fixture.acceptance_ledger(&accepted, Vec::new());
    assert!(
        store
            .accept_frontier_baseline(&accepted, Path::new("baselines.json"), &ledger)
            .is_err()
    );

    let mut running = pending;
    running.status = FrontierRunStatus::Running;
    store.save_frontier(&running).unwrap();
    let mut awaiting = running;
    awaiting.status = FrontierRunStatus::AwaitingDecision;
    store.save_frontier(&awaiting).unwrap();
    let accepted = fixture.accepted_state(&awaiting);
    let mut rewritten = fixture.acceptance_ledger(&accepted, Vec::new());
    rewritten
        .baselines
        .insert(0, rewritten.baselines[0].clone());
    assert!(
        store
            .accept_frontier_baseline(&accepted, Path::new("baselines.json"), &rewritten)
            .is_err()
    );
}

#[test]
fn acceptance_recovers_each_interrupted_transaction_boundary() {
    for failure in [
        FrontierFailurePoint::Journal,
        FrontierFailurePoint::State,
        FrontierFailurePoint::Ledger,
    ] {
        let fixture = Fixture::new();
        let mut store = FileFrontierStore::new(&fixture.root).unwrap();
        let pending = fixture.state();
        store.create_frontier(&pending).unwrap();
        let mut running = pending;
        running.status = FrontierRunStatus::Running;
        store.save_frontier(&running).unwrap();
        let mut awaiting = running;
        awaiting.status = FrontierRunStatus::AwaitingDecision;
        store.save_frontier(&awaiting).unwrap();
        let accepted = fixture.accepted_state(&awaiting);
        let ledger = fixture.acceptance_ledger(&accepted, Vec::new());
        let mut failing = FileFrontierStore::with_failure(&fixture.root, failure).unwrap();
        assert!(
            failing
                .accept_frontier_baseline(&accepted, Path::new("baselines.json"), &ledger)
                .is_err()
        );
        let recovered = FileFrontierStore::new(&fixture.root).unwrap();
        assert_eq!(
            recovered
                .load_frontier(&accepted.configuration.run_id)
                .unwrap(),
            accepted
        );
        assert_eq!(
            recovered
                .load_frontier_baselines(Path::new("baselines.json"))
                .unwrap(),
            ledger
        );
    }
}

#[test]
fn accepted_replay_is_a_no_op_and_conflicting_replay_fails() {
    let fixture = Fixture::new();
    let mut store = FileFrontierStore::new(&fixture.root).unwrap();
    let pending = fixture.state();
    store.create_frontier(&pending).unwrap();
    let mut running = pending;
    running.status = FrontierRunStatus::Running;
    store.save_frontier(&running).unwrap();
    let mut awaiting = running;
    awaiting.status = FrontierRunStatus::AwaitingDecision;
    store.save_frontier(&awaiting).unwrap();
    let accepted = fixture.accepted_state(&awaiting);
    let ledger = fixture.acceptance_ledger(&accepted, Vec::new());
    store
        .accept_frontier_baseline(&accepted, Path::new("baselines.json"), &ledger)
        .unwrap();
    store
        .accept_frontier_baseline(&accepted, Path::new("baselines.json"), &ledger)
        .unwrap();
    let mut conflict = ledger;
    conflict.baselines[0].accepted_at = Timestamp("2026-08-27T00:00:01+0000".to_owned());
    assert!(
        store
            .accept_frontier_baseline(&accepted, Path::new("baselines.json"), &conflict)
            .is_err()
    );
}

struct Fixture {
    root: PathBuf,
    plan_sha256: String,
    suite_sha256: String,
    capabilities_sha256: String,
}

impl Fixture {
    fn new() -> Self {
        let sequence = ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "skill-eval-frontier-store-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("plan.json"), b"frozen plan\n").unwrap();
        fs::write(root.join("capabilities.json"), b"frozen capabilities\n").unwrap();
        let suite = FrontierSuite {
            version: 1,
            tiers: BTreeMap::from([(
                Tier::T1,
                FrontierTierSuite {
                    group_weights_basis_points: BTreeMap::from([(
                        FrontierCaseGroup::Normal,
                        10_000,
                    )]),
                    cases: vec![FrontierCaseReference {
                        artifact_path: PathBuf::from("skills/demo"),
                        artifact_revision: "revision".to_owned(),
                        case: CaseId("case-1".to_owned()),
                        group: FrontierCaseGroup::Normal,
                        is_confirmation: false,
                    }],
                },
            )]),
        };
        let suite_bytes = json_bytes(&suite);
        fs::write(root.join("suite.json"), &suite_bytes).unwrap();
        Self {
            plan_sha256: digest(b"frozen plan\n"),
            suite_sha256: digest(&suite_bytes),
            capabilities_sha256: digest(b"frozen capabilities\n"),
            root,
        }
    }

    fn state(&self) -> FrontierRunState {
        FrontierRunState {
            configuration: FrontierRunConfiguration {
                run_id: FrontierRunId("run-1".to_owned()),
                created_at: self.timestamp(),
                plan_path: PathBuf::from("plan.json"),
                plan_sha256: self.plan_sha256.clone(),
                plan: FrontierPlan {
                    version: 1,
                    suite: FrontierSuiteIdentity {
                        path: PathBuf::from("suite.json"),
                        sha256: self.suite_sha256.clone(),
                        version: 1,
                    },
                    capabilities: T1ScreenSnapshotIdentity {
                        path: PathBuf::from("capabilities.json"),
                        sha256: self.capabilities_sha256.clone(),
                        version: 1,
                        observed_at_unix_seconds: 1,
                        pi_version: "1.0".to_owned(),
                    },
                    entrants: vec![FrontierEntrant {
                        provider: "first-party".to_owned(),
                        model: "alpha".to_owned(),
                        entry_tier: Tier::T1,
                        thinking_levels: vec!["low".to_owned()],
                        catalog_observed_at: self.timestamp(),
                    }],
                    judge: ModelIdentity {
                        tier: Tier::T5,
                        provider: "first-party".to_owned(),
                        model: "judge".to_owned(),
                        thinking: "high".to_owned(),
                    },
                    policy: FrontierPolicy {
                        screening_trials_per_case: 1,
                        confirmation_trials_per_case: 3,
                        maximum_trials_per_case: 5,
                        minimum_trial_score: 8,
                        minimum_weighted_pass_basis_points: 8_000,
                        minimum_lower_bound_basis_points: 7_000,
                        confidence_level_basis_points: 9_500,
                        confidence_method: FrontierConfidenceMethod::StratifiedBootstrap,
                        confidence_resamples: 100,
                        maximum_infrastructure_attempts: 2,
                        maximum_catalog_age_seconds: 86_400,
                        active_pool_size: 5,
                        maximum_trial_cost_millionths_of_dollar: 1,
                        spending_limit_millionths_of_dollar: 1_000,
                        is_provider_limit_enforced: true,
                        is_first_party_only: true,
                    },
                },
            },
            status: FrontierRunStatus::Pending,
            models: vec![FrontierModelProgress {
                provider: "first-party".to_owned(),
                model: "alpha".to_owned(),
                entry_tier: Tier::T1,
                selected_routes: Vec::new(),
                next_tier: None,
                next_thinking_index: None,
                is_exhausted: false,
            }],
            cells: Vec::new(),
            infrastructure_events: Vec::new(),
            pause: None,
            decision: None,
            spent_millionths_of_dollar: 0,
        }
    }

    fn trial(&self) -> TrialRecord {
        let evidence = self.root.join(".map/skill-eval/frontier/run-1/evidence");
        fs::create_dir_all(&evidence).unwrap();
        fs::write(evidence.join("artifact.txt"), b"artifact").unwrap();
        fs::write(evidence.join("transcript.txt"), b"transcript").unwrap();
        TrialRecord {
            key: TrialKey {
                artifact: ArtifactName("demo".to_owned()),
                tier: Tier::T1,
                route_index: 0,
                case: CaseId("case-1".to_owned()),
                attempt: 1,
            },
            model: ModelIdentity {
                tier: Tier::T1,
                provider: "first-party".to_owned(),
                model: "alpha".to_owned(),
                thinking: "low".to_owned(),
            },
            harness: HarnessIdentity {
                runner_version: "1".to_owned(),
                pi_version: "1".to_owned(),
                artifact_revision: "revision".to_owned(),
                tool_policy_digest: "policy".to_owned(),
            },
            artifact_path: PathBuf::from(".map/skill-eval/frontier/run-1/evidence/artifact.txt"),
            transcript_path: PathBuf::from(
                ".map/skill-eval/frontier/run-1/evidence/transcript.txt",
            ),
            candidate_usage: zero_usage(),
            judge_model: ModelIdentity {
                tier: Tier::T5,
                provider: "first-party".to_owned(),
                model: "judge".to_owned(),
                thinking: "high".to_owned(),
            },
            judge_usage: zero_usage(),
            verdict: TrialVerdict {
                score: 9,
                is_catastrophic: false,
                failure_mode: None,
                checks: Vec::new(),
            },
        }
    }

    fn selector(&self) -> FrontierTrialSelector {
        FrontierTrialSelector {
            run_id: FrontierRunId("run-1".to_owned()),
            provider: "first-party".to_owned(),
            model: "alpha".to_owned(),
            tier: Tier::T1,
            thinking: "low".to_owned(),
            artifact: ArtifactName("demo".to_owned()),
            case: CaseId("case-1".to_owned()),
            attempt: 1,
        }
    }

    fn accepted_state(&self, awaiting: &FrontierRunState) -> FrontierRunState {
        let mut accepted = awaiting.clone();
        accepted.status = FrontierRunStatus::Accepted;
        accepted.decision = Some(FrontierDecisionRecord {
            decision: Decision::Accepted,
            reason: "approved".to_owned(),
            decided_at: self.timestamp(),
        });
        accepted
    }

    fn acceptance_ledger(
        &self,
        accepted: &FrontierRunState,
        prefix: Vec<FrontierBaseline>,
    ) -> FrontierBaselineLedger {
        let previous_entry_sha256 = prefix
            .last()
            .map(|entry| digest(&serde_json::to_vec(entry).unwrap()));
        let mut baselines = prefix;
        baselines.push(FrontierBaseline {
            accepted_at: self.timestamp(),
            run_id: accepted.configuration.run_id.clone(),
            run_evidence: FrontierEvidenceIdentity {
                path: PathBuf::from(".map/skill-eval/frontier/run-1/state.json"),
                sha256: digest(&json_bytes(accepted)),
            },
            previous_entry_sha256,
            pools: BTreeMap::new(),
            capabilities: Vec::new(),
        });
        FrontierBaselineLedger {
            version: 1,
            baselines,
        }
    }

    fn evidence(&self, name: &str, bytes: &[u8]) -> FrontierEvidenceIdentity {
        fs::write(self.root.join(name), bytes).unwrap();
        FrontierEvidenceIdentity {
            path: PathBuf::from(name),
            sha256: digest(bytes),
        }
    }

    fn timestamp(&self) -> Timestamp {
        Timestamp("2026-08-27T00:00:00+0000".to_owned())
    }

    fn write_json<T: Serialize>(&self, name: &str, value: &T) {
        fs::write(self.root.join(name), json_bytes(value)).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
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

fn json_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap();
    bytes.push(b'\n');
    bytes
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
