#[path = "../src/model.rs"]
mod model;
#[path = "../src/model_capabilities.rs"]
mod model_capabilities;
#[path = "../src/models.rs"]
mod models;
#[path = "../src/ports.rs"]
mod ports;
#[path = "../src/source.rs"]
mod source;
#[path = "../src/t1_screen_campaign_store.rs"]
mod t1_screen_campaign_store;
#[path = "../src/t1_screen_store.rs"]
mod t1_screen_store;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use model::{
    CandidateEnvironmentEntry, ConfidenceInterval, HarnessIdentity, ModelIdentity,
    PoolEntrantEvidence, PoolStage, RunId, SkillEvalError, T1ScreenAttemptEvidence,
    T1ScreenCampaignId, T1ScreenCandidateEnvironment, T1ScreenCandidatePrice, T1ScreenCapExtension,
    T1ScreenChildStatus, T1ScreenModelOutcome, T1ScreenModelState, T1ScreenPauseReason,
    T1ScreenPolicy, T1ScreenRouteFailure, T1ScreenRunConfiguration, T1ScreenRunId,
    T1ScreenRunState, T1ScreenRunStatus, Tier, Timestamp, TrialUsage,
};
use ports::{ArtifactSource, RunIdSource};
use sha2::{Digest, Sha256};
use source::FileArtifactSource;
use t1_screen_store::{
    FileT1ScreenStore, T1ScreenFailurePoint, candidate_environment_manifest_digest,
    preallocate_t1_screen_children, t1_screen_classification_digest, t1_screen_effective_caps,
    validate_t1_screen_state, validate_t1_screen_transition,
};

static TEMPORARY_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let sequence = TEMPORARY_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skill-eval-t1-store-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn snapshot(&self) -> PathBuf {
        self.path
            .join(".map/skill-eval/t1-screening/screen-1/state.json")
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}

struct SequentialRunIds {
    next: u64,
}

impl RunIdSource for SequentialRunIds {
    fn next(&mut self) -> Result<RunId, SkillEvalError> {
        let value = self.next;
        self.next += 1;
        Ok(RunId(format!("t1-child-{value:04}")))
    }
}

struct RepeatedRunIds;

impl RunIdSource for RepeatedRunIds {
    fn next(&mut self) -> Result<RunId, SkillEvalError> {
        Ok(RunId("same-child".to_owned()))
    }
}

struct UnsafeRunIds;

