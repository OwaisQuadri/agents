#[path = "../src/model.rs"]
mod model;
#[path = "../src/ports.rs"]
mod ports;
#[path = "../src/t1_screen_campaign_store.rs"]
mod t1_screen_campaign_store;
#[path = "../src/t1_screen_store.rs"]
mod t1_screen_store;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use model::{
    SkillEvalError, T1ScreenCampaignCapExtension, T1ScreenCampaignCapExtensionRequest,
    T1ScreenCampaignId, T1ScreenCampaignRunRetirement, T1ScreenCampaignRunRetirementRequest,
    T1ScreenCampaignStatus, T1ScreenRunId, T1ScreenRunState, T1ScreenRunStatus, Timestamp,
};
use t1_screen_campaign_store::{
    FileT1ScreenCampaignStore, T1_SCREEN_CAMPAIGN_APPROVED_TOTAL, T1ScreenCampaignFailurePoint,
    validate_campaign_state, validate_campaign_transition,
};

static TEMPORARY_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const EXPECTED_TOTAL: u64 = 13_672_958;
const EXPECTED_REMAINING: u64 = 6_327_042;

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let sequence = TEMPORARY_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skill-eval-t1-campaign-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn campaign_snapshot(&self) -> PathBuf {
        self.path
            .join(".map/skill-eval/t1-screening-campaigns/campaign/state.json")
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/t1-screen-campaign-old-runs")
}

fn copy_real_runs(directory: &TemporaryDirectory) -> Vec<T1ScreenRunId> {
    let destination = directory.path.join(".map/skill-eval/t1-screening");
    fs::create_dir_all(&destination).unwrap();
    let mut run_ids = Vec::new();
    for item in fs::read_dir(fixture_root()).unwrap() {
        let item = item.unwrap();
        let run_id = item.file_name().into_string().unwrap();
        let run_directory = destination.join(&run_id);
        fs::create_dir(&run_directory).unwrap();
        fs::copy(
            item.path().join("state.json"),
            run_directory.join("state.json"),
        )
        .unwrap();
        run_ids.push(T1ScreenRunId(run_id));
    }
    run_ids.sort();
    run_ids
}

fn create_real_campaign(
    directory: &TemporaryDirectory,
) -> (FileT1ScreenCampaignStore, model::T1ScreenCampaignState) {
    let run_ids = copy_real_runs(directory);
    let mut store = FileT1ScreenCampaignStore::new(&directory.path).unwrap();
    let state = store
        .create_from_runs(
            &T1ScreenCampaignId("campaign".to_owned()),
            T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
            "Owner approved one total T1 judge budget",
            Timestamp("2026-08-26T03:00:00-0400".to_owned()),
            &run_ids,
        )
        .unwrap();
    (store, state)
}

fn pause_campaign(
    store: &mut FileT1ScreenCampaignStore,
    state: &model::T1ScreenCampaignState,
) -> model::T1ScreenCampaignState {
    let mut paused = state.clone();
    paused.status = T1ScreenCampaignStatus::Paused;
    store.save(&paused).unwrap();
    paused
}

fn extension_request(total: u64, reason: &str) -> T1ScreenCampaignCapExtensionRequest {
    T1ScreenCampaignCapExtensionRequest {
        campaign_id: T1ScreenCampaignId("campaign".to_owned()),
        new_approved_total_millionths_of_dollar: total,
        owner_reason: reason.to_owned(),
    }
}

fn retirement_request(
    run_id: &T1ScreenRunId,
    reason: &str,
) -> T1ScreenCampaignRunRetirementRequest {
    T1ScreenCampaignRunRetirementRequest {
        campaign_id: T1ScreenCampaignId("campaign".to_owned()),
        run_id: run_id.clone(),
        owner_reason: reason.to_owned(),
    }
}

