#[path = "../src/model.rs"]
mod model;
#[path = "../src/pool_store.rs"]
mod pool_store;
#[path = "../src/ports.rs"]
mod ports;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use model::{
    ModelIdentity, PoolChildRun, PoolChildStatus, PoolEntrant, PoolPauseReason, PoolPolicy,
    PoolRunConfiguration, PoolRunId, PoolRunState, PoolRunStatus, PoolStage, RankedPool, RunId,
    SkillEvalError, Tier, Timestamp,
};
use pool_store::{FailurePoint, FilePoolStore};
use ports::PoolStore;

static TEMPORARY_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let sequence = TEMPORARY_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skill-eval-pool-store-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("temporary directory should be created");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn snapshot(&self) -> PathBuf {
        self.path.join("pools/pool-1/state.json")
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).expect("temporary directory should be removed");
    }
}

fn identity(tier: Tier, index: usize) -> ModelIdentity {
    ModelIdentity {
        tier,
        provider: "provider".to_owned(),
        model: format!("model-{index}"),
        thinking: "medium".to_owned(),
    }
}

// TODO(AGNT-0032.T88): Prove store preservation and rejection for frozen exam definitions.
fn initial_state() -> PoolRunState {
    let tier = Tier::T2;
    let entrants = (0..3)
        .map(|index| PoolEntrant {
            model: identity(tier, index),
            catalog_observed_at: Timestamp("2026-08-23T12:00:00-0400".to_owned()),
        })
        .collect::<Vec<_>>();
    let child_runs = (0_u8..3)
        .flat_map(|entrant_index| {
            [PoolStage::Calibration, PoolStage::Qualification]
                .into_iter()
                .map(move |stage| PoolChildRun {
                    tier,
                    entrant_index,
                    stage,
                    run_id: RunId(format!(
                        "child-{entrant_index}-{}",
                        match stage {
                            PoolStage::Calibration => "calibration",
                            PoolStage::Qualification => "qualification",
                        }
                    )),
                    status: PoolChildStatus::Pending,
                })
        })
        .collect();
    PoolRunState {
        configuration: PoolRunConfiguration {
            run_id: PoolRunId("pool-1".to_owned()),
            created_at: Timestamp("2026-08-23T12:00:00-0400".to_owned()),
            entrants: BTreeMap::from([(tier, entrants)]),
            control: ModelIdentity {
                tier: Tier::T1,
                provider: "openrouter".to_owned(),
                model: "openrouter/free".to_owned(),
                thinking: "low".to_owned(),
            },
            policy: PoolPolicy {
                calibration_repeats_per_case: 1,
                qualification_repeats_per_case: 3,
                promotion_count: 2,
                minimum_score: 8,
                minimum_reliability_basis_points: 9_500,
                maximum_catalog_age_seconds: 7_200,
                spending_limit_millionths_of_dollar: 10_000_000,
                is_provider_limit_enforced: true,
            },
        },
        selected_tiers: vec![tier],
        status: PoolRunStatus::Pending,
        child_runs,
        pools: Vec::new(),
        pause: None,
        spent_millionths_of_dollar: 0,
    }
}

fn invalid_message(error: SkillEvalError) -> String {
    match error {
        SkillEvalError::InvalidConfiguration(message) => message,
        other => panic!("expected invalid configuration, got {other:?}"),
    }
}