impl RunIdSource for UnsafeRunIds {
    fn next(&mut self) -> Result<RunId, SkillEvalError> {
        Ok(RunId("../child".to_owned()))
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn preview() -> model::T1ScreenPreviewReport {
    let root = repository_root();
    let relative = Path::new("research/model-routing/pi-model-capabilities.json");
    let mut report = model_capabilities::t1_screen_preview(&root, relative).unwrap();
    report.snapshot.path = root.join(relative).canonicalize().unwrap();
    report
}

fn initial_state() -> T1ScreenRunState {
    let report = preview();
    let exam_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/model-calibration")
        .canonicalize()
        .unwrap();
    let exam = FileArtifactSource.load(&exam_root).unwrap();
    assert_eq!(exam.cases.len(), 5);
    let mut run_ids = SequentialRunIds { next: 0 };
    let children = preallocate_t1_screen_children(&report.eligible, &mut run_ids).unwrap();
    let models = report
        .eligible
        .iter()
        .map(|row| T1ScreenModelState {
            provider: row.provider.clone(),
            model: row.model.clone(),
            attempts: Vec::new(),
            outcome: None,
        })
        .collect();
    let harness = HarnessIdentity {
        runner_version: env!("CARGO_PKG_VERSION").to_owned(),
        pi_version: report.snapshot.pi_version.clone(),
        artifact_revision: exam.revision.clone(),
        tool_policy_digest: "a".repeat(64),
    };
    let classification_sha256 =
        t1_screen_classification_digest(&report.eligible, &report.excluded).unwrap();
    T1ScreenRunState {
        configuration: T1ScreenRunConfiguration {
            run_id: T1ScreenRunId("screen-1".to_owned()),
            campaign_id: T1ScreenCampaignId("campaign-1".to_owned()),
            created_at: Timestamp("2026-08-25T22:00:00-0400".to_owned()),
            capability_snapshot: report.snapshot,
            classification_sha256,
            eligible: report.eligible,
            excluded: report.excluded,
            exam,
            judge: ModelIdentity {
                tier: Tier::T5,
                provider: "judge-provider".to_owned(),
                model: "judge-model".to_owned(),
                thinking: "high".to_owned(),
            },
            candidate_environment: candidate_environment(vec![harness; 5]),
            policy: T1ScreenPolicy {
                minimum_score: 8,
                calibration_minimum_reliability_basis_points: 8_000,
                maximum_catastrophic_trials: 0,
                repeats_per_case: 1,
                candidate_timeout_seconds: None,
            },
            is_complete_thinking_coverage: true,
            candidate_calls: report.candidate_calls,
            judge_calls: report.judge_calls,
            candidate_price: T1ScreenCandidatePrice {
                input_per_million_tokens: 0,
                output_per_million_tokens: 0,
            },
            owner_approved_judge_cap_millionths_of_dollar: 100,
            provider_enforced_judge_cap_millionths_of_dollar: 80,
        },
        cap_extensions: Vec::new(),
        route_failures: Vec::new(),
        status: T1ScreenRunStatus::Pending,
        child_runs: children,
        models,
        candidate_usage: zero_usage(),
        judge_usage: zero_usage(),
        spent_judge_millionths_of_dollar: 0,
        pause: None,
    }
}

fn thinking_state(levels: &[&str]) -> T1ScreenRunState {
    let mut state = initial_state();
    state.configuration.eligible.truncate(1);
    state.configuration.eligible[0].supported_pi_thinking_levels =
        levels.iter().map(|level| (*level).to_owned()).collect();
    state.configuration.classification_sha256 = t1_screen_classification_digest(
        &state.configuration.eligible,
        &state.configuration.excluded,
    )
    .unwrap();
    let mut run_ids = SequentialRunIds { next: 0 };
    state.child_runs =
        preallocate_t1_screen_children(&state.configuration.eligible, &mut run_ids).unwrap();
    state.models.truncate(1);
    state.models[0].attempts.clear();
    state.models[0].outcome = None;
    let complete_calls = u64::try_from(levels.len()).unwrap() * 5;
    state.configuration.candidate_calls.minimum = complete_calls;
    state.configuration.candidate_calls.maximum = complete_calls;
    state.configuration.judge_calls = state.configuration.candidate_calls.clone();
    state
}

fn candidate_environment(harnesses: Vec<HarnessIdentity>) -> T1ScreenCandidateEnvironment {
    let manifest = vec![CandidateEnvironmentEntry {
        key: "pi-agent/settings.json".to_owned(),
        sha256: "b".repeat(64),
    }];
    let digest = candidate_environment_manifest_digest(&manifest).unwrap();
    T1ScreenCandidateEnvironment {
        harnesses,
        manifest,
        digest,
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

fn usage(cost: u64) -> TrialUsage {
    TrialUsage {
        input_tokens: 1,
        output_tokens: 2,
        cache_read_tokens: 3,
        cache_write_tokens: 4,
        turns: 1,
        tool_calls: 1,
        elapsed_milliseconds: 5,
        cost_millionths_of_dollar: cost,
    }
}

fn sum_usage(left: &TrialUsage, right: &TrialUsage) -> TrialUsage {
    TrialUsage {
        input_tokens: left.input_tokens + right.input_tokens,
        output_tokens: left.output_tokens + right.output_tokens,
        cache_read_tokens: left.cache_read_tokens + right.cache_read_tokens,
        cache_write_tokens: left.cache_write_tokens + right.cache_write_tokens,
        turns: left.turns + right.turns,
        tool_calls: left.tool_calls + right.tool_calls,
        elapsed_milliseconds: left.elapsed_milliseconds + right.elapsed_milliseconds,
        cost_millionths_of_dollar: left.cost_millionths_of_dollar + right.cost_millionths_of_dollar,
    }
}

fn attempt(
    state: &T1ScreenRunState,
    child_index: usize,
    is_passing: bool,
) -> T1ScreenAttemptEvidence {
    let child = &state.child_runs[child_index];
    let candidate_usage = usage(0);
    let judge_usage = usage(5);
    T1ScreenAttemptEvidence {
        child_run_id: child.run_id.clone(),
        evidence: PoolEntrantEvidence {
            stage: PoolStage::Calibration,
            requested_model: child.model.clone(),
            effective_model: child.model.clone(),
            judge_model: state.configuration.judge.clone(),
            harnesses: state.configuration.candidate_environment.harnesses.clone(),
            is_passing,
            completed_trials: 5,
            expected_trials: 5,
            failed_trials: u32::from(!is_passing),
            catastrophic_trials: 0,
            score: ConfidenceInterval {
                lower: if is_passing { 0.8 } else { 0.0 },
                estimate: if is_passing { 1.0 } else { 0.4 },
                upper: 1.0,
            },
            total_usage: sum_usage(&candidate_usage, &judge_usage),
            candidate_usage,
            judge_usage,
        },
    }
}

fn record_attempt(
    state: &mut T1ScreenRunState,
    child_index: usize,
    is_passing: bool,
    status: T1ScreenChildStatus,
) {
    state.child_runs[child_index].status = status;
    let evidence = attempt(state, child_index, is_passing);
    state.candidate_usage = sum_usage(&state.candidate_usage, &evidence.evidence.candidate_usage);
    state.judge_usage = sum_usage(&state.judge_usage, &evidence.evidence.judge_usage);
    state.spent_judge_millionths_of_dollar = state.judge_usage.cost_millionths_of_dollar;
    state.models[0].attempts.push(evidence);
}

fn cap_extension(previous: u64, next: u64, reason: &str) -> T1ScreenCapExtension {
    T1ScreenCapExtension {
        timestamp: Timestamp("2026-08-26T01:02:03-0400".to_owned()),
        previous_owner_cap_millionths_of_dollar: previous,
        new_owner_cap_millionths_of_dollar: next,
        previous_provider_cap_millionths_of_dollar: previous,
        new_provider_cap_millionths_of_dollar: next,
        owner_reason: reason.to_owned(),
    }
}

fn paused_infrastructure_state() -> T1ScreenRunState {
    let mut state = initial_state();
    state.status = T1ScreenRunStatus::Paused;
    state.child_runs[0].status = T1ScreenChildStatus::Paused;
    state.pause = Some(T1ScreenPauseReason::Infrastructure {
        message: "exact saved route failure".to_owned(),
    });
    state
}

fn route_failed_state(stored: &T1ScreenRunState, reason: &str) -> T1ScreenRunState {
    let mut next = stored.clone();
    let child = stored
        .child_runs
        .iter()
        .find(|child| child.status == T1ScreenChildStatus::Paused)
        .unwrap()
        .clone();
    let message = match stored.pause.as_ref().unwrap() {
        T1ScreenPauseReason::Infrastructure { message } => message,
        _ => unreachable!(),
    };
    next.route_failures.push(T1ScreenRouteFailure {
        timestamp: Timestamp("2026-08-26T02:00:00-0400".to_owned()),
        child_run_id: child.run_id.clone(),
        model: child.model.clone(),
        paused_message_sha256: Sha256::digest(message.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        owner_reason: reason.to_owned(),
    });
    next.status = T1ScreenRunStatus::Running;
    next.pause = None;
    let child_index = next
        .child_runs
        .iter()
        .position(|candidate| candidate.run_id == child.run_id)
        .unwrap();
    next.child_runs[child_index].status = T1ScreenChildStatus::Failed;
    for sibling in next.child_runs.iter_mut().filter(|sibling| {
        sibling.model_index == child.model_index && sibling.thinking_index > child.thinking_index
    }) {
        sibling.status = T1ScreenChildStatus::Skipped;
    }
    let model_index = usize::try_from(child.model_index).unwrap();
    next.models[model_index].outcome = Some(T1ScreenModelOutcome::InfrastructureFailed {
        model: child.model,
        child_run_id: child.run_id,
    });
    next
}

fn paused_judge_cap_state(base_cap: u64, spend: u64) -> T1ScreenRunState {
    let mut state = initial_state();
    state
        .configuration
        .owner_approved_judge_cap_millionths_of_dollar = base_cap;
    state
        .configuration
        .provider_enforced_judge_cap_millionths_of_dollar = base_cap;
    state.status = T1ScreenRunStatus::Paused;
    state.child_runs[0].status = T1ScreenChildStatus::Paused;
    state.judge_usage = usage(spend);
    state.spent_judge_millionths_of_dollar = spend;
    state.pause = Some(T1ScreenPauseReason::JudgeCap {
        spent_millionths_of_dollar: spend,
        owner_approved_millionths_of_dollar: base_cap,
        provider_enforced_millionths_of_dollar: base_cap,
    });
    state
}

fn invalid_message(error: SkillEvalError) -> String {
    match error {
        SkillEvalError::InvalidConfiguration(message) => message,
        other => panic!("expected invalid configuration, got {other:?}"),
    }
}

#[test]
fn exact_complete_call_projection() {
    let state = initial_state();
    assert_eq!(state.configuration.candidate_calls.minimum, 495);
    assert_eq!(state.configuration.candidate_calls.maximum, 495);
    assert_eq!(
        state.configuration.judge_calls,
        state.configuration.candidate_calls
    );
    validate_t1_screen_state(&state).unwrap();

    for calls in [(480, 480), (490, 490), (480, 485)] {
        let mut invalid = state.clone();
        invalid.configuration.candidate_calls.minimum = calls.0;
        invalid.configuration.candidate_calls.maximum = calls.1;
        invalid.configuration.judge_calls = invalid.configuration.candidate_calls.clone();
        assert!(
            invalid_message(validate_t1_screen_state(&invalid).unwrap_err())
                .contains("call projection differs")
        );
    }

    let mut mismatched_judge = state.clone();
    mismatched_judge.configuration.judge_calls.minimum = 480;
    mismatched_judge.configuration.judge_calls.maximum = 480;
    assert!(
        invalid_message(validate_t1_screen_state(&mismatched_judge).unwrap_err())
            .contains("call projection differs")
    );

    let rejected_directory = TemporaryDirectory::new("adaptive-create");
    let mut rejected_store = FileT1ScreenStore::new(&rejected_directory.path).unwrap();
    let mut adaptive = state.clone();
    adaptive.configuration.is_complete_thinking_coverage = false;
    adaptive.configuration.candidate_calls.minimum =
        u64::try_from(adaptive.configuration.eligible.len()).unwrap() * 5;
    adaptive.configuration.judge_calls = adaptive.configuration.candidate_calls.clone();
    assert!(
        invalid_message(rejected_store.create(&adaptive).unwrap_err())
            .contains("must use complete thinking coverage")
    );
    assert!(!rejected_directory.snapshot().exists());

    let historical_directory = TemporaryDirectory::new("adaptive-history");
    let mut historical_store = FileT1ScreenStore::new(&historical_directory.path).unwrap();
    historical_store.create(&state).unwrap();
    let snapshot = historical_directory.snapshot();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&snapshot).unwrap()).unwrap();
    value["configuration"]
        .as_object_mut()
        .unwrap()
        .remove("is_complete_thinking_coverage");
    let adaptive_minimum = u64::try_from(state.configuration.eligible.len()).unwrap() * 5;
    value["configuration"]["candidate_calls"]["minimum"] = serde_json::json!(adaptive_minimum);
    value["configuration"]["judge_calls"]["minimum"] = serde_json::json!(adaptive_minimum);
    fs::write(&snapshot, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

    let loaded = historical_store.load(&state.configuration.run_id).unwrap();
    assert!(!loaded.configuration.is_complete_thinking_coverage);
    assert_eq!(loaded.configuration.candidate_calls.minimum, 105);
    assert_eq!(loaded.configuration.candidate_calls.maximum, 495);
    historical_store.save(&loaded).unwrap();
    assert_eq!(
        historical_store
            .load(&state.configuration.run_id)
            .unwrap()
            .configuration,
        loaded.configuration
    );
}

#[test]
fn real_preview_preallocates_all_99_safe_stable_children() {
    let report = preview();
    assert_eq!(report.eligible.len(), 21);
    assert_eq!(report.excluded.len(), 363);
    let mut first_source = SequentialRunIds { next: 0 };
    let mut second_source = SequentialRunIds { next: 0 };
    let first = preallocate_t1_screen_children(&report.eligible, &mut first_source).unwrap();
    let second = preallocate_t1_screen_children(&report.eligible, &mut second_source).unwrap();

    assert_eq!(first.len(), 99);
    assert_eq!(first, second);
    assert_eq!(
        first
            .iter()
            .map(|child| child.run_id.0.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        first.len()
    );
    for child in &first {
        assert!(
            child.run_id.0.chars().all(
                |character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            )
        );
        let row = &report.eligible[usize::try_from(child.model_index).unwrap()];
        assert_eq!(child.model.provider, row.provider);
        assert_eq!(child.model.model, row.model);
        assert_eq!(
            child.model.thinking,
            row.supported_pi_thinking_levels[usize::try_from(child.thinking_index).unwrap()]
        );
    }
}

#[test]
fn candidate_timeout_round_trips_unbounded_and_positive_and_rejects_zero() {
    let legacy_directory = TemporaryDirectory::new("candidate-timeout-legacy");
    let mut legacy_store = FileT1ScreenStore::new(&legacy_directory.path).unwrap();
    let legacy_state = initial_state();
    legacy_store.create(&legacy_state).unwrap();
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(legacy_directory.snapshot()).unwrap()).unwrap();
    legacy["configuration"]["policy"]
        .as_object_mut()
        .unwrap()
        .remove("candidate_timeout_seconds");
    fs::write(
        legacy_directory.snapshot(),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();
    assert_eq!(
        legacy_store
            .load(&legacy_state.configuration.run_id)
            .unwrap()
            .configuration
            .policy
            .candidate_timeout_seconds,
        None
    );

    let bounded_directory = TemporaryDirectory::new("candidate-timeout-bounded");
    let mut bounded_store = FileT1ScreenStore::new(&bounded_directory.path).unwrap();
    let mut bounded = initial_state();
    bounded.configuration.policy.candidate_timeout_seconds = Some(29);
    bounded_store.create(&bounded).unwrap();
    assert_eq!(
        bounded_store
            .load(&bounded.configuration.run_id)
            .unwrap()
            .configuration
            .policy
            .candidate_timeout_seconds,
        Some(29)
    );

    let zero_directory = TemporaryDirectory::new("candidate-timeout-zero");
    let mut zero_store = FileT1ScreenStore::new(&zero_directory.path).unwrap();
    let mut zero = initial_state();
    zero.configuration.policy.candidate_timeout_seconds = Some(0);
    assert!(invalid_message(zero_store.create(&zero).unwrap_err()).contains("thresholds"));
}

#[test]
fn create_load_round_trip_is_create_new_and_path_restricted() {
    let directory = TemporaryDirectory::new("round-trip");
    let mut store = FileT1ScreenStore::new(&directory.path).unwrap();
    let state = initial_state();
    store.create(&state).unwrap();
    assert_eq!(store.load(&state.configuration.run_id).unwrap(), state);
    assert!(invalid_message(store.create(&state).unwrap_err()).contains("already exists"));
    assert!(directory.snapshot().is_file());

    let mut unsafe_state = state;
    unsafe_state.configuration.run_id = T1ScreenRunId("../escape".to_owned());
    assert!(invalid_message(store.create(&unsafe_state).unwrap_err()).contains("safe path"));
}

#[test]
fn duplicate_collision_unsafe_and_reordered_children_are_rejected() {
    let report = preview();
    assert!(
        invalid_message(
            preallocate_t1_screen_children(&report.eligible, &mut RepeatedRunIds).unwrap_err()
        )
        .contains("collision")
    );
    assert!(
        invalid_message(
            preallocate_t1_screen_children(&report.eligible, &mut UnsafeRunIds).unwrap_err()
        )
        .contains("safe path")
    );

    let mut duplicate = initial_state();
    duplicate.child_runs[1].run_id = duplicate.child_runs[0].run_id.clone();
    assert!(
        invalid_message(validate_t1_screen_state(&duplicate).unwrap_err()).contains("duplicate")
    );

    let mut reordered = initial_state();
    reordered.child_runs.swap(0, 1);
    assert!(invalid_message(validate_t1_screen_state(&reordered).unwrap_err()).contains("order"));
}

#[test]
fn candidate_environment_manifest_rejects_duplicate_unsorted_and_inconsistent_inputs() {
    let mut duplicate = initial_state();
    duplicate
        .configuration
        .candidate_environment
        .manifest
        .push(duplicate.configuration.candidate_environment.manifest[0].clone());
    duplicate.configuration.candidate_environment.digest = candidate_environment_manifest_digest(
        &duplicate.configuration.candidate_environment.manifest,
    )
    .unwrap();
    assert!(
        invalid_message(validate_t1_screen_state(&duplicate).unwrap_err())
            .contains("duplicate or unsorted")
    );

    let mut unsorted = initial_state();
    unsorted
        .configuration
        .candidate_environment
        .manifest
        .push(CandidateEnvironmentEntry {
            key: "pi-agent/models.json".to_owned(),
            sha256: "c".repeat(64),
        });
    unsorted.configuration.candidate_environment.digest = candidate_environment_manifest_digest(
        &unsorted.configuration.candidate_environment.manifest,
    )
    .unwrap();
    assert!(
        invalid_message(validate_t1_screen_state(&unsorted).unwrap_err())
            .contains("duplicate or unsorted")
    );

    let mut inconsistent = initial_state();
    inconsistent.configuration.candidate_environment.digest = "d".repeat(64);
    assert!(
        invalid_message(validate_t1_screen_state(&inconsistent).unwrap_err())
            .contains("manifest digest differs")
    );
}

#[test]
fn legacy_candidate_environment_state_fails_closed_with_specific_error() {
    let directory = TemporaryDirectory::new("legacy-manifest");
    let mut store = FileT1ScreenStore::new(&directory.path).unwrap();
    let state = initial_state();
    store.create(&state).unwrap();
    let snapshot = directory.snapshot();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&snapshot).unwrap()).unwrap();
    value["configuration"]["candidate_environment"]
        .as_object_mut()
        .unwrap()
        .remove("manifest");
    fs::write(&snapshot, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

    assert_eq!(
        invalid_message(store.load(&state.configuration.run_id).unwrap_err()),
        "legacy candidate environment manifest missing"
    );
}

#[test]
fn every_frozen_identity_rejects_drift_and_preserves_bytes() {
    let directory = TemporaryDirectory::new("frozen");
    let mut store = FileT1ScreenStore::new(&directory.path).unwrap();
    let state = initial_state();
    store.create(&state).unwrap();
    let before = fs::read(directory.snapshot()).unwrap();

    let changes: [fn(&mut T1ScreenRunState); 7] = [
        |state| state.configuration.capability_snapshot.sha256 = "c".repeat(64),
        |state| state.configuration.classification_sha256 = "d".repeat(64),
        |state| state.configuration.exam.revision.push('x'),
        |state| state.configuration.judge.model.push('x'),
        |state| state.configuration.candidate_environment.digest = "e".repeat(64),
        |state| state.configuration.policy.minimum_score = 9,
        |state| {
            state
                .configuration
                .owner_approved_judge_cap_millionths_of_dollar += 1
        },
    ];
    for change in changes {
        let mut changed = state.clone();
        change(&mut changed);
        assert!(store.save(&changed).is_err());
        assert_eq!(fs::read(directory.snapshot()).unwrap(), before);
    }

    let mut changed = state.clone();
    changed.configuration.candidate_environment.manifest[0].sha256 = "f".repeat(64);
    changed.configuration.candidate_environment.digest = candidate_environment_manifest_digest(
        &changed.configuration.candidate_environment.manifest,
    )
    .unwrap();
    assert!(store.save(&changed).is_err());
    assert_eq!(fs::read(directory.snapshot()).unwrap(), before);
}

#[test]
fn evidence_is_append_only_exact_and_outcomes_are_terminal() {
    let mut stored = thinking_state(&["off", "high"]);
    stored.status = T1ScreenRunStatus::Running;
    stored.child_runs[0].status = T1ScreenChildStatus::Running;
    let mut next = stored.clone();
    next.child_runs[0].status = T1ScreenChildStatus::Completed;
    let evidence = attempt(&next, 0, false);
    next.candidate_usage = evidence.evidence.candidate_usage.clone();
    next.judge_usage = evidence.evidence.judge_usage.clone();
    next.spent_judge_millionths_of_dollar = 5;
    next.models[0].attempts.push(evidence);
    validate_t1_screen_state(&next).unwrap();
    validate_t1_screen_transition(&stored, &next).unwrap();

    let mut removed = next.clone();
    removed.models[0].attempts.clear();
    assert!(
        invalid_message(validate_t1_screen_transition(&next, &removed).unwrap_err())
            .contains("append-only")
    );

    let mut changed = next.clone();
    changed.models[0].attempts[0].evidence.failed_trials = 4;
    assert!(
        invalid_message(validate_t1_screen_transition(&next, &changed).unwrap_err())
            .contains("append-only")
    );

    let mut wrong_child = next.clone();
    wrong_child.models[0].attempts[0].child_run_id = wrong_child.child_runs[1].run_id.clone();
    assert!(
        invalid_message(validate_t1_screen_state(&wrong_child).unwrap_err())
            .contains("exact child")
    );
}

#[test]
fn thinking_progression_rejects_gaps_early_outcomes_skips_and_duplicates() {
    let mut gap = thinking_state(&["off", "high", "max"]);
    gap.status = T1ScreenRunStatus::Running;
    gap.child_runs[1].status = T1ScreenChildStatus::Running;
    assert!(invalid_message(validate_t1_screen_state(&gap).unwrap_err()).contains("gap"));

    let mut early_selected = thinking_state(&["off", "high", "max"]);
    early_selected.status = T1ScreenRunStatus::Running;
    record_attempt(&mut early_selected, 0, true, T1ScreenChildStatus::Completed);
    early_selected.models[0].outcome = Some(T1ScreenModelOutcome::Selected {
        model: early_selected.child_runs[0].model.clone(),
    });
    assert!(
        invalid_message(validate_t1_screen_state(&early_selected).unwrap_err())
            .contains("complete")
    );

    let mut skipped = thinking_state(&["off", "high", "max"]);
    skipped.status = T1ScreenRunStatus::Running;
    record_attempt(&mut skipped, 0, true, T1ScreenChildStatus::Completed);
    skipped.child_runs[1].status = T1ScreenChildStatus::Skipped;
    assert!(invalid_message(validate_t1_screen_state(&skipped).unwrap_err()).contains("gap"));

    let mut duplicate = thinking_state(&["off", "high", "max"]);
    duplicate.status = T1ScreenRunStatus::Running;
    record_attempt(&mut duplicate, 0, false, T1ScreenChildStatus::Completed);
    duplicate.child_runs[1].status = T1ScreenChildStatus::Completed;
    let repeated = duplicate.models[0].attempts[0].clone();
    duplicate.candidate_usage = sum_usage(
        &duplicate.candidate_usage,
        &repeated.evidence.candidate_usage,
    );
    duplicate.judge_usage = sum_usage(&duplicate.judge_usage, &repeated.evidence.judge_usage);
    duplicate.spent_judge_millionths_of_dollar = duplicate.judge_usage.cost_millionths_of_dollar;
    duplicate.models[0].attempts.push(repeated);
    assert!(
        invalid_message(validate_t1_screen_state(&duplicate).unwrap_err()).contains("exact child")
    );

    let mut no_outcome = thinking_state(&["off", "high"]);
    no_outcome.status = T1ScreenRunStatus::Running;
    record_attempt(&mut no_outcome, 0, false, T1ScreenChildStatus::Completed);
    record_attempt(&mut no_outcome, 1, false, T1ScreenChildStatus::Completed);
    assert!(
        invalid_message(validate_t1_screen_state(&no_outcome).unwrap_err()).contains("no outcome")
    );
}

#[test]
fn thinking_outcome_uses_complete_evidence_and_the_first_pass() {
    let mut selected = thinking_state(&["off", "high", "max"]);
    selected.status = T1ScreenRunStatus::Running;
    record_attempt(&mut selected, 0, false, T1ScreenChildStatus::Completed);
    record_attempt(&mut selected, 1, true, T1ScreenChildStatus::Completed);
    record_attempt(&mut selected, 2, true, T1ScreenChildStatus::Completed);
    selected.models[0].outcome = Some(T1ScreenModelOutcome::Selected {
        model: selected.child_runs[1].model.clone(),
    });
    validate_t1_screen_state(&selected).unwrap();

    let mut wrong_selection = selected.clone();
    wrong_selection.models[0].outcome = Some(T1ScreenModelOutcome::Selected {
        model: wrong_selection.child_runs[2].model.clone(),
    });
    assert!(
        invalid_message(validate_t1_screen_state(&wrong_selection).unwrap_err())
            .contains("first-passing")
    );

    let mut exhausted = thinking_state(&["off", "high", "max"]);
    exhausted.status = T1ScreenRunStatus::Running;
    record_attempt(&mut exhausted, 0, false, T1ScreenChildStatus::Completed);
    record_attempt(&mut exhausted, 1, false, T1ScreenChildStatus::Completed);
    record_attempt(&mut exhausted, 2, false, T1ScreenChildStatus::Exhausted);
    exhausted.models[0].outcome = Some(T1ScreenModelOutcome::Exhausted);
    validate_t1_screen_state(&exhausted).unwrap();
}

#[test]
fn child_and_run_status_graphs_accept_only_declared_edges() {
    let child_statuses = [
        T1ScreenChildStatus::Pending,
        T1ScreenChildStatus::Running,
        T1ScreenChildStatus::Paused,
        T1ScreenChildStatus::Completed,
        T1ScreenChildStatus::Skipped,
        T1ScreenChildStatus::Exhausted,
        T1ScreenChildStatus::Failed,
    ];
    let legal_children = [
        (T1ScreenChildStatus::Pending, T1ScreenChildStatus::Running),
        (T1ScreenChildStatus::Pending, T1ScreenChildStatus::Skipped),
        (T1ScreenChildStatus::Running, T1ScreenChildStatus::Paused),
        (T1ScreenChildStatus::Running, T1ScreenChildStatus::Completed),
        (T1ScreenChildStatus::Running, T1ScreenChildStatus::Exhausted),
        (T1ScreenChildStatus::Running, T1ScreenChildStatus::Failed),
        (T1ScreenChildStatus::Paused, T1ScreenChildStatus::Running),
    ];
    for old in child_statuses {
        for new in child_statuses {
            if old == new {
                continue;
            }
            let mut stored = initial_state();
            let mut next = stored.clone();
            stored.child_runs[0].status = old;
            next.child_runs[0].status = new;
            assert_eq!(
                validate_t1_screen_transition(&stored, &next).is_ok(),
                legal_children.contains(&(old, new)),
                "child {old:?} -> {new:?}"
            );
        }
    }

    let run_statuses = [
        T1ScreenRunStatus::Pending,
        T1ScreenRunStatus::Running,
        T1ScreenRunStatus::Paused,
        T1ScreenRunStatus::AwaitingOwner,
        T1ScreenRunStatus::Completed,
        T1ScreenRunStatus::Failed,
    ];
    let legal_runs = [
        (T1ScreenRunStatus::Pending, T1ScreenRunStatus::Running),
        (T1ScreenRunStatus::Running, T1ScreenRunStatus::Paused),
        (T1ScreenRunStatus::Running, T1ScreenRunStatus::AwaitingOwner),
        (T1ScreenRunStatus::Running, T1ScreenRunStatus::Failed),
        (T1ScreenRunStatus::Paused, T1ScreenRunStatus::Running),
        (T1ScreenRunStatus::Paused, T1ScreenRunStatus::Failed),
        (
            T1ScreenRunStatus::AwaitingOwner,
            T1ScreenRunStatus::Completed,
        ),
        (T1ScreenRunStatus::AwaitingOwner, T1ScreenRunStatus::Failed),
    ];
    for old in run_statuses {
        for new in run_statuses {
            if old == new {
                continue;
            }
            let mut stored = initial_state();
            let mut next = stored.clone();
            stored.status = old;
            next.status = new;
            assert_eq!(
                validate_t1_screen_transition(&stored, &next).is_ok(),
                legal_runs.contains(&(old, new)),
                "run {old:?} -> {new:?}"
            );
        }
    }
}

#[test]
fn one_active_child_and_terminal_skip_exhaustion_are_enforced() {
    let mut two_running = initial_state();
    two_running.status = T1ScreenRunStatus::Running;
    two_running.child_runs[0].status = T1ScreenChildStatus::Running;
    two_running.child_runs[1].status = T1ScreenChildStatus::Running;
    assert!(
        invalid_message(validate_t1_screen_state(&two_running).unwrap_err()).contains("one active")
    );

    for terminal in [
        T1ScreenChildStatus::Skipped,
        T1ScreenChildStatus::Exhausted,
        T1ScreenChildStatus::Completed,
        T1ScreenChildStatus::Failed,
    ] {
        let mut stored = initial_state();
        stored.child_runs[0].status = terminal;
        for next_status in [
            T1ScreenChildStatus::Pending,
            T1ScreenChildStatus::Running,
            T1ScreenChildStatus::Paused,
        ] {
            let mut next = stored.clone();
            next.child_runs[0].status = next_status;
            assert!(validate_t1_screen_transition(&stored, &next).is_err());
        }
    }
}

#[test]
fn usage_spend_are_checked_monotonic_and_stop_at_both_caps() {
    let stored = initial_state();
    let mut next = stored.clone();
    next.judge_usage = usage(80);
    next.spent_judge_millionths_of_dollar = 80;
    validate_t1_screen_state(&next).unwrap();
    validate_t1_screen_transition(&stored, &next).unwrap();

    next.status = T1ScreenRunStatus::Paused;
    next.pause = Some(T1ScreenPauseReason::JudgeCap {
        spent_millionths_of_dollar: 80,
        owner_approved_millionths_of_dollar: 100,
        provider_enforced_millionths_of_dollar: 80,
    });
    validate_t1_screen_state(&next).unwrap();

    let mut over = next.clone();
    over.status = T1ScreenRunStatus::Running;
    over.pause = None;
    over.judge_usage.cost_millionths_of_dollar = 81;
    over.spent_judge_millionths_of_dollar = 81;
    assert!(invalid_message(validate_t1_screen_state(&over).unwrap_err()).contains("exceeds"));

    let mut decreased = next.clone();
    decreased.judge_usage.cost_millionths_of_dollar = 79;
    decreased.spent_judge_millionths_of_dollar = 79;
    assert!(
        invalid_message(validate_t1_screen_transition(&next, &decreased).unwrap_err())
            .contains("decrease")
    );

    let mut candidate_cost = stored;
    candidate_cost.candidate_usage.cost_millionths_of_dollar = 1;
    assert!(
        invalid_message(validate_t1_screen_state(&candidate_cost).unwrap_err())
            .contains("cost zero")
    );
}

#[test]
fn exact_fifteen_to_twenty_million_extension_preserves_every_other_field() {
    let stored = paused_judge_cap_state(15_000_000, 5_811_172);
    let mut next = stored.clone();
    next.cap_extensions.push(cap_extension(
        15_000_000,
        20_000_000,
        "Owner approved the remaining judge work",
    ));

    validate_t1_screen_state(&next).unwrap();
    validate_t1_screen_transition(&stored, &next).unwrap();
    assert_eq!(
        t1_screen_effective_caps(&next).unwrap(),
        (20_000_000, 20_000_000)
    );
    assert_eq!(next.configuration, stored.configuration);
    assert_eq!(next.child_runs, stored.child_runs);
    assert_eq!(next.models, stored.models);
    assert_eq!(next.candidate_usage, stored.candidate_usage);
    assert_eq!(next.judge_usage, stored.judge_usage);
    assert_eq!(next.spent_judge_millionths_of_dollar, 5_811_172);
    assert_eq!(next.pause, stored.pause);
}

#[test]
fn cap_extensions_chain_and_accept_an_updated_pause_boundary() {
    let stored = paused_judge_cap_state(15_000_000, 5_811_172);
    let mut first = stored.clone();
    first
        .cap_extensions
        .push(cap_extension(15_000_000, 20_000_000, "first"));
    validate_t1_screen_transition(&stored, &first).unwrap();

    let mut second = first.clone();
    let mut second_extension = cap_extension(20_000_000, 25_000_000, "second");
    second_extension.timestamp = Timestamp("2026-08-26T01:02:04-0400".to_owned());
    second.cap_extensions.push(second_extension);
    second.pause = Some(T1ScreenPauseReason::JudgeCap {
        spent_millionths_of_dollar: 5_811_172,
        owner_approved_millionths_of_dollar: 25_000_000,
        provider_enforced_millionths_of_dollar: 25_000_000,
    });
    validate_t1_screen_state(&second).unwrap();
    validate_t1_screen_transition(&first, &second).unwrap();
    assert_eq!(
        t1_screen_effective_caps(&second).unwrap(),
        (25_000_000, 25_000_000)
    );

    let mut reordered = second.clone();
    reordered.cap_extensions.swap(0, 1);
    assert!(validate_t1_screen_transition(&second, &reordered).is_err());
    let mut rewritten = second.clone();
    rewritten.cap_extensions[0]
        .owner_reason
        .push_str(" rewritten");
    assert!(validate_t1_screen_transition(&second, &rewritten).is_err());
    assert!(validate_t1_screen_transition(&second, &first).is_err());
}

#[test]
fn cap_extension_chain_rejects_wrong_prior_decrease_provider_over_owner_and_bad_metadata() {
    let stored = paused_judge_cap_state(15_000_000, 5_811_172);
    let changes: [fn(&mut T1ScreenCapExtension); 5] = [
        |extension| extension.previous_owner_cap_millionths_of_dollar -= 1,
        |extension| extension.new_owner_cap_millionths_of_dollar = 15_000_000,
        |extension| extension.new_provider_cap_millionths_of_dollar = 20_000_001,
        |extension| extension.owner_reason = "   ".to_owned(),
        |extension| extension.timestamp = Timestamp("2026-02-30T01:02:03-0400".to_owned()),
    ];
    for change in changes {
        let mut next = stored.clone();
        let mut extension = cap_extension(15_000_000, 20_000_000, "approved");
        change(&mut extension);
        next.cap_extensions.push(extension);
        assert!(validate_t1_screen_state(&next).is_err());
        assert!(validate_t1_screen_transition(&stored, &next).is_err());
    }

    let mut value =
        serde_json::to_value(cap_extension(15_000_000, 20_000_000, "approved")).unwrap();
    value["unknown"] = serde_json::json!(true);
    assert!(serde_json::from_value::<T1ScreenCapExtension>(value).is_err());
    assert!(serde_json::from_str::<T1ScreenCapExtension>(
        r#"{"timestamp":"2026-08-26T01:02:03-0400","previous_owner_cap_millionths_of_dollar":15000000,"new_owner_cap_millionths_of_dollar":18446744073709551616,"previous_provider_cap_millionths_of_dollar":15000000,"new_provider_cap_millionths_of_dollar":20000000,"owner_reason":"approved"}"#
    )
    .is_err());
}

#[test]
fn cap_extension_store_is_append_only_and_only_paused_judge_cap_can_append() {
    let directory = TemporaryDirectory::new("cap-append");
    let mut store = FileT1ScreenStore::new(&directory.path).unwrap();
    let stored = paused_judge_cap_state(15_000_000, 5_811_172);
    store.create(&initial_state()).unwrap();
    let mut running = initial_state();
    running.status = T1ScreenRunStatus::Running;
    running.child_runs[0].status = T1ScreenChildStatus::Running;
    store.save(&running).unwrap();
    let mut paused = running;
    paused.status = T1ScreenRunStatus::Paused;
    paused.child_runs[0].status = T1ScreenChildStatus::Paused;
    paused.pause = Some(T1ScreenPauseReason::JudgeCap {
        spent_millionths_of_dollar: 0,
        owner_approved_millionths_of_dollar: 100,
        provider_enforced_millionths_of_dollar: 80,
    });
    store.save(&paused).unwrap();
    let mut extended = paused.clone();
    extended.cap_extensions.push(T1ScreenCapExtension {
        timestamp: Timestamp("2026-08-26T01:02:03-0400".to_owned()),
        previous_owner_cap_millionths_of_dollar: 100,
        new_owner_cap_millionths_of_dollar: 120,
        previous_provider_cap_millionths_of_dollar: 80,
        new_provider_cap_millionths_of_dollar: 100,
        owner_reason: "approved".to_owned(),
    });
    store.save(&extended).unwrap();

    let before = fs::read(directory.snapshot()).unwrap();
    for mut invalid in [paused.clone(), extended.clone()] {
        if invalid.cap_extensions.is_empty() {
            invalid.cap_extensions.push(T1ScreenCapExtension {
                timestamp: Timestamp("2026-08-26T01:02:03-0400".to_owned()),
                previous_owner_cap_millionths_of_dollar: 100,
                new_owner_cap_millionths_of_dollar: 120,
                previous_provider_cap_millionths_of_dollar: 80,
                new_provider_cap_millionths_of_dollar: 100,
                owner_reason: "rewritten".to_owned(),
            });
        } else {
            invalid.cap_extensions.clear();
        }
        assert!(store.save(&invalid).is_err());
        assert_eq!(fs::read(directory.snapshot()).unwrap(), before);
    }

    for status in [
        T1ScreenRunStatus::Pending,
        T1ScreenRunStatus::Running,
        T1ScreenRunStatus::AwaitingOwner,
        T1ScreenRunStatus::Completed,
        T1ScreenRunStatus::Failed,
    ] {
        let mut invalid = stored.clone();
        invalid.status = status;
        invalid.pause = None;
        let mut next = invalid.clone();
        next.cap_extensions
            .push(cap_extension(15_000_000, 20_000_000, "approved"));
        assert!(validate_t1_screen_transition(&invalid, &next).is_err());
    }
}

#[test]
fn old_snapshot_without_extension_history_loads_with_empty_history() {
    let directory = TemporaryDirectory::new("old-cap-history");
    let mut store = FileT1ScreenStore::new(&directory.path).unwrap();
    let state = initial_state();
    store.create(&state).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.snapshot()).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("cap_extensions");
    fs::write(
        directory.snapshot(),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();

    let loaded = store.load(&state.configuration.run_id).unwrap();
    assert!(loaded.cap_extensions.is_empty());
    assert_eq!(t1_screen_effective_caps(&loaded).unwrap(), (100, 80));
}

#[test]
fn malformed_unknown_snapshots_fail_closed() {
    let directory = TemporaryDirectory::new("unknown");
    let mut store = FileT1ScreenStore::new(&directory.path).unwrap();
    let state = initial_state();
    store.create(&state).unwrap();

    let mut value = serde_json::to_value(&state).unwrap();
    value["recommendation"] = serde_json::json!(null);
    fs::write(directory.snapshot(), serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        invalid_message(store.load(&state.configuration.run_id).unwrap_err()).contains("malformed")
    );

    fs::write(directory.snapshot(), b"{not-json\n").unwrap();
    assert!(
        invalid_message(store.load(&state.configuration.run_id).unwrap_err()).contains("malformed")
    );
}

#[test]
fn write_sync_rename_and_directory_sync_failures_preserve_prior_bytes() {
    for failure in [
        T1ScreenFailurePoint::Write,
        T1ScreenFailurePoint::FileSync,
        T1ScreenFailurePoint::Rename,
        T1ScreenFailurePoint::DirectorySync,
    ] {
        let directory = TemporaryDirectory::new("atomic");
        let mut store = FileT1ScreenStore::new(&directory.path).unwrap();
        let state = initial_state();
        store.create(&state).unwrap();
        let before = fs::read(directory.snapshot()).unwrap();
        let mut next = state.clone();
        next.status = T1ScreenRunStatus::Running;

        let mut failing = FileT1ScreenStore::with_failure(&directory.path, failure).unwrap();
        assert!(matches!(
            failing.save(&next),
            Err(SkillEvalError::Io { .. })
        ));
        assert_eq!(
            fs::read(directory.snapshot()).unwrap(),
            before,
            "{failure:?}"
        );
        assert_eq!(store.load(&state.configuration.run_id).unwrap(), state);
    }
}

#[test]
fn route_failure_migrates_and_round_trips_exact_authority() {
    let directory = TemporaryDirectory::new("route-migration");
    let mut store = FileT1ScreenStore::new(&directory.path).unwrap();
    let initial = initial_state();
    store.create(&initial).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.snapshot()).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("route_failures");
    fs::write(
        directory.snapshot(),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
    assert!(
        store
            .load(&initial.configuration.run_id)
            .unwrap()
            .route_failures
            .is_empty()
    );

    let mut running = initial;
    running.status = T1ScreenRunStatus::Running;
    running.child_runs[0].status = T1ScreenChildStatus::Running;
    store.save(&running).unwrap();
    let mut paused = running;
    paused.status = T1ScreenRunStatus::Paused;
    paused.child_runs[0].status = T1ScreenChildStatus::Paused;
    paused.pause = Some(T1ScreenPauseReason::Infrastructure {
        message: "exact saved route failure".to_owned(),
    });
    store.save(&paused).unwrap();
    let failed = route_failed_state(&paused, "Owner accepted this exact route failure");
    store.save(&failed).unwrap();

    let loaded = store.load(&failed.configuration.run_id).unwrap();
    assert_eq!(loaded, failed);
    assert_eq!(loaded.route_failures.len(), 1);
    assert_eq!(loaded.child_runs[0].status, T1ScreenChildStatus::Failed);
    assert!(
        loaded
            .child_runs
            .iter()
            .filter(|child| child.model_index == 0)
            .skip(1)
            .all(|child| child.status == T1ScreenChildStatus::Skipped)
    );
    assert!(matches!(
        loaded.models[0].outcome,
        Some(T1ScreenModelOutcome::InfrastructureFailed { .. })
    ));
}

#[test]
fn route_failure_after_passing_thinking_evidence_remains_distinct() {
    let mut stored = thinking_state(&["off", "high", "max"]);
    stored.status = T1ScreenRunStatus::Paused;
    record_attempt(&mut stored, 0, true, T1ScreenChildStatus::Completed);
    stored.child_runs[1].status = T1ScreenChildStatus::Paused;
    stored.pause = Some(T1ScreenPauseReason::Infrastructure {
        message: "exact stronger route failure".to_owned(),
    });
    validate_t1_screen_state(&stored).unwrap();

    let failed = route_failed_state(&stored, "Owner accepted the exact stronger route failure");

    validate_t1_screen_state(&failed).unwrap();
    validate_t1_screen_transition(&stored, &failed).unwrap();
    assert_eq!(failed.models[0].attempts.len(), 1);
    assert!(failed.models[0].attempts[0].evidence.is_passing);
    assert_eq!(failed.child_runs[0].status, T1ScreenChildStatus::Completed);
    assert_eq!(failed.child_runs[1].status, T1ScreenChildStatus::Failed);
    assert_eq!(failed.child_runs[2].status, T1ScreenChildStatus::Skipped);
    assert!(matches!(
        failed.models[0].outcome,
        Some(T1ScreenModelOutcome::InfrastructureFailed { .. })
    ));
}

#[test]
fn route_failure_rejects_invalid_transition_and_preserves_bytes() {
    let stored = paused_infrastructure_state();
    let valid = route_failed_state(&stored, "approved");
    validate_t1_screen_state(&valid).unwrap();
    validate_t1_screen_transition(&stored, &valid).unwrap();

    let mut wrong_digest = valid.clone();
    wrong_digest.route_failures[0].paused_message_sha256 = "0".repeat(64);
    assert!(validate_t1_screen_transition(&stored, &wrong_digest).is_err());
    let mut malformed_digest = valid.clone();
    malformed_digest.route_failures[0].paused_message_sha256 = "A".repeat(64);
    assert!(validate_t1_screen_state(&malformed_digest).is_err());
    let mut blank = valid.clone();
    blank.route_failures[0].owner_reason = "   ".to_owned();
    assert!(validate_t1_screen_state(&blank).is_err());
    let mut old_timestamp = valid.clone();
    old_timestamp.route_failures[0].timestamp = stored.configuration.created_at.clone();
    assert!(validate_t1_screen_state(&old_timestamp).is_err());
    let mut wrong_model = valid.clone();
    wrong_model.route_failures[0].model = stored.child_runs[1].model.clone();
    assert!(validate_t1_screen_state(&wrong_model).is_err());
    let mut duplicate = valid.clone();
    let mut repeated = duplicate.route_failures[0].clone();
    repeated.timestamp = Timestamp("2026-08-26T02:00:01-0400".to_owned());
    duplicate.route_failures.push(repeated);
    assert!(validate_t1_screen_state(&duplicate).is_err());
    let mut changed_usage = valid.clone();
    changed_usage.candidate_usage.input_tokens = 1;
    assert!(validate_t1_screen_transition(&stored, &changed_usage).is_err());
    let mut changed_evidence = valid.clone();
    changed_evidence.models[1].outcome = Some(T1ScreenModelOutcome::Exhausted);
    assert!(validate_t1_screen_transition(&stored, &changed_evidence).is_err());
    let mut wrong_parent = stored.clone();
    wrong_parent.status = T1ScreenRunStatus::Running;
    wrong_parent.pause = None;
    assert!(validate_t1_screen_transition(&wrong_parent, &valid).is_err());
    let mut wrong_child = stored.clone();
    wrong_child.child_runs[0].status = T1ScreenChildStatus::Running;
    assert!(validate_t1_screen_transition(&wrong_child, &valid).is_err());
    let mut wrong_pause = stored.clone();
    wrong_pause.pause = Some(T1ScreenPauseReason::Quota {
        model: wrong_pause.child_runs[0].model.clone(),
        reset_at: None,
    });
    assert!(validate_t1_screen_transition(&wrong_pause, &valid).is_err());
    let mut direct = stored.clone();
    direct.child_runs[0].status = T1ScreenChildStatus::Failed;
    assert!(validate_t1_screen_transition(&stored, &direct).is_err());

    for failure in [
        T1ScreenFailurePoint::Write,
        T1ScreenFailurePoint::FileSync,
        T1ScreenFailurePoint::Rename,
        T1ScreenFailurePoint::DirectorySync,
    ] {
        let directory = TemporaryDirectory::new("route-atomic");
        let mut store = FileT1ScreenStore::new(&directory.path).unwrap();
        store.create(&initial_state()).unwrap();
        let mut running = initial_state();
        running.status = T1ScreenRunStatus::Running;
        running.child_runs[0].status = T1ScreenChildStatus::Running;
        store.save(&running).unwrap();
        let mut paused = running;
        paused.status = T1ScreenRunStatus::Paused;
        paused.child_runs[0].status = T1ScreenChildStatus::Paused;
        paused.pause = Some(T1ScreenPauseReason::Infrastructure {
            message: "exact saved route failure".to_owned(),
        });
        store.save(&paused).unwrap();
        let before = fs::read(directory.snapshot()).unwrap();
        let mut failing = FileT1ScreenStore::with_failure(&directory.path, failure).unwrap();
        assert!(
            failing
                .save(&route_failed_state(&paused, "approved"))
                .is_err()
        );
        assert_eq!(fs::read(directory.snapshot()).unwrap(), before);
    }
}

#[test]
fn stale_route_failure_writer_cannot_replace_saved_authority() {
    let directory = TemporaryDirectory::new("route-stale");
    let mut store = FileT1ScreenStore::new(&directory.path).unwrap();
    store.create(&initial_state()).unwrap();
    let mut running = initial_state();
    running.status = T1ScreenRunStatus::Running;
    running.child_runs[0].status = T1ScreenChildStatus::Running;
    store.save(&running).unwrap();
    let mut paused = running;
    paused.status = T1ScreenRunStatus::Paused;
    paused.child_runs[0].status = T1ScreenChildStatus::Paused;
    paused.pause = Some(T1ScreenPauseReason::Infrastructure {
        message: "exact saved route failure".to_owned(),
    });
    store.save(&paused).unwrap();

    let first = route_failed_state(&paused, "first writer");
    let stale = route_failed_state(&paused, "stale writer");
    let mut first_store = FileT1ScreenStore::open(&directory.path).unwrap();
    let mut stale_store = FileT1ScreenStore::open(&directory.path).unwrap();
    let lock = directory
        .path
        .join(".map/skill-eval/t1-screening/screen-1/.state.lock");
    let before_lock = fs::read(directory.snapshot()).unwrap();
    fs::write(&lock, b"other writer\n").unwrap();
    assert!(invalid_message(first_store.save(&first).unwrap_err()).contains("concurrent writer"));
    assert_eq!(fs::read(directory.snapshot()).unwrap(), before_lock);
    fs::remove_file(lock).unwrap();
    first_store.save(&first).unwrap();
    let saved = fs::read(directory.snapshot()).unwrap();
    assert!(stale_store.save(&stale).is_err());
    assert_eq!(fs::read(directory.snapshot()).unwrap(), saved);
    assert_eq!(
        first_store.load(&first.configuration.run_id).unwrap(),
        first
    );
}

#[test]
fn thinking_stale_writer_cannot_replace_scored_evidence() {
    let directory = TemporaryDirectory::new("thinking-stale");
    let initial = thinking_state(&["off", "high"]);
    let mut store = FileT1ScreenStore::new(&directory.path).unwrap();
    store.create(&initial).unwrap();
    let mut running = initial;
    running.status = T1ScreenRunStatus::Running;
    running.child_runs[0].status = T1ScreenChildStatus::Running;
    store.save(&running).unwrap();

    let mut first = running.clone();
    record_attempt(&mut first, 0, false, T1ScreenChildStatus::Completed);
    let mut stale = running;
    record_attempt(&mut stale, 0, true, T1ScreenChildStatus::Completed);
    let mut first_store = FileT1ScreenStore::open(&directory.path).unwrap();
    let mut stale_store = FileT1ScreenStore::open(&directory.path).unwrap();
    first_store.save(&first).unwrap();
    let saved = fs::read(directory.snapshot()).unwrap();

    assert!(stale_store.save(&stale).is_err());
    assert_eq!(fs::read(directory.snapshot()).unwrap(), saved);
    assert_eq!(
        first_store.load(&first.configuration.run_id).unwrap(),
        first
    );
}

#[cfg(unix)]
#[test]
fn store_and_preallocation_make_no_model_or_process_call() {
    use std::os::unix::fs::PermissionsExt;

    let directory = TemporaryDirectory::new("no-process");
    let bin = directory.path.join("bin");
    fs::create_dir(&bin).unwrap();
    let pi = bin.join("pi");
    let log = directory.path.join("pi-called");
    fs::write(
        &pi,
        format!("#!/bin/sh\nprintf called > {}\nexit 97\n", log.display()),
    )
    .unwrap();
    let mut permissions = fs::metadata(&pi).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&pi, permissions).unwrap();

    let state = initial_state();
    let mut store = FileT1ScreenStore::new(&directory.path).unwrap();
    store.create(&state).unwrap();
    store.load(&state.configuration.run_id).unwrap();
    assert!(!log.exists());
}