fn create_resumable_campaign(
    directory: &TemporaryDirectory,
    run_status: T1ScreenRunStatus,
) -> (
    FileT1ScreenCampaignStore,
    model::T1ScreenCampaignState,
    T1ScreenRunId,
    Vec<u8>,
) {
    let run_ids = copy_real_runs(directory);
    let run_id = run_ids[0].clone();
    let path = directory
        .path
        .join(".map/skill-eval/t1-screening")
        .join(&run_id.0)
        .join("state.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["configuration"]["campaign_id"] = serde_json::json!("campaign");
    value["configuration"]["candidate_environment"]["manifest"] = serde_json::json!([{
        "key": "fixture",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }]);
    value["status"] = serde_json::to_value(run_status).unwrap();
    let run_bytes = serde_json::to_vec_pretty(&value).unwrap();
    fs::write(&path, &run_bytes).unwrap();
    let mut store = FileT1ScreenCampaignStore::new(&directory.path).unwrap();
    let state = store
        .create_from_runs(
            &T1ScreenCampaignId("campaign".to_owned()),
            T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
            "Owner approved one total T1 judge budget",
            Timestamp("2026-08-26T03:00:00-0400".to_owned()),
            &run_ids,
        )
        .unwrap();
    let mut paused = state;
    paused.active_run_id = Some(run_id.clone());
    paused.status = T1ScreenCampaignStatus::Paused;
    store.save(&paused).unwrap();
    (store, paused, run_id, run_bytes)
}

fn create_exhausted_campaign(
    directory: &TemporaryDirectory,
) -> (FileT1ScreenCampaignStore, model::T1ScreenCampaignState) {
    let run_ids = copy_real_runs(directory);
    let path = directory
        .path
        .join(".map/skill-eval/t1-screening")
        .join(&run_ids[0].0)
        .join("state.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let current = value["spent_judge_millionths_of_dollar"].as_u64().unwrap();
    value["spent_judge_millionths_of_dollar"] = serde_json::json!(current + EXPECTED_REMAINING);
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let mut store = FileT1ScreenCampaignStore::new(&directory.path).unwrap();
    let state = store
        .create_from_runs(
            &T1ScreenCampaignId("campaign".to_owned()),
            T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
            "Owner approved one total T1 judge budget",
            Timestamp("2026-08-26T03:00:00-0400".to_owned()),
            &run_ids,
        )
        .unwrap();
    (store, state)
}

fn invalid_message(error: SkillEvalError) -> String {
    match error {
        SkillEvalError::InvalidConfiguration(message) => message,
        other => panic!("expected invalid configuration, got {other:?}"),
    }
}

#[test]
fn real_five_run_import_has_exact_spend_remaining_order_and_immutable_audit_entries() {
    let directory = TemporaryDirectory::new("real");
    let (store, state) = create_real_campaign(&directory);

    assert_eq!(state.runs.len(), 5);
    assert_eq!(
        state.aggregate_judge_spent_millionths_of_dollar,
        EXPECTED_TOTAL
    );
    assert_eq!(
        state.approved_judge_total_millionths_of_dollar
            - state.aggregate_judge_spent_millionths_of_dollar,
        EXPECTED_REMAINING
    );
    assert_eq!(state.status, T1ScreenCampaignStatus::Open);
    assert!(state.active_run_id.is_none());
    assert!(state.runs.iter().all(|run| {
        !run.is_resumable
            && run.candidate_cost_millionths_of_dollar == 0
            && run.superseded_reason.as_deref()
                == Some("legacy candidate environment schema cannot resume")
            && run.state_file_sha256.len() == 64
            && run.canonical_state_path.is_absolute()
    }));
    assert!(state.runs.windows(2).all(|runs| {
        (runs[0].created_at.0.as_str(), runs[0].run_id.0.as_str())
            < (runs[1].created_at.0.as_str(), runs[1].run_id.0.as_str())
    }));
    assert_eq!(
        store
            .load(&T1ScreenCampaignId("campaign".to_owned()))
            .unwrap(),
        state
    );
    let serialized = serde_json::to_value(&state).unwrap();
    for absent in ["evidence", "recommendation", "publication", "decision"] {
        assert!(serialized.get(absent).is_none());
    }
}

#[test]
fn duplicate_omitted_forged_hash_and_candidate_cost_imports_fail_closed() {
    let directory = TemporaryDirectory::new("import-failures");
    let run_ids = copy_real_runs(&directory);
    let campaign_id = T1ScreenCampaignId("campaign".to_owned());
    let created_at = Timestamp("2026-08-26T03:00:00-0400".to_owned());

    let mut duplicate = run_ids.clone();
    duplicate.push(run_ids[0].clone());
    let mut store = FileT1ScreenCampaignStore::new(&directory.path).unwrap();
    assert!(
        invalid_message(
            store
                .create_from_runs(
                    &campaign_id,
                    T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
                    "approved",
                    created_at.clone(),
                    &duplicate,
                )
                .unwrap_err()
        )
        .contains("duplicate")
    );
    assert!(
        invalid_message(
            store
                .create_from_runs(
                    &campaign_id,
                    T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
                    "approved",
                    created_at.clone(),
                    &run_ids[..4],
                )
                .unwrap_err()
        )
        .contains("omitted")
    );

    let first = directory
        .path
        .join(".map/skill-eval/t1-screening")
        .join(&run_ids[0].0)
        .join("state.json");
    let original = fs::read(&first).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    value["configuration"]["run_id"] = serde_json::json!("forged-run");
    fs::write(&first, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        invalid_message(
            store
                .create_from_runs(
                    &campaign_id,
                    T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
                    "approved",
                    created_at.clone(),
                    &run_ids,
                )
                .unwrap_err()
        )
        .contains("identity")
    );

    fs::write(&first, &original).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    value["candidate_usage"]["cost_millionths_of_dollar"] = serde_json::json!(1);
    fs::write(&first, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        invalid_message(
            store
                .create_from_runs(
                    &campaign_id,
                    T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
                    "approved",
                    created_at,
                    &run_ids,
                )
                .unwrap_err()
        )
        .contains("candidate cost")
    );
}

#[test]
fn changed_imported_state_hash_is_rejected_during_reconciliation() {
    let directory = TemporaryDirectory::new("hash");
    let (mut store, state) = create_real_campaign(&directory);
    let path = &state.runs[0].canonical_state_path;
    let mut bytes = fs::read(path).unwrap();
    bytes.push(b' ');
    fs::write(path, bytes).unwrap();

    assert!(
        invalid_message(store.reconcile(&state.campaign_id).unwrap_err()).contains("state bytes")
    );
    assert_eq!(store.load(&state.campaign_id).unwrap(), state);
}

#[test]
fn path_unknown_field_cap_blank_reason_over_cap_and_overflow_failures_preserve_no_campaign() {
    let directory = TemporaryDirectory::new("strict");
    let run_ids = copy_real_runs(&directory);
    let mut store = FileT1ScreenCampaignStore::new(&directory.path).unwrap();
    let campaign_id = T1ScreenCampaignId("campaign".to_owned());
    let created_at = Timestamp("2026-08-26T03:00:00-0400".to_owned());

    for (cap, reason) in [
        (19_999_999, "approved"),
        (20_000_001, "approved"),
        (20_000_000, " "),
    ] {
        assert!(
            store
                .create_from_runs(&campaign_id, cap, reason, created_at.clone(), &run_ids)
                .is_err()
        );
    }

    let (_, mut state) = create_real_campaign(&TemporaryDirectory::new("path-source"));
    state.campaign_id = T1ScreenCampaignId("other".to_owned());
    state.runs[0].canonical_state_path = PathBuf::from("/tmp/escape/state.json");
    assert!(validate_campaign_state(&state).is_err());

    let first_overflow_path = PathBuf::from("/tmp")
        .join(&state.runs[0].run_id.0)
        .join("state.json");
    let overflow_runs = vec![
        model::T1ScreenCampaignRunEntry {
            canonical_state_path: first_overflow_path,
            judge_spend_millionths_of_dollar: u64::MAX,
            ..state.runs[0].clone()
        },
        model::T1ScreenCampaignRunEntry {
            run_id: T1ScreenRunId("overflow-2".to_owned()),
            canonical_state_path: PathBuf::from("/tmp/overflow-2/state.json"),
            created_at: Timestamp("2026-08-26T03:00:01-0400".to_owned()),
            judge_spend_millionths_of_dollar: 1,
            ..state.runs[1].clone()
        },
    ];
    state.runs = overflow_runs;
    state.aggregate_judge_spent_millionths_of_dollar = 0;
    let overflow_message = invalid_message(validate_campaign_state(&state).unwrap_err());
    assert!(overflow_message.contains("overflow"), "{overflow_message}");

    let state = store
        .create_from_runs(
            &campaign_id,
            T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
            "approved",
            Timestamp("2026-08-26T03:00:00-0400".to_owned()),
            &run_ids,
        )
        .unwrap();
    let snapshot = directory.campaign_snapshot();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&snapshot).unwrap()).unwrap();
    value["unknown"] = serde_json::json!(true);
    fs::write(&snapshot, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(store.load(&state.campaign_id).is_err());
}

#[test]
fn append_only_order_immutable_fields_one_active_and_exact_aggregate_are_enforced() {
    let directory = TemporaryDirectory::new("transition");
    let (_, state) = create_real_campaign(&directory);

    let mut rewritten = state.clone();
    rewritten.owner_reason.push_str(" rewritten");
    assert!(validate_campaign_transition(&state, &rewritten).is_err());

    let mut unauthorized_total = state.clone();
    unauthorized_total.approved_judge_total_millionths_of_dollar = 66_038_087;
    assert!(validate_campaign_state(&unauthorized_total).is_err());
    assert!(validate_campaign_transition(&state, &unauthorized_total).is_err());

    let mut reordered = state.clone();
    reordered.runs.swap(0, 1);
    assert!(validate_campaign_transition(&state, &reordered).is_err());

    let mut omitted = state.clone();
    omitted.runs.pop();
    omitted.aggregate_judge_spent_millionths_of_dollar = omitted
        .runs
        .iter()
        .map(|run| run.judge_spend_millionths_of_dollar)
        .sum();
    assert!(validate_campaign_transition(&state, &omitted).is_err());

    let mut wrong_sum = state.clone();
    wrong_sum.aggregate_judge_spent_millionths_of_dollar += 1;
    assert!(validate_campaign_state(&wrong_sum).is_err());

    let mut active_legacy = state;
    active_legacy.active_run_id = Some(active_legacy.runs[0].run_id.clone());
    assert!(validate_campaign_state(&active_legacy).is_err());
}

#[test]
fn legacy_campaign_load_defaults_to_empty_extension_and_retirement_history() {
    let directory = TemporaryDirectory::new("legacy-history-fields");
    let (store, state) = create_real_campaign(&directory);
    let snapshot = directory.campaign_snapshot();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&snapshot).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("cap_extensions");
    value.as_object_mut().unwrap().remove("retirements");
    fs::write(&snapshot, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

    let loaded = store.load(&state.campaign_id).unwrap();

    assert!(loaded.cap_extensions.is_empty());
    assert!(loaded.retirements.is_empty());
    assert_eq!(
        loaded.approved_judge_total_millionths_of_dollar,
        T1_SCREEN_CAMPAIGN_APPROVED_TOTAL
    );
}

#[test]
fn paused_campaign_extensions_reload_as_one_exact_append_only_chain() {
    let directory = TemporaryDirectory::new("extensions");
    let (mut store, state) = create_real_campaign(&directory);
    let paused = pause_campaign(&mut store, &state);
    let run_bytes = serde_json::to_vec(&paused.runs).unwrap();
    let evidence_bytes = paused
        .runs
        .iter()
        .map(|run| fs::read(&run.canonical_state_path).unwrap())
        .collect::<Vec<_>>();

    let first = store
        .extend_cap(
            &extension_request(66_038_087, "Owner approved the aggregate campaign total"),
            Timestamp("2026-08-26T04:00:00-0400".to_owned()),
        )
        .unwrap();

    assert_eq!(first.approved_judge_total_millionths_of_dollar, 66_038_087);
    assert_eq!(first.status, T1ScreenCampaignStatus::Open);
    assert!(first.active_run_id.is_none());
    assert_eq!(
        first.aggregate_judge_spent_millionths_of_dollar,
        EXPECTED_TOTAL
    );
    assert_eq!(serde_json::to_vec(&first.runs).unwrap(), run_bytes);
    assert_eq!(first.cap_extensions.len(), 1);
    assert_eq!(
        first.cap_extensions[0],
        T1ScreenCampaignCapExtension {
            timestamp: Timestamp("2026-08-26T04:00:00-0400".to_owned()),
            previous_approved_total_millionths_of_dollar: 20_000_000,
            new_approved_total_millionths_of_dollar: 66_038_087,
            owner_reason: "Owner approved the aggregate campaign total".to_owned(),
        }
    );
    assert_eq!(store.load(&state.campaign_id).unwrap(), first);

    let paused_again = pause_campaign(&mut store, &first);
    let second = store
        .extend_cap(
            &extension_request(70_000_000, "Owner approved a second aggregate total"),
            Timestamp("2026-08-26T05:00:00-0400".to_owned()),
        )
        .unwrap();

    assert_eq!(second.cap_extensions.len(), 2);
    assert_eq!(
        second.cap_extensions[1].previous_approved_total_millionths_of_dollar,
        66_038_087
    );
    assert_eq!(
        second.cap_extensions[1].new_approved_total_millionths_of_dollar,
        70_000_000
    );
    assert_eq!(second.cap_extensions[..1], paused_again.cap_extensions);
    assert_eq!(serde_json::to_vec(&second.runs).unwrap(), run_bytes);
    assert_eq!(
        second
            .runs
            .iter()
            .map(|run| fs::read(&run.canonical_state_path).unwrap())
            .collect::<Vec<_>>(),
        evidence_bytes
    );
    assert_eq!(
        second.aggregate_judge_spent_millionths_of_dollar,
        EXPECTED_TOTAL
    );
    assert_eq!(store.load(&state.campaign_id).unwrap(), second);
}

#[test]
fn exhausted_campaign_reopens_after_an_extension() {
    let directory = TemporaryDirectory::new("exhausted-extension");
    let (mut store, state) = create_exhausted_campaign(&directory);
    assert_eq!(state.status, T1ScreenCampaignStatus::Exhausted);
    assert_eq!(
        state.aggregate_judge_spent_millionths_of_dollar,
        T1_SCREEN_CAMPAIGN_APPROVED_TOTAL
    );

    let extended = store
        .extend_cap(
            &extension_request(66_038_087, "Owner approved more aggregate authority"),
            Timestamp("2026-08-26T04:00:00-0400".to_owned()),
        )
        .unwrap();

    assert_eq!(extended.status, T1ScreenCampaignStatus::Open);
    assert_eq!(
        extended.aggregate_judge_spent_millionths_of_dollar,
        T1_SCREEN_CAMPAIGN_APPROVED_TOTAL
    );
}

#[test]
fn extension_rejects_invalid_states_amounts_reasons_and_timestamps() {
    let directory = TemporaryDirectory::new("extension-invalid");
    let (mut store, state) = create_real_campaign(&directory);
    assert!(
        invalid_message(
            store
                .extend_cap(
                    &extension_request(66_038_087, "approved"),
                    Timestamp("2026-08-26T04:00:00-0400".to_owned()),
                )
                .unwrap_err()
        )
        .contains("paused or exhausted")
    );

    let paused = pause_campaign(&mut store, &state);
    for (total, reason) in [
        (0, "approved"),
        (19_999_999, "approved"),
        (20_000_000, "approved"),
        (66_038_087, "   "),
    ] {
        assert!(
            store
                .extend_cap(
                    &extension_request(total, reason),
                    Timestamp("2026-08-26T04:00:00-0400".to_owned()),
                )
                .is_err()
        );
        assert_eq!(store.load(&state.campaign_id).unwrap(), paused);
    }
    assert!(
        store
            .extend_cap(
                &extension_request(66_038_087, "approved"),
                Timestamp("2026-02-30T04:00:00-0400".to_owned()),
            )
            .is_err()
    );
    assert!(
        invalid_message(
            store
                .extend_cap(
                    &extension_request(66_038_087, "approved"),
                    Timestamp("2026-08-26T02:59:59-0400".to_owned()),
                )
                .unwrap_err()
        )
        .contains("timestamps")
    );

    for status in [
        T1ScreenCampaignStatus::AwaitingOwner,
        T1ScreenCampaignStatus::Closed,
    ] {
        let mut invalid_state = paused.clone();
        invalid_state.status = status;
        store.save(&invalid_state).unwrap();
        assert!(
            store
                .extend_cap(
                    &extension_request(66_038_087, "approved"),
                    Timestamp("2026-08-26T04:00:00-0400".to_owned()),
                )
                .is_err()
        );
        store.save(&paused).unwrap();
    }
}

#[test]
fn malformed_history_changed_run_evidence_and_stale_transition_fail_closed() {
    let directory = TemporaryDirectory::new("extension-malformed");
    let (mut store, state) = create_real_campaign(&directory);
    let paused = pause_campaign(&mut store, &state);
    let before = fs::read(directory.campaign_snapshot()).unwrap();
    let mut malformed: serde_json::Value = serde_json::from_slice(&before).unwrap();
    malformed["approved_judge_total_millionths_of_dollar"] = serde_json::json!(66_038_087);
    malformed["cap_extensions"] = serde_json::json!([{
        "timestamp": "2026-08-26T04:00:00-0400",
        "previous_approved_total_millionths_of_dollar": 20_000_001,
        "new_approved_total_millionths_of_dollar": 66_038_087,
        "owner_reason": "approved"
    }]);
    fs::write(
        directory.campaign_snapshot(),
        serde_json::to_vec_pretty(&malformed).unwrap(),
    )
    .unwrap();
    assert!(store.load(&state.campaign_id).is_err());
    fs::write(directory.campaign_snapshot(), &before).unwrap();

    let first_run = &paused.runs[0].canonical_state_path;
    let original_run = fs::read(first_run).unwrap();
    let mut changed_run = original_run.clone();
    changed_run.push(b' ');
    fs::write(first_run, changed_run).unwrap();
    assert!(
        invalid_message(
            store
                .extend_cap(
                    &extension_request(66_038_087, "approved"),
                    Timestamp("2026-08-26T04:00:00-0400".to_owned()),
                )
                .unwrap_err()
        )
        .contains("state bytes")
    );
    assert_eq!(fs::read(directory.campaign_snapshot()).unwrap(), before);
    fs::write(first_run, original_run).unwrap();

    let current = store
        .extend_cap(
            &extension_request(66_038_087, "approved"),
            Timestamp("2026-08-26T04:00:00-0400".to_owned()),
        )
        .unwrap();
    let mut unordered = current.clone();
    unordered.approved_judge_total_millionths_of_dollar = 70_000_000;
    unordered.cap_extensions.push(T1ScreenCampaignCapExtension {
        timestamp: Timestamp("2026-08-26T03:59:59-0400".to_owned()),
        previous_approved_total_millionths_of_dollar: 66_038_087,
        new_approved_total_millionths_of_dollar: 70_000_000,
        owner_reason: "unordered approval".to_owned(),
    });
    assert!(
        invalid_message(validate_campaign_state(&unordered).unwrap_err()).contains("timestamps")
    );

    let mut stale = paused;
    stale.approved_judge_total_millionths_of_dollar = 70_000_000;
    stale.cap_extensions.push(T1ScreenCampaignCapExtension {
        timestamp: Timestamp("2026-08-26T05:00:00-0400".to_owned()),
        previous_approved_total_millionths_of_dollar: 20_000_000,
        new_approved_total_millionths_of_dollar: 70_000_000,
        owner_reason: "stale approval".to_owned(),
    });
    stale.status = T1ScreenCampaignStatus::Open;
    assert!(store.save(&stale).is_err());
    assert_eq!(store.load(&state.campaign_id).unwrap(), current);
}

#[test]
fn extension_atomic_and_concurrent_writer_failures_preserve_prior_bytes() {
    for failure in [
        T1ScreenCampaignFailurePoint::Write,
        T1ScreenCampaignFailurePoint::FileSync,
        T1ScreenCampaignFailurePoint::Rename,
        T1ScreenCampaignFailurePoint::DirectorySync,
    ] {
        let directory = TemporaryDirectory::new("extension-atomic");
        let (mut store, state) = create_real_campaign(&directory);
        pause_campaign(&mut store, &state);
        let before = fs::read(directory.campaign_snapshot()).unwrap();
        let mut failing =
            FileT1ScreenCampaignStore::with_failure(&directory.path, failure).unwrap();
        assert!(matches!(
            failing.extend_cap(
                &extension_request(66_038_087, "approved"),
                Timestamp("2026-08-26T04:00:00-0400".to_owned()),
            ),
            Err(SkillEvalError::Io { .. })
        ));
        assert_eq!(fs::read(directory.campaign_snapshot()).unwrap(), before);
    }

    let directory = TemporaryDirectory::new("extension-concurrent");
    let (mut store, state) = create_real_campaign(&directory);
    pause_campaign(&mut store, &state);
    let before = fs::read(directory.campaign_snapshot()).unwrap();
    fs::write(
        directory
            .campaign_snapshot()
            .parent()
            .unwrap()
            .join(".state.lock"),
        b"other writer\n",
    )
    .unwrap();
    assert!(
        invalid_message(
            store
                .extend_cap(
                    &extension_request(66_038_087, "approved"),
                    Timestamp("2026-08-26T04:00:00-0400".to_owned()),
                )
                .unwrap_err()
        )
        .contains("concurrent")
    );
    assert_eq!(fs::read(directory.campaign_snapshot()).unwrap(), before);
}

#[test]
fn write_sync_rename_directory_sync_and_concurrent_writer_failures_preserve_prior_bytes() {
    for failure in [
        T1ScreenCampaignFailurePoint::Write,
        T1ScreenCampaignFailurePoint::FileSync,
        T1ScreenCampaignFailurePoint::Rename,
        T1ScreenCampaignFailurePoint::DirectorySync,
    ] {
        let directory = TemporaryDirectory::new("atomic");
        let (_, state) = create_real_campaign(&directory);
        let before = fs::read(directory.campaign_snapshot()).unwrap();
        let mut next = state.clone();
        next.status = T1ScreenCampaignStatus::Paused;
        let mut failing =
            FileT1ScreenCampaignStore::with_failure(&directory.path, failure).unwrap();
        assert!(matches!(
            failing.save(&next),
            Err(SkillEvalError::Io { .. })
        ));
        assert_eq!(fs::read(directory.campaign_snapshot()).unwrap(), before);
    }

    let directory = TemporaryDirectory::new("concurrent");
    let (mut store, state) = create_real_campaign(&directory);
    let before = fs::read(directory.campaign_snapshot()).unwrap();
    let lock = directory
        .campaign_snapshot()
        .parent()
        .unwrap()
        .join(".state.lock");
    fs::write(&lock, b"other writer\n").unwrap();
    let mut next = state;
    next.status = T1ScreenCampaignStatus::Paused;
    assert!(invalid_message(store.save(&next).unwrap_err()).contains("concurrent"));
    assert_eq!(fs::read(directory.campaign_snapshot()).unwrap(), before);
}

#[test]
fn paused_active_run_retirement_reloads_without_changing_run_bytes_or_spend() {
    let directory = TemporaryDirectory::new("retirement-success");
    let (mut store, paused, run_id, run_bytes) =
        create_resumable_campaign(&directory, T1ScreenRunStatus::Paused);
    let reason = "Owner retired the paused run";

    let retired = store
        .retire_run(
            &retirement_request(&run_id, reason),
            Timestamp("2026-08-26T04:00:00-0400".to_owned()),
        )
        .unwrap();

    assert_eq!(retired.status, T1ScreenCampaignStatus::Open);
    assert!(retired.active_run_id.is_none());
    assert_eq!(retired.retirements.len(), 1);
    assert_eq!(retired.retirements[0].run_id, run_id);
    assert_eq!(retired.retirements[0].owner_reason, reason);
    let retired_entry = retired
        .runs
        .iter()
        .find(|entry| entry.run_id == run_id)
        .unwrap();
    assert!(!retired_entry.is_resumable);
    assert_eq!(retired_entry.superseded_reason.as_deref(), Some(reason));
    assert_eq!(
        retired.aggregate_judge_spent_millionths_of_dollar,
        paused.aggregate_judge_spent_millionths_of_dollar
    );
    assert_eq!(
        retired.approved_judge_total_millionths_of_dollar,
        paused.approved_judge_total_millionths_of_dollar
    );
    assert_eq!(
        fs::read(&retired_entry.canonical_state_path).unwrap(),
        run_bytes
    );
    assert_eq!(store.load(&retired.campaign_id).unwrap(), retired);
}

#[test]
fn retirement_state_and_transition_validation_fail_closed() {
    let directory = TemporaryDirectory::new("retirement-validation");
    let (mut store, paused, run_id, _) =
        create_resumable_campaign(&directory, T1ScreenRunStatus::Paused);
    let retired = store
        .retire_run(
            &retirement_request(&run_id, "Owner retired the paused run"),
            Timestamp("2026-08-26T04:00:00-0400".to_owned()),
        )
        .unwrap();

    let mut wrong_reason = retired.clone();
    wrong_reason.retirements[0].owner_reason = "different reason".to_owned();
    assert!(validate_campaign_state(&wrong_reason).is_err());

    let mut resumable = retired.clone();
    let entry = resumable
        .runs
        .iter_mut()
        .find(|entry| entry.run_id == run_id)
        .unwrap();
    entry.is_resumable = true;
    entry.superseded_reason = None;
    assert!(validate_campaign_state(&resumable).is_err());

    let mut active = retired.clone();
    active.active_run_id = Some(run_id.clone());
    assert!(validate_campaign_state(&active).is_err());

    let mut duplicate = retired.clone();
    duplicate.retirements.push(T1ScreenCampaignRunRetirement {
        timestamp: Timestamp("2026-08-26T05:00:00-0400".to_owned()),
        run_id: run_id.clone(),
        owner_reason: "Owner retired the paused run".to_owned(),
    });
    assert!(validate_campaign_state(&duplicate).is_err());

    let mut unknown = retired.clone();
    unknown.retirements[0].run_id = T1ScreenRunId("unknown-run".to_owned());
    assert!(validate_campaign_state(&unknown).is_err());

    let mut early = retired.clone();
    early.retirements[0].timestamp = early.created_at.clone();
    assert!(validate_campaign_state(&early).is_err());

    let mut rewritten = retired.clone();
    rewritten.runs[1].state_file_sha256 = "b".repeat(64);
    assert!(validate_campaign_transition(&paused, &rewritten).is_err());

    let mut malformed_append = retired.clone();
    malformed_append.retirements[0].timestamp = Timestamp("2026-08-26T02:00:00-0400".to_owned());
    assert!(validate_campaign_transition(&paused, &malformed_append).is_err());
}

#[test]
fn retirement_rejects_stale_bytes_wrong_campaign_run_reason_and_status() {
    let directory = TemporaryDirectory::new("retirement-rejections");
    let (mut store, paused, run_id, run_bytes) =
        create_resumable_campaign(&directory, T1ScreenRunStatus::Paused);
    let request = retirement_request(&run_id, "Owner retired the paused run");
    let path = paused
        .runs
        .iter()
        .find(|entry| entry.run_id == run_id)
        .unwrap()
        .canonical_state_path
        .clone();
    let mut stale = run_bytes.clone();
    stale.push(b' ');
    fs::write(&path, stale).unwrap();
    assert!(
        invalid_message(
            store
                .retire_run(&request, Timestamp("2026-08-26T04:00:00-0400".to_owned()),)
                .unwrap_err()
        )
        .contains("state bytes")
    );
    fs::write(&path, run_bytes).unwrap();

    let mut wrong_campaign = request.clone();
    wrong_campaign.campaign_id = T1ScreenCampaignId("other".to_owned());
    assert!(
        store
            .retire_run(
                &wrong_campaign,
                Timestamp("2026-08-26T04:00:00-0400".to_owned()),
            )
            .is_err()
    );
    let mut wrong_run = request.clone();
    wrong_run.run_id = T1ScreenRunId("other".to_owned());
    assert!(
        store
            .retire_run(&wrong_run, Timestamp("2026-08-26T04:00:00-0400".to_owned()),)
            .is_err()
    );
    let mut blank = request.clone();
    blank.owner_reason = "   ".to_owned();
    assert!(
        store
            .retire_run(&blank, Timestamp("2026-08-26T04:00:00-0400".to_owned()),)
            .is_err()
    );

    let mut open = paused.clone();
    open.status = T1ScreenCampaignStatus::Open;
    store.save(&open).unwrap();
    assert!(
        store
            .retire_run(&request, Timestamp("2026-08-26T04:00:00-0400".to_owned()),)
            .is_err()
    );
    store.save(&paused).unwrap();
    assert!(
        store
            .retire_run(&request, Timestamp("2026-02-30T04:00:00-0400".to_owned()),)
            .is_err()
    );

    let retired = store
        .retire_run(&request, Timestamp("2026-08-26T04:00:00-0400".to_owned()))
        .unwrap();
    assert!(
        store
            .retire_run(&request, Timestamp("2026-08-26T05:00:00-0400".to_owned()),)
            .is_err()
    );
    assert_eq!(store.load(&retired.campaign_id).unwrap(), retired);

    for status in [
        T1ScreenRunStatus::Pending,
        T1ScreenRunStatus::Running,
        T1ScreenRunStatus::AwaitingOwner,
        T1ScreenRunStatus::Completed,
        T1ScreenRunStatus::Failed,
    ] {
        let directory = TemporaryDirectory::new("retirement-run-status");
        let (mut store, state, run_id, _) = create_resumable_campaign(&directory, status);
        assert!(
            store
                .retire_run(
                    &retirement_request(&run_id, "Owner retired the paused run"),
                    Timestamp("2026-08-26T04:00:00-0400".to_owned()),
                )
                .is_err()
        );
        assert_eq!(store.load(&state.campaign_id).unwrap(), state);
    }
}

#[test]
fn retirement_atomic_and_concurrent_writer_failures_preserve_prior_bytes() {
    for failure in [
        T1ScreenCampaignFailurePoint::Write,
        T1ScreenCampaignFailurePoint::FileSync,
        T1ScreenCampaignFailurePoint::Rename,
        T1ScreenCampaignFailurePoint::DirectorySync,
    ] {
        let directory = TemporaryDirectory::new("retirement-atomic");
        let (_, state, run_id, _) =
            create_resumable_campaign(&directory, T1ScreenRunStatus::Paused);
        let before = fs::read(directory.campaign_snapshot()).unwrap();
        let mut failing =
            FileT1ScreenCampaignStore::with_failure(&directory.path, failure).unwrap();
        assert!(matches!(
            failing.retire_run(
                &retirement_request(&run_id, "Owner retired the paused run"),
                Timestamp("2026-08-26T04:00:00-0400".to_owned()),
            ),
            Err(SkillEvalError::Io { .. })
        ));
        assert_eq!(fs::read(directory.campaign_snapshot()).unwrap(), before);
        assert_eq!(failing.load(&state.campaign_id).unwrap(), state);
    }

    let directory = TemporaryDirectory::new("retirement-concurrent");
    let (mut store, state, run_id, _) =
        create_resumable_campaign(&directory, T1ScreenRunStatus::Paused);
    let before = fs::read(directory.campaign_snapshot()).unwrap();
    fs::write(
        directory
            .campaign_snapshot()
            .parent()
            .unwrap()
            .join(".state.lock"),
        b"other writer\n",
    )
    .unwrap();
    assert!(
        invalid_message(
            store
                .retire_run(
                    &retirement_request(&run_id, "Owner retired the paused run"),
                    Timestamp("2026-08-26T04:00:00-0400".to_owned()),
                )
                .unwrap_err()
        )
        .contains("concurrent")
    );
    assert_eq!(fs::read(directory.campaign_snapshot()).unwrap(), before);
    assert_eq!(store.load(&state.campaign_id).unwrap(), state);
}

#[test]
fn retired_run_reconciles_and_a_new_run_can_register_without_reactivation() {
    let directory = TemporaryDirectory::new("retirement-followup");
    let (mut store, _, run_id, run_bytes) =
        create_resumable_campaign(&directory, T1ScreenRunStatus::Paused);
    let retired = store
        .retire_run(
            &retirement_request(&run_id, "Owner retired the paused run"),
            Timestamp("2026-08-26T04:00:00-0400".to_owned()),
        )
        .unwrap();

    assert_eq!(store.reconcile(&retired.campaign_id).unwrap(), retired);

    let mut next_run: T1ScreenRunState = serde_json::from_slice(&run_bytes).unwrap();
    next_run.configuration.run_id = T1ScreenRunId("new-run".to_owned());
    next_run.configuration.created_at = Timestamp("2026-08-26T05:00:00-0400".to_owned());
    next_run.status = T1ScreenRunStatus::Pending;
    next_run.pause = None;
    let next_directory = directory.path.join(".map/skill-eval/t1-screening/new-run");
    fs::create_dir(&next_directory).unwrap();
    fs::write(
        next_directory.join("state.json"),
        serde_json::to_vec_pretty(&next_run).unwrap(),
    )
    .unwrap();

    let registered = store.register_active_run(&next_run).unwrap();

    assert_eq!(
        registered.active_run_id.as_ref(),
        Some(&next_run.configuration.run_id)
    );
    assert_eq!(registered.status, T1ScreenCampaignStatus::Open);
    let old = registered
        .runs
        .iter()
        .find(|entry| entry.run_id == run_id)
        .unwrap();
    assert!(!old.is_resumable);
    assert_eq!(
        old.superseded_reason.as_deref(),
        Some("Owner retired the paused run")
    );
    assert_eq!(registered.retirements, retired.retirements);
}

#[test]
fn retirement_and_extension_timestamps_are_strictly_interleaved() {
    let directory = TemporaryDirectory::new("retirement-extension-order");
    let (mut store, _, run_id, _) =
        create_resumable_campaign(&directory, T1ScreenRunStatus::Paused);
    let retired = store
        .retire_run(
            &retirement_request(&run_id, "Owner retired the paused run"),
            Timestamp("2026-08-26T04:00:00-0400".to_owned()),
        )
        .unwrap();
    let mut paused = retired.clone();
    paused.status = T1ScreenCampaignStatus::Paused;
    store.save(&paused).unwrap();

    assert!(
        store
            .extend_cap(
                &extension_request(66_038_087, "Owner approved more authority"),
                Timestamp("2026-08-26T03:59:59-0400".to_owned()),
            )
            .is_err()
    );
    let extended = store
        .extend_cap(
            &extension_request(66_038_087, "Owner approved more authority"),
            Timestamp("2026-08-26T05:00:00-0400".to_owned()),
        )
        .unwrap();
    assert_eq!(extended.retirements, retired.retirements);
    assert_eq!(extended.cap_extensions.len(), 1);
}

#[cfg(unix)]
#[test]
fn import_follows_no_state_path_symlink_and_makes_no_process_call() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let directory = TemporaryDirectory::new("offline");
    let run_ids = copy_real_runs(&directory);
    let first_directory = directory
        .path
        .join(".map/skill-eval/t1-screening")
        .join(&run_ids[0].0);
    let real = first_directory.join("real.json");
    fs::rename(first_directory.join("state.json"), &real).unwrap();
    symlink(&real, first_directory.join("state.json")).unwrap();
    let mut store = FileT1ScreenCampaignStore::new(&directory.path).unwrap();
    assert!(
        invalid_message(
            store
                .create_from_runs(
                    &T1ScreenCampaignId("campaign".to_owned()),
                    T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
                    "approved",
                    Timestamp("2026-08-26T03:00:00-0400".to_owned()),
                    &run_ids,
                )
                .unwrap_err()
        )
        .contains("escapes")
    );

    fs::remove_file(first_directory.join("state.json")).unwrap();
    fs::rename(real, first_directory.join("state.json")).unwrap();
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
    let state = store
        .create_from_runs(
            &T1ScreenCampaignId("campaign".to_owned()),
            T1_SCREEN_CAMPAIGN_APPROVED_TOTAL,
            "approved",
            Timestamp("2026-08-26T03:00:00-0400".to_owned()),
            &run_ids,
        )
        .unwrap();
    pause_campaign(&mut store, &state);
    store
        .extend_cap(
            &extension_request(66_038_087, "approved"),
            Timestamp("2026-08-26T04:00:00-0400".to_owned()),
        )
        .unwrap();
    assert!(!log.exists());
}