#[test]
fn every_child_identity_survives_running_paused_and_completed_snapshots() {
    let directory = TemporaryDirectory::new("resume");
    let mut store = FilePoolStore::new(directory.path()).unwrap();
    let mut state = initial_state();
    let child_identities = state
        .child_runs
        .iter()
        .map(|child| child.run_id.clone())
        .collect::<Vec<_>>();

    store.create_pool(&state).unwrap();
    assert_eq!(store.load_pool(&state.configuration.run_id).unwrap(), state);

    state.status = PoolRunStatus::Running;
    store.save_pool(&state).unwrap();
    state.child_runs[0].status = PoolChildStatus::Running;
    state.pools.push(RankedPool {
        tier: Tier::T2,
        calibration: Vec::new(),
        promoted: Vec::new(),
        qualification: Vec::new(),
        ranked: Vec::new(),
        is_complete: false,
    });
    state.spent_millionths_of_dollar = 125;
    store.save_pool(&state).unwrap();
    assert_eq!(store.load_pool(&state.configuration.run_id).unwrap(), state);

    state.child_runs[0].status = PoolChildStatus::Paused;
    store.save_pool(&state).unwrap();
    state.status = PoolRunStatus::Paused;
    state.pause = Some(PoolPauseReason::Infrastructure {
        message: "interrupted".to_owned(),
    });
    store.save_pool(&state).unwrap();
    assert_eq!(store.load_pool(&state.configuration.run_id).unwrap(), state);

    state.status = PoolRunStatus::Running;
    state.pause = None;
    store.save_pool(&state).unwrap();
    state.child_runs[0].status = PoolChildStatus::Running;
    store.save_pool(&state).unwrap();
    state.child_runs[0].status = PoolChildStatus::Completed;
    state.spent_millionths_of_dollar = 250;
    store.save_pool(&state).unwrap();

    let reloaded = store.load_pool(&state.configuration.run_id).unwrap();
    assert_eq!(reloaded, state);
    assert_eq!(
        reloaded
            .child_runs
            .iter()
            .map(|child| child.run_id.clone())
            .collect::<Vec<_>>(),
        child_identities
    );
}

#[test]
fn creation_requires_complete_unique_preallocation_and_is_not_repeatable() {
    let directory = TemporaryDirectory::new("create");
    let mut store = FilePoolStore::new(directory.path()).unwrap();
    let state = initial_state();
    store.create_pool(&state).unwrap();
    assert!(invalid_message(store.create_pool(&state).unwrap_err()).contains("already exists"));

    let missing_directory = TemporaryDirectory::new("missing-child");
    let mut missing_store = FilePoolStore::new(missing_directory.path()).unwrap();
    let mut missing = initial_state();
    missing.child_runs.pop();
    assert!(
        invalid_message(missing_store.create_pool(&missing).unwrap_err()).contains("unallocated")
    );

    let duplicate_directory = TemporaryDirectory::new("duplicate-child");
    let mut duplicate_store = FilePoolStore::new(duplicate_directory.path()).unwrap();
    let mut duplicate = initial_state();
    duplicate.child_runs[1].run_id = duplicate.child_runs[0].run_id.clone();
    assert!(
        invalid_message(duplicate_store.create_pool(&duplicate).unwrap_err())
            .contains("duplicate child run")
    );

    let duplicate_slot_directory = TemporaryDirectory::new("duplicate-slot");
    let mut duplicate_slot_store = FilePoolStore::new(duplicate_slot_directory.path()).unwrap();
    let mut duplicate_slot = initial_state();
    duplicate_slot.child_runs[1].stage = PoolStage::Calibration;
    assert!(
        invalid_message(
            duplicate_slot_store
                .create_pool(&duplicate_slot)
                .unwrap_err()
        )
        .contains("duplicate child slots")
    );
}

#[test]
fn unsafe_identifiers_and_escaping_pool_directories_are_rejected() {
    let directory = TemporaryDirectory::new("identifiers");
    let mut store = FilePoolStore::new(directory.path()).unwrap();
    let mut state = initial_state();
    state.configuration.run_id = PoolRunId("../escape".to_owned());
    assert!(
        invalid_message(store.create_pool(&state).unwrap_err()).contains("safe path component")
    );

    let mut child = initial_state();
    child.child_runs[0].run_id = RunId("child/escape".to_owned());
    assert!(
        invalid_message(store.create_pool(&child).unwrap_err()).contains("safe path component")
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let outside = TemporaryDirectory::new("outside");
        symlink(outside.path(), directory.path().join("pools/pool-1")).unwrap();
        let error = store
            .load_pool(&PoolRunId("pool-1".to_owned()))
            .unwrap_err();
        assert!(invalid_message(error).contains("escapes"));
    }
}

