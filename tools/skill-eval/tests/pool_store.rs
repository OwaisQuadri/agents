#[path = "../src/model.rs"]
mod model;
#[path = "../src/pool_store.rs"]
mod pool_store;
#[path = "../src/ports.rs"]
mod ports;
#[path = "../src/statistics.rs"]
mod statistics;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use model::{
    ArtifactDefinition, ArtifactKind, ArtifactName, CaseDefinition, CaseDrive, CaseId,
    ConfidenceInterval, ExecutionDefinition, HarnessIdentity, ModelIdentity, PoolChildRun,
    PoolChildStatus, PoolEntrant, PoolEntrantEvidence, PoolPauseReason, PoolPolicy,
    PoolRunConfiguration, PoolRunId, PoolRunState, PoolRunStatus, PoolStage, RankedPool, RunId,
    SkillEvalError, Tier, TierDestination, Timestamp, TrialUsage,
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

fn frozen_artifact() -> ArtifactDefinition {
    ArtifactDefinition {
        name: ArtifactName("calibration".to_owned()),
        kind: ArtifactKind::Skill,
        root: PathBuf::from("exam"),
        revision: "exam-revision".to_owned(),
        required_destinations: vec![TierDestination::SkillMinimum],
        current_tiers: Vec::new(),
        cases: vec![CaseDefinition {
            id: CaseId("fixed".to_owned()),
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
        }],
    }
}

fn initial_state() -> PoolRunState {
    let tier = Tier::T2;
    let entrants = (0..3)
        .map(|index| {
            let model = identity(tier, index);
            PoolEntrant {
                thinking_levels: vec![model.thinking.clone()],
                retained_lower_thinking_level: None,
                model,
                candidate_timeout_seconds: None,
                catalog_observed_at: Timestamp("2026-08-23T12:00:00-0400".to_owned()),
            }
        })
        .collect::<Vec<_>>();
    let child_runs = (0_u8..3)
        .flat_map(|entrant_index| {
            [PoolStage::Calibration, PoolStage::Qualification]
                .into_iter()
                .map(move |stage| PoolChildRun {
                    tier,
                    entrant_index,
                    thinking_index: 0,
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
            artifacts: vec![frozen_artifact()],
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
                calibration_minimum_reliability_basis_points: 8_000,
                qualification_minimum_reliability_basis_points: 10_000,
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

fn multilevel_state() -> PoolRunState {
    let mut state = initial_state();
    let entrant = &mut state.configuration.entrants.get_mut(&Tier::T2).unwrap()[0];
    entrant.thinking_levels = vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()];
    entrant.model.thinking = "medium".to_owned();
    state.child_runs = state.configuration.entrants[&Tier::T2]
        .iter()
        .enumerate()
        .flat_map(|(entrant_index, entrant)| {
            (0..entrant.thinking_levels.len()).flat_map(move |thinking_index| {
                [PoolStage::Calibration, PoolStage::Qualification]
                    .into_iter()
                    .map(move |stage| PoolChildRun {
                        tier: Tier::T2,
                        entrant_index: u8::try_from(entrant_index).unwrap(),
                        thinking_index: u8::try_from(thinking_index).unwrap(),
                        stage,
                        run_id: RunId(format!(
                            "child-{entrant_index}-{thinking_index}-{}",
                            match stage {
                                PoolStage::Calibration => "calibration",
                                PoolStage::Qualification => "qualification",
                            }
                        )),
                        status: PoolChildStatus::Pending,
                    })
            })
        })
        .collect();
    state
}

fn thinking_identity(index: usize, thinking: &str) -> ModelIdentity {
    let mut model = identity(Tier::T2, index);
    model.thinking = thinking.to_owned();
    model
}

fn thinking_evidence(model: ModelIdentity, is_passing: bool) -> PoolEntrantEvidence {
    let candidate_usage = trial_usage(3);
    let judge_usage = trial_usage(4);
    PoolEntrantEvidence {
        stage: PoolStage::Calibration,
        requested_model: model.clone(),
        effective_model: model,
        judge_model: ModelIdentity {
            tier: Tier::T5,
            provider: "judge".to_owned(),
            model: "judge".to_owned(),
            thinking: "high".to_owned(),
        },
        harnesses: thinking_harnesses(),
        is_passing,
        completed_trials: 5,
        expected_trials: 5,
        failed_trials: u32::from(!is_passing),
        catastrophic_trials: 0,
        score: ConfidenceInterval {
            lower: if is_passing { 0.8 } else { 0.1 },
            estimate: if is_passing { 0.9 } else { 0.1 },
            upper: if is_passing { 1.0 } else { 0.2 },
        },
        total_usage: TrialUsage {
            input_tokens: 2,
            output_tokens: 2,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            turns: 2,
            tool_calls: 0,
            elapsed_milliseconds: 2,
            cost_millionths_of_dollar: 7,
        },
        candidate_usage,
        judge_usage,
    }
}

fn thinking_harnesses() -> Vec<HarnessIdentity> {
    (1..=5)
        .map(|index| HarnessIdentity {
            runner_version: "runner".to_owned(),
            pi_version: "pi".to_owned(),
            artifact_revision: "exam-revision".to_owned(),
            tool_policy_digest: format!("case-{index}"),
        })
        .collect()
}

fn qualification_evidence(model: ModelIdentity) -> PoolEntrantEvidence {
    let mut evidence = thinking_evidence(model, true);
    evidence.stage = PoolStage::Qualification;
    evidence.harnesses = (1..=5)
        .map(|index| HarnessIdentity {
            runner_version: "runner".to_owned(),
            pi_version: "pi".to_owned(),
            artifact_revision: "qualification-revision".to_owned(),
            tool_policy_digest: format!("case-{index}"),
        })
        .collect();
    evidence.completed_trials = 15;
    evidence.expected_trials = 15;
    evidence
}

fn trial_usage(cost: u64) -> TrialUsage {
    TrialUsage {
        input_tokens: 1,
        output_tokens: 1,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        turns: 1,
        tool_calls: 0,
        elapsed_milliseconds: 1,
        cost_millionths_of_dollar: cost,
    }
}

fn invalid_message(error: SkillEvalError) -> String {
    match error {
        SkillEvalError::InvalidConfiguration(message) => message,
        other => panic!("expected invalid configuration, got {other:?}"),
    }
}

#[test]
fn candidate_timeout_round_trips_unbounded_and_positive_and_rejects_zero() {
    let directory = TemporaryDirectory::new("candidate-timeout");
    let mut store = FilePoolStore::new(directory.path()).unwrap();
    let state = initial_state();
    store.create_pool(&state).unwrap();

    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.snapshot()).unwrap()).unwrap();
    for entrants in legacy["configuration"]["entrants"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        for entrant in entrants.as_array_mut().unwrap() {
            entrant
                .as_object_mut()
                .unwrap()
                .remove("candidate_timeout_seconds");
            entrant
                .as_object_mut()
                .unwrap()
                .remove("retained_lower_thinking_level");
        }
    }
    fs::write(
        directory.snapshot(),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();
    let loaded = store.load_pool(&state.configuration.run_id).unwrap();
    assert!(
        loaded
            .configuration
            .entrants
            .values()
            .flatten()
            .all(|entrant| { entrant.candidate_timeout_seconds.is_none() })
    );

    let bounded_directory = TemporaryDirectory::new("candidate-timeout-bounded");
    let mut bounded_store = FilePoolStore::new(bounded_directory.path()).unwrap();
    let mut bounded = initial_state();
    bounded.configuration.entrants.get_mut(&Tier::T2).unwrap()[0].candidate_timeout_seconds =
        Some(23);
    bounded_store.create_pool(&bounded).unwrap();
    assert_eq!(
        bounded_store
            .load_pool(&bounded.configuration.run_id)
            .unwrap()
            .configuration
            .entrants[&Tier::T2][0]
            .candidate_timeout_seconds,
        Some(23)
    );

    let zero_directory = TemporaryDirectory::new("candidate-timeout-zero");
    let mut zero_store = FilePoolStore::new(zero_directory.path()).unwrap();
    let mut zero = initial_state();
    zero.configuration.entrants.get_mut(&Tier::T2).unwrap()[0].candidate_timeout_seconds = Some(0);
    assert!(
        invalid_message(zero_store.create_pool(&zero).unwrap_err()).contains("invalid entrant")
    );
}

#[test]
fn retained_lower_route_continues_stronger() {
    let directory = TemporaryDirectory::new("retained-lower");
    let mut store = FilePoolStore::new(directory.path()).unwrap();
    let mut state = multilevel_state();
    state.configuration.entrants.get_mut(&Tier::T2).unwrap()[0].retained_lower_thinking_level =
        Some("low".to_owned());
    store.create_pool(&state).unwrap();
    state.status = PoolRunStatus::Running;
    store.save_pool(&state).unwrap();

    state.pools.push(RankedPool {
        tier: Tier::T2,
        calibration: vec![
            thinking_evidence(thinking_identity(0, "low"), true),
            thinking_evidence(thinking_identity(0, "medium"), true),
            thinking_evidence(thinking_identity(0, "high"), true),
            thinking_evidence(thinking_identity(1, "medium"), true),
            thinking_evidence(thinking_identity(2, "medium"), true),
        ],
        thinking_selections: vec![
            thinking_identity(0, "low"),
            thinking_identity(1, "medium"),
            thinking_identity(2, "medium"),
        ],
        retained_lower_routes: Vec::new(),
        promoted: vec![
            thinking_identity(0, "medium"),
            thinking_identity(1, "medium"),
            thinking_identity(2, "medium"),
        ],
        qualification: Vec::new(),
        ranked: Vec::new(),
        is_complete: false,
    });
    store.save_pool(&state).unwrap();

    state.pools[0]
        .qualification
        .push(qualification_evidence(thinking_identity(0, "low")));
    state.pools[0]
        .retained_lower_routes
        .push(thinking_identity(0, "low"));
    store.save_pool(&state).unwrap();
    assert_eq!(
        store.load_pool(&state.configuration.run_id).unwrap().pools[0].retained_lower_routes,
        vec![thinking_identity(0, "low")]
    );

    let durable = fs::read(directory.snapshot()).unwrap();
    let mut forged = state.clone();
    forged.pools[0].retained_lower_routes[0] = thinking_identity(0, "medium");
    assert!(store.save_pool(&forged).is_err());
    assert_eq!(fs::read(directory.snapshot()).unwrap(), durable);

    let legacy_directory = TemporaryDirectory::new("legacy-retained-lower");
    let mut legacy_store = FilePoolStore::new(legacy_directory.path()).unwrap();
    let mut legacy_state = multilevel_state();
    legacy_store.create_pool(&legacy_state).unwrap();
    legacy_state.status = PoolRunStatus::Running;
    legacy_store.save_pool(&legacy_state).unwrap();
    legacy_state.pools.push(RankedPool {
        tier: Tier::T2,
        calibration: Vec::new(),
        thinking_selections: Vec::new(),
        retained_lower_routes: Vec::new(),
        promoted: Vec::new(),
        qualification: Vec::new(),
        ranked: Vec::new(),
        is_complete: false,
    });
    legacy_store.save_pool(&legacy_state).unwrap();
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(legacy_directory.snapshot()).unwrap()).unwrap();
    for entrant in legacy["configuration"]["entrants"]["t2"]
        .as_array_mut()
        .unwrap()
    {
        entrant
            .as_object_mut()
            .unwrap()
            .remove("retained_lower_thinking_level");
    }
    legacy["pools"][0]
        .as_object_mut()
        .unwrap()
        .remove("retained_lower_routes");
    fs::write(
        legacy_directory.snapshot(),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();
    let loaded = legacy_store
        .load_pool(&legacy_state.configuration.run_id)
        .unwrap();
    assert!(
        loaded.configuration.entrants[&Tier::T2]
            .iter()
            .all(|entrant| entrant.retained_lower_thinking_level.is_none())
    );
    assert!(loaded.pools[0].retained_lower_routes.is_empty());
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
        thinking_selections: Vec::new(),
        retained_lower_routes: Vec::new(),
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

    let artifact_directory = TemporaryDirectory::new("frozen-artifacts");
    let mut artifact_store = FilePoolStore::new(artifact_directory.path()).unwrap();

    let mut empty = initial_state();
    empty.configuration.artifacts.clear();
    assert!(
        invalid_message(artifact_store.create_pool(&empty).unwrap_err())
            .contains("at least one artifact")
    );

    let mut duplicate = initial_state();
    duplicate
        .configuration
        .artifacts
        .push(duplicate.configuration.artifacts[0].clone());
    assert!(
        invalid_message(artifact_store.create_pool(&duplicate).unwrap_err())
            .contains("duplicate frozen artifact")
    );

    let mut blank_revision = initial_state();
    blank_revision.configuration.artifacts[0].revision.clear();
    assert!(
        invalid_message(artifact_store.create_pool(&blank_revision).unwrap_err())
            .contains("incomplete frozen artifact")
    );

    let mut empty_root = initial_state();
    empty_root.configuration.artifacts[0].root = PathBuf::new();
    assert!(
        invalid_message(artifact_store.create_pool(&empty_root).unwrap_err())
            .contains("incomplete frozen artifact")
    );

    let mut holdout_only = initial_state();
    holdout_only.configuration.artifacts[0].cases[0].is_holdout = true;
    assert!(
        invalid_message(artifact_store.create_pool(&holdout_only).unwrap_err())
            .contains("incomplete frozen artifact")
    );
}

#[test]
fn multilevel_preallocation_requires_every_unique_thinking_stage_slot() {
    let directory = TemporaryDirectory::new("thinking-slots");
    let mut store = FilePoolStore::new(directory.path()).unwrap();
    let state = multilevel_state();
    assert_eq!(state.child_runs.len(), 10);
    store.create_pool(&state).unwrap();
    let identities = state
        .child_runs
        .iter()
        .map(|child| {
            (
                child.entrant_index,
                child.thinking_index,
                child.stage,
                child.run_id.clone(),
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(identities.len(), state.child_runs.len());

    let missing_directory = TemporaryDirectory::new("missing-thinking-slot");
    let mut missing_store = FilePoolStore::new(missing_directory.path()).unwrap();
    let mut missing = multilevel_state();
    missing.child_runs.remove(2);
    assert!(
        invalid_message(missing_store.create_pool(&missing).unwrap_err()).contains("unallocated")
    );

    let out_of_range_directory = TemporaryDirectory::new("thinking-out-of-range");
    let mut out_of_range_store = FilePoolStore::new(out_of_range_directory.path()).unwrap();
    let mut out_of_range = multilevel_state();
    out_of_range.child_runs[0].thinking_index = 3;
    assert!(
        invalid_message(out_of_range_store.create_pool(&out_of_range).unwrap_err())
            .contains("thinking index")
    );

    let frozen = fs::read(directory.snapshot()).unwrap();
    let mut changed_levels = state.clone();
    changed_levels
        .configuration
        .entrants
        .get_mut(&Tier::T2)
        .unwrap()[0]
        .thinking_levels[0] = "minimal".to_owned();
    assert!(
        invalid_message(store.save_pool(&changed_levels).unwrap_err()).contains("configuration")
    );
    assert_eq!(fs::read(directory.snapshot()).unwrap(), frozen);

    let mut changed_identity = state;
    changed_identity.child_runs.swap(0, 2);
    changed_identity.child_runs[0].thinking_index = 0;
    changed_identity.child_runs[2].thinking_index = 1;
    assert!(store.save_pool(&changed_identity).is_err());
    assert_eq!(fs::read(directory.snapshot()).unwrap(), frozen);
}

#[test]
fn complete_thinking_evidence_rejects_calibration_skips_and_backs_qualification_skips() {
    let directory = TemporaryDirectory::new("thinking-skips");
    let mut store = FilePoolStore::new(directory.path()).unwrap();
    let mut state = multilevel_state();
    store.create_pool(&state).unwrap();
    state.status = PoolRunStatus::Running;
    store.save_pool(&state).unwrap();
    state.pools.push(RankedPool {
        tier: Tier::T2,
        calibration: vec![
            thinking_evidence(thinking_identity(0, "low"), false),
            thinking_evidence(thinking_identity(0, "medium"), true),
            thinking_evidence(thinking_identity(0, "high"), true),
        ],
        thinking_selections: Vec::new(),
        retained_lower_routes: Vec::new(),
        promoted: Vec::new(),
        qualification: Vec::new(),
        ranked: Vec::new(),
        is_complete: false,
    });
    store.save_pool(&state).unwrap();

    let durable = fs::read(directory.snapshot()).unwrap();
    let mut skipped_calibration = state.clone();
    skipped_calibration.child_runs[4].status = PoolChildStatus::Skipped;
    assert!(
        invalid_message(store.save_pool(&skipped_calibration).unwrap_err()).contains("calibration")
    );
    assert_eq!(fs::read(directory.snapshot()).unwrap(), durable);

    state.pools[0].calibration.extend([
        thinking_evidence(thinking_identity(1, "medium"), true),
        thinking_evidence(thinking_identity(2, "medium"), true),
    ]);
    state.pools[0].thinking_selections = vec![
        thinking_identity(0, "medium"),
        thinking_identity(1, "medium"),
        thinking_identity(2, "medium"),
    ];
    state.pools[0].promoted = vec![
        thinking_identity(0, "medium"),
        thinking_identity(1, "medium"),
        thinking_identity(2, "medium"),
    ];
    store.save_pool(&state).unwrap();

    let mut hidden = state.clone();
    hidden.pools[0].qualification = vec![
        qualification_evidence(thinking_identity(0, "medium")),
        qualification_evidence(thinking_identity(1, "medium")),
        qualification_evidence(thinking_identity(2, "medium")),
    ];
    assert!(invalid_message(store.save_pool(&hidden).unwrap_err()).contains("hides"));

    state.child_runs[1].status = PoolChildStatus::Skipped;
    store.save_pool(&state).unwrap();

    let mut selected = state.clone();
    selected.child_runs[3].status = PoolChildStatus::Skipped;
    assert!(
        invalid_message(store.save_pool(&selected).unwrap_err()).contains("qualification decision")
    );
}

#[test]
fn thinking_selections_are_backed_unique_and_monotonic_before_promotion() {
    let directory = TemporaryDirectory::new("thinking-selection-validation");
    let mut store = FilePoolStore::new(directory.path()).unwrap();
    let mut state = multilevel_state();
    store.create_pool(&state).unwrap();
    state.status = PoolRunStatus::Running;
    store.save_pool(&state).unwrap();
    state.pools.push(RankedPool {
        tier: Tier::T2,
        calibration: vec![
            thinking_evidence(thinking_identity(0, "low"), false),
            thinking_evidence(thinking_identity(0, "medium"), true),
            thinking_evidence(thinking_identity(0, "high"), true),
        ],
        thinking_selections: Vec::new(),
        retained_lower_routes: Vec::new(),
        promoted: Vec::new(),
        qualification: Vec::new(),
        ranked: Vec::new(),
        is_complete: false,
    });
    store.save_pool(&state).unwrap();

    let mut unbacked = state.clone();
    unbacked.pools[0]
        .thinking_selections
        .push(thinking_identity(1, "medium"));
    assert!(invalid_message(store.save_pool(&unbacked).unwrap_err()).contains("backed"));

    state.pools[0]
        .thinking_selections
        .push(thinking_identity(0, "medium"));
    store.save_pool(&state).unwrap();
    let durable = fs::read(directory.snapshot()).unwrap();

    let mut removed = state.clone();
    removed.pools[0].thinking_selections.clear();
    assert!(invalid_message(store.save_pool(&removed).unwrap_err()).contains("backward"));
    assert_eq!(fs::read(directory.snapshot()).unwrap(), durable);

    let mut duplicate = state.clone();
    duplicate.pools[0]
        .thinking_selections
        .push(thinking_identity(0, "medium"));
    assert!(invalid_message(store.save_pool(&duplicate).unwrap_err()).contains("duplicate"));
    assert_eq!(fs::read(directory.snapshot()).unwrap(), durable);

    let mut invalid_promotion = state.clone();
    invalid_promotion.pools[0].promoted = vec![
        thinking_identity(0, "medium"),
        thinking_identity(1, "medium"),
    ];
    assert!(
        invalid_message(store.save_pool(&invalid_promotion).unwrap_err()).contains("promotion")
    );
    assert_eq!(fs::read(directory.snapshot()).unwrap(), durable);
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
fn promotion_backed_qualification_skip_is_the_only_legal_skip_and_is_terminal() {
    let directory = TemporaryDirectory::new("qualification-skip");
    let mut store = FilePoolStore::new(directory.path()).unwrap();
    let mut state = initial_state();
    store.create_pool(&state).unwrap();

    state.status = PoolRunStatus::Running;
    store.save_pool(&state).unwrap();
    state.pools.push(RankedPool {
        tier: Tier::T2,
        calibration: vec![
            thinking_evidence(identity(Tier::T2, 0), false),
            thinking_evidence(identity(Tier::T2, 1), true),
            thinking_evidence(identity(Tier::T2, 2), true),
        ],
        thinking_selections: vec![identity(Tier::T2, 1), identity(Tier::T2, 2)],
        retained_lower_routes: Vec::new(),
        promoted: vec![identity(Tier::T2, 1), identity(Tier::T2, 2)],
        qualification: Vec::new(),
        ranked: Vec::new(),
        is_complete: false,
    });
    store.save_pool(&state).unwrap();

    state.child_runs[1].status = PoolChildStatus::Skipped;
    store.save_pool(&state).unwrap();
    let skipped_bytes = fs::read(directory.snapshot()).unwrap();

    for status in [
        PoolChildStatus::Pending,
        PoolChildStatus::Running,
        PoolChildStatus::Paused,
        PoolChildStatus::Completed,
        PoolChildStatus::Failed,
    ] {
        let mut changed = state.clone();
        changed.child_runs[1].status = status;
        assert!(store.save_pool(&changed).is_err());
        assert_eq!(fs::read(directory.snapshot()).unwrap(), skipped_bytes);
    }

    let unbacked_directory = TemporaryDirectory::new("unbacked-skip");
    let mut unbacked_store = FilePoolStore::new(unbacked_directory.path()).unwrap();
    let mut unbacked = initial_state();
    unbacked_store.create_pool(&unbacked).unwrap();
    unbacked.status = PoolRunStatus::Running;
    unbacked_store.save_pool(&unbacked).unwrap();
    unbacked.child_runs[1].status = PoolChildStatus::Skipped;
    assert!(invalid_message(unbacked_store.save_pool(&unbacked).unwrap_err()).contains("backed"));

    let calibration_directory = TemporaryDirectory::new("calibration-skip");
    let mut calibration_store = FilePoolStore::new(calibration_directory.path()).unwrap();
    let mut calibration = initial_state();
    calibration_store.create_pool(&calibration).unwrap();
    calibration.status = PoolRunStatus::Running;
    calibration_store.save_pool(&calibration).unwrap();
    calibration.pools.push(RankedPool {
        tier: Tier::T2,
        calibration: Vec::new(),
        thinking_selections: Vec::new(),
        retained_lower_routes: Vec::new(),
        promoted: Vec::new(),
        qualification: Vec::new(),
        ranked: Vec::new(),
        is_complete: false,
    });
    calibration_store.save_pool(&calibration).unwrap();
    calibration.child_runs[0].status = PoolChildStatus::Skipped;
    assert!(
        invalid_message(calibration_store.save_pool(&calibration).unwrap_err())
            .contains("calibration")
    );
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