#[test]
fn frozen_fields_rollbacks_skips_and_illegal_aggregate_transitions_preserve_bytes() {
    let directory = TemporaryDirectory::new("transitions");
    let mut store = FilePoolStore::new(directory.path()).unwrap();
    let state = initial_state();
    store.create_pool(&state).unwrap();
    let before = fs::read(directory.snapshot()).unwrap();

    let mut changed_configuration = state.clone();
    changed_configuration.configuration.policy.minimum_score = 9;
    assert!(
        invalid_message(store.save_pool(&changed_configuration).unwrap_err())
            .contains("configuration")
    );

    let mut changed_tiers = state.clone();
    changed_tiers.selected_tiers.clear();
    assert!(store.save_pool(&changed_tiers).is_err());

    let mut changed_identity = state.clone();
    changed_identity.child_runs[0].run_id = RunId("replacement".to_owned());
    assert!(
        invalid_message(store.save_pool(&changed_identity).unwrap_err()).contains("identities")
    );

    let mut skipped_child = state.clone();
    skipped_child.child_runs[0].status = PoolChildStatus::Completed;
    assert!(invalid_message(store.save_pool(&skipped_child).unwrap_err()).contains("skipped"));

    let mut skipped_aggregate = state.clone();
    skipped_aggregate.status = PoolRunStatus::Completed;
    assert!(
        invalid_message(store.save_pool(&skipped_aggregate).unwrap_err()).contains("aggregate")
    );

    let mut running = state.clone();
    running.status = PoolRunStatus::Running;
    store.save_pool(&running).unwrap();
    running.spent_millionths_of_dollar = 10;
    store.save_pool(&running).unwrap();
    let durable = fs::read(directory.snapshot()).unwrap();
    let mut decreased = running.clone();
    decreased.spent_millionths_of_dollar = 9;
    assert!(invalid_message(store.save_pool(&decreased).unwrap_err()).contains("cannot decrease"));
    assert_eq!(fs::read(directory.snapshot()).unwrap(), durable);
    assert_ne!(before, durable);
}

#[test]
fn malformed_unknown_and_identity_mismatched_snapshots_are_rejected() {
    let directory = TemporaryDirectory::new("malformed");
    let mut store = FilePoolStore::new(directory.path()).unwrap();
    let state = initial_state();
    store.create_pool(&state).unwrap();

    fs::write(directory.snapshot(), b"{not-json\n").unwrap();
    assert!(
        invalid_message(store.load_pool(&state.configuration.run_id).unwrap_err())
            .contains("malformed")
    );

    fs::write(
        directory.snapshot(),
        serde_json::to_vec(&serde_json::json!({
            "configuration": state.configuration,
            "selected_tiers": state.selected_tiers,
            "status": state.status,
            "child_runs": state.child_runs,
            "pools": state.pools,
            "pause": state.pause,
            "spent_millionths_of_dollar": state.spent_millionths_of_dollar,
            "unknown": true
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(
        invalid_message(
            store
                .load_pool(&PoolRunId("pool-1".to_owned()))
                .unwrap_err()
        )
        .contains("unknown data")
    );

    let mut wrong_identity = initial_state();
    wrong_identity.configuration.run_id = PoolRunId("pool-2".to_owned());
    fs::write(
        directory.snapshot(),
        serde_json::to_vec(&wrong_identity).unwrap(),
    )
    .unwrap();
    assert!(
        invalid_message(
            store
                .load_pool(&PoolRunId("pool-1".to_owned()))
                .unwrap_err()
        )
        .contains("identity")
    );
}

#[test]
fn failed_write_sync_rename_and_directory_sync_keep_prior_snapshot_bytes() {
    for failure in [
        FailurePoint::Write,
        FailurePoint::FileSync,
        FailurePoint::Rename,
        FailurePoint::DirectorySync,
    ] {
        let directory = TemporaryDirectory::new("rollback");
        let mut store = FilePoolStore::new(directory.path()).unwrap();
        let state = initial_state();
        store.create_pool(&state).unwrap();
        let before = fs::read(directory.snapshot()).unwrap();
        let mut replacement = state.clone();
        replacement.spent_millionths_of_dollar = 1;

        let mut failing = FilePoolStore::with_failure(directory.path(), failure).unwrap();
        assert!(matches!(
            failing.save_pool(&replacement),
            Err(SkillEvalError::Io { .. })
        ));
        assert_eq!(
            fs::read(directory.snapshot()).unwrap(),
            before,
            "{failure:?}"
        );
        assert_eq!(store.load_pool(&state.configuration.run_id).unwrap(), state);
    }
}
