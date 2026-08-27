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
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use frontier_store::{FileFrontierStore, FrontierFailurePoint, rollback_result};
use model::{
    CaseDrive, CaseId, CommandDefinition, FrontierCaseGroup, FrontierCaseInventoryEntry,
    FrontierCaseKey, FrontierCaseReference, FrontierCaseReviewDecision, FrontierCaseReviewRecord,
    FrontierSuiteConstructionPlan, FrontierSuiteConstructionPolicy, FrontierSuiteInventory,
    FrontierSuiteProposal, FrontierSuiteProposalStatus, FrontierSuiteReviewSet,
    FrontierTierCapacity, FrontierTierSuite, SkillEvalError, Tier, Timestamp,
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn safe_loads_validate_all_four_evidence_shapes() {
    let fixture = Fixture::new();
    let store = FileFrontierStore::new(&fixture.root).unwrap();
    fixture.write("evidence/plan.json", &plan());
    fixture.write("evidence/inventory.json", &inventory());
    fixture.write("evidence/reviews.json", &reviews());
    fixture.write("evidence/proposal.json", &ready_proposal());

    assert_eq!(
        store
            .load_frontier_suite_construction_plan(Path::new("evidence/plan.json"))
            .unwrap(),
        plan()
    );
    assert_eq!(
        store
            .load_frontier_suite_inventory(Path::new("evidence/inventory.json"))
            .unwrap(),
        inventory()
    );
    assert_eq!(
        store
            .load_frontier_suite_review_set(Path::new("evidence/reviews.json"))
            .unwrap(),
        reviews()
    );
    assert_eq!(
        store
            .load_frontier_suite_proposal(Path::new("evidence/proposal.json"))
            .unwrap(),
        ready_proposal()
    );
}

#[test]
fn strict_loads_reject_versions_unknown_fields_and_malformed_shapes() {
    let fixture = Fixture::new();
    let store = FileFrontierStore::new(&fixture.root).unwrap();
    let cases = [
        (
            "bad/version.json",
            json!({"version": 2, "artifact_roots": ["skills/a"], "policy": policy()}),
        ),
        (
            "bad/unknown.json",
            json!({"version": 1, "artifact_roots": ["skills/a"], "policy": policy(), "unknown": true}),
        ),
        (
            "bad/reviews.json",
            json!({"version": 1, "inventory_sha256": "0".repeat(64), "records": [{"key": key(0), "reviewer": "", "reviewed_at": timestamp(), "decision": {"decision": "eligible", "relative_difficulty_basis_points": 0, "group": "normal", "is_confirmation": false, "evidence": []}}]}),
        ),
    ];
    for (path, value) in cases {
        fixture.write_value(path, &value);
    }

    assert!(
        store
            .load_frontier_suite_construction_plan(Path::new("bad/version.json"))
            .is_err()
    );
    assert!(
        store
            .load_frontier_suite_construction_plan(Path::new("bad/unknown.json"))
            .is_err()
    );
    assert!(
        store
            .load_frontier_suite_review_set(Path::new("bad/reviews.json"))
            .is_err()
    );
}

#[test]
fn loads_reject_absolute_parent_and_symlink_escape_paths() {
    let fixture = Fixture::new();
    let store = FileFrontierStore::new(&fixture.root).unwrap();
    let outside = Fixture::new();
    outside.write("plan.json", &plan());

    assert!(
        store
            .load_frontier_suite_construction_plan(&outside.root.join("plan.json"))
            .is_err()
    );
    assert!(
        store
            .load_frontier_suite_construction_plan(Path::new("../plan.json"))
            .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside.root, fixture.root.join("escape")).unwrap();
        assert!(
            store
                .load_frontier_suite_construction_plan(Path::new("escape/plan.json"))
                .is_err()
        );
    }
}

#[test]
fn inventory_and_ready_or_blocked_proposals_are_immutable_and_idempotent() {
    let fixture = Fixture::new();
    let mut store = FileFrontierStore::new(&fixture.root).unwrap();
    let inventory_path = Path::new("evidence/inventory.json");
    let proposal_path = Path::new("evidence/proposal.json");
    let inventory = inventory();
    let ready = ready_proposal();

    store
        .save_frontier_suite_inventory(inventory_path, &inventory)
        .unwrap();
    let inventory_bytes = fs::read(fixture.root.join(inventory_path)).unwrap();
    store
        .save_frontier_suite_inventory(inventory_path, &inventory)
        .unwrap();
    assert_eq!(
        fs::read(fixture.root.join(inventory_path)).unwrap(),
        inventory_bytes
    );
    let mut conflict = inventory.clone();
    conflict.generated_at = Timestamp("later".to_owned());
    assert!(
        store
            .save_frontier_suite_inventory(inventory_path, &conflict)
            .is_err()
    );
    assert_eq!(
        fs::read(fixture.root.join(inventory_path)).unwrap(),
        inventory_bytes
    );

    store
        .save_frontier_suite_proposal(proposal_path, &ready)
        .unwrap();
    let proposal_bytes = fs::read(fixture.root.join(proposal_path)).unwrap();
    store
        .save_frontier_suite_proposal(proposal_path, &ready)
        .unwrap();
    let blocked = blocked_proposal();
    assert!(
        store
            .save_frontier_suite_proposal(proposal_path, &blocked)
            .is_err()
    );
    assert_eq!(
        fs::read(fixture.root.join(proposal_path)).unwrap(),
        proposal_bytes
    );

    store
        .save_frontier_suite_proposal(Path::new("evidence/blocked.json"), &blocked)
        .unwrap();
    assert_eq!(
        store
            .load_frontier_suite_proposal(Path::new("evidence/blocked.json"))
            .unwrap(),
        blocked
    );
}

#[test]
fn ready_publication_replaces_suite_and_returns_exact_receipt() {
    let fixture = Fixture::new();
    let mut store = FileFrontierStore::new(&fixture.root).unwrap();
    let output = Path::new("suites/frontier.json");
    let proposal_path = Path::new("evidence/proposal.json");
    fixture.write_bytes(output, b"old suite\n");
    let proposal = ready_proposal();
    store
        .save_frontier_suite_proposal(proposal_path, &proposal)
        .unwrap();
    let proposal_bytes = fs::read(fixture.root.join(proposal_path)).unwrap();
    let publication = store
        .apply_frontier_suite_proposal(&proposal, output, &timestamp())
        .unwrap();
    let suite_bytes = fs::read(fixture.root.join(output)).unwrap();

    let shasum = Command::new("shasum")
        .args(["-a", "256"])
        .arg(fixture.root.join(proposal_path))
        .output()
        .unwrap();
    assert!(shasum.status.success());
    let shasum = String::from_utf8(shasum.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(publication.proposal_sha256, shasum);
    assert_eq!(publication.proposal_sha256, digest_bytes(&proposal_bytes));
    assert_eq!(publication.suite_sha256, digest_bytes(&suite_bytes));
    assert_eq!(publication.suite_path, output);
    assert_eq!(publication.published_at, timestamp());
    let suite: serde_json::Value = serde_json::from_slice(&suite_bytes).unwrap();
    assert_eq!(suite["version"], 1);
    assert_eq!(suite["tiers"].as_object().unwrap().len(), 5);
}

#[test]
fn blocked_and_malformed_proposals_never_change_suite_bytes() {
    let fixture = Fixture::new();
    let mut store = FileFrontierStore::new(&fixture.root).unwrap();
    let output = Path::new("suites/frontier.json");
    fixture.write_bytes(output, b"authoritative suite\n");
    let old = fs::read(fixture.root.join(output)).unwrap();

    let mut malformed = ready_proposal();
    malformed.version = 2;
    for proposal in [blocked_proposal(), malformed] {
        assert!(
            store
                .apply_frontier_suite_proposal(&proposal, output, &timestamp())
                .is_err()
        );
        assert_eq!(fs::read(fixture.root.join(output)).unwrap(), old);
    }
}

#[test]
fn stale_capacity_duplicate_weight_group_holdout_and_anchor_fail_before_write() {
    let fixture = Fixture::new();
    let mut store = FileFrontierStore::new(&fixture.root).unwrap();
    let output = Path::new("suites/frontier.json");
    fixture.write_bytes(output, b"authority\n");
    let old = fs::read(fixture.root.join(output)).unwrap();
    let base = ready_proposal();
    let mut invalid = Vec::new();

    let mut stale = base.clone();
    stale.review_set_sha256 = "stale".to_owned();
    invalid.push(stale);
    let mut capacity = base.clone();
    capacity
        .tier_capacity
        .get_mut(&Tier::T1)
        .unwrap()
        .accepted_unique_cases = 31;
    invalid.push(capacity);
    let mut duplicate = base.clone();
    duplicate.proposed_tiers.get_mut(&Tier::T2).unwrap().cases[0] =
        duplicate.proposed_tiers[&Tier::T1].cases[0].clone();
    invalid.push(duplicate);
    let mut weight = base.clone();
    weight
        .proposed_tiers
        .get_mut(&Tier::T3)
        .unwrap()
        .group_weights_basis_points
        .insert(FrontierCaseGroup::Normal, 3_999);
    invalid.push(weight);
    let mut missing_group = base.clone();
    missing_group
        .proposed_tiers
        .get_mut(&Tier::T1)
        .unwrap()
        .cases
        .iter_mut()
        .filter(|case| case.group == FrontierCaseGroup::Critical)
        .for_each(|case| case.group = FrontierCaseGroup::Adversarial);
    invalid.push(missing_group);
    let mut holdout = base.clone();
    let confirmation = holdout
        .proposed_tiers
        .values()
        .flat_map(|tier| &tier.cases)
        .find(|case| case.is_confirmation)
        .unwrap()
        .clone();
    holdout
        .holdout_cases
        .retain(|key| key.case != confirmation.case);
    invalid.push(holdout);
    let mut anchor = base;
    anchor.calibration_anchors = vec![reference_key(&anchor.proposed_tiers[&Tier::T5].cases[0])];
    invalid.push(anchor);

    for proposal in invalid {
        assert!(
            store
                .apply_frontier_suite_proposal(&proposal, output, &timestamp())
                .is_err()
        );
        assert_eq!(fs::read(fixture.root.join(output)).unwrap(), old);
    }
}

#[test]
fn unsafe_publication_path_does_not_change_repository_or_outside_bytes() {
    let fixture = Fixture::new();
    let outside = Fixture::new();
    outside.write_bytes(Path::new("suite.json"), b"outside\n");
    let old = fs::read(outside.root.join("suite.json")).unwrap();
    let mut store = FileFrontierStore::new(&fixture.root).unwrap();

    assert!(
        store
            .apply_frontier_suite_proposal(
                &ready_proposal(),
                &outside.root.join("suite.json"),
                &timestamp()
            )
            .is_err()
    );
    assert!(
        store
            .apply_frontier_suite_proposal(
                &ready_proposal(),
                Path::new("../suite.json"),
                &timestamp()
            )
            .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            outside.root.join("suite.json"),
            fixture.root.join("suite-link"),
        )
        .unwrap();
        assert!(
            store
                .apply_frontier_suite_proposal(
                    &ready_proposal(),
                    Path::new("suite-link"),
                    &timestamp()
                )
                .is_err()
        );
    }
    assert_eq!(fs::read(outside.root.join("suite.json")).unwrap(), old);
}

#[test]
fn interrupted_inventory_and_proposal_writes_leave_no_authority_or_temporary_file() {
    for (failure, path, is_inventory) in [
        (
            FrontierFailurePoint::Inventory,
            "evidence/inventory.json",
            true,
        ),
        (
            FrontierFailurePoint::InventoryAfterLink,
            "evidence/inventory.json",
            true,
        ),
        (
            FrontierFailurePoint::Proposal,
            "evidence/proposal.json",
            false,
        ),
        (
            FrontierFailurePoint::ProposalAfterLink,
            "evidence/proposal.json",
            false,
        ),
    ] {
        let fixture = Fixture::new();
        let mut store = FileFrontierStore::with_failure(&fixture.root, failure).unwrap();
        let result = if is_inventory {
            store.save_frontier_suite_inventory(Path::new(path), &inventory())
        } else {
            store.save_frontier_suite_proposal(Path::new(path), &ready_proposal())
        };
        assert!(result.is_err());
        assert!(!fixture.root.join(path).exists());
        assert_no_temporary_files(&fixture.root);

        if is_inventory {
            store
                .save_frontier_suite_inventory(Path::new(path), &inventory())
                .unwrap();
        } else {
            store
                .save_frontier_suite_proposal(Path::new(path), &ready_proposal())
                .unwrap();
        }
        assert!(fixture.root.join(path).is_file());
        assert_no_temporary_files(&fixture.root);
    }
}

#[test]
fn interrupted_suite_replacement_preserves_old_bytes_and_recovers_cleanly() {
    let output = Path::new("suites/frontier.json");
    for failure in [
        FrontierFailurePoint::Suite,
        FrontierFailurePoint::SuiteAfterRename,
    ] {
        let fixture = Fixture::new();
        fixture.write_bytes(output, b"old authority\n");
        let old = fs::read(fixture.root.join(output)).unwrap();
        let mut store = FileFrontierStore::with_failure(&fixture.root, failure).unwrap();

        assert!(
            store
                .apply_frontier_suite_proposal(&ready_proposal(), output, &timestamp())
                .is_err()
        );
        assert_eq!(fs::read(fixture.root.join(output)).unwrap(), old);
        assert_no_temporary_files(&fixture.root);

        store
            .apply_frontier_suite_proposal(&ready_proposal(), output, &timestamp())
            .unwrap();
        assert_ne!(fs::read(fixture.root.join(output)).unwrap(), old);
        assert_no_temporary_files(&fixture.root);
    }

    let fixture = Fixture::new();
    let mut store =
        FileFrontierStore::with_failure(&fixture.root, FrontierFailurePoint::SuiteAfterRename)
            .unwrap();
    assert!(
        store
            .apply_frontier_suite_proposal(&ready_proposal(), output, &timestamp())
            .is_err()
    );
    assert!(!fixture.root.join(output).exists());
    assert_no_temporary_files(&fixture.root);
    store
        .apply_frontier_suite_proposal(&ready_proposal(), output, &timestamp())
        .unwrap();
    assert!(fixture.root.join(output).is_file());
    assert_no_temporary_files(&fixture.root);
}

#[test]
fn rollback_result_preserves_or_combines_errors_without_panicking() {
    let initiating_error =
        SkillEvalError::InvalidConfiguration("injected post-authority failure".to_owned());
    assert_eq!(
        rollback_result("frontier suite authority", initiating_error.clone(), Ok(())),
        initiating_error
    );

    let error = rollback_result(
        "frontier suite authority",
        SkillEvalError::InvalidConfiguration("injected post-authority failure".to_owned()),
        Err(SkillEvalError::InvalidConfiguration(
            "injected rollback failure".to_owned(),
        )),
    );
    let message = match error {
        SkillEvalError::InvalidConfiguration(message) => message,
        _ => String::new(),
    };
    assert!(message.contains("frontier suite authority rollback failed"));
    assert!(message.contains("injected rollback failure"));
    assert!(message.contains("injected post-authority failure"));
}

#[test]
fn strict_evidence_validation_rejects_unsorted_identity_digest_and_policy_drift() {
    let fixture = Fixture::new();
    let store = FileFrontierStore::new(&fixture.root).unwrap();
    let mut unsorted = inventory();
    unsorted.cases.reverse();
    fixture.write("bad/inventory.json", &unsorted);
    let mut proposal = ready_proposal();
    proposal.inventory_sha256 = "A".repeat(64);
    fixture.write("bad/proposal.json", &proposal);
    let mut invalid_plan = plan();
    invalid_plan.policy.minimum_reviewers_per_case = 1;
    fixture.write("bad/plan.json", &invalid_plan);

    assert!(
        store
            .load_frontier_suite_inventory(Path::new("bad/inventory.json"))
            .is_err()
    );
    assert!(
        store
            .load_frontier_suite_proposal(Path::new("bad/proposal.json"))
            .is_err()
    );
    assert!(
        store
            .load_frontier_suite_construction_plan(Path::new("bad/plan.json"))
            .is_err()
    );
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let sequence = ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "skill-eval-frontier-suite-store-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write<T: Serialize>(&self, path: &str, value: &T) {
        let mut bytes = serde_json::to_vec_pretty(value).unwrap();
        bytes.push(b'\n');
        self.write_bytes(Path::new(path), &bytes);
    }

    fn write_value(&self, path: &str, value: &serde_json::Value) {
        self.write(path, value);
    }

    fn write_bytes(&self, path: &Path, bytes: &[u8]) {
        let destination = self.root.join(path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, bytes).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn plan() -> FrontierSuiteConstructionPlan {
    FrontierSuiteConstructionPlan {
        version: 1,
        artifact_roots: vec![PathBuf::from("skills/a")],
        policy: policy(),
    }
}

fn policy() -> FrontierSuiteConstructionPolicy {
    FrontierSuiteConstructionPolicy {
        required_tiers: vec![Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5],
        minimum_unique_cases_per_tier: 30,
        minimum_reviewers_per_case: 2,
        group_weights_basis_points: weights(),
        is_unanimous_eligibility_required: true,
        is_cross_tier_reuse_allowed: false,
        is_calibration_anchor_counted_toward_minimum: false,
    }
}

fn inventory() -> FrontierSuiteInventory {
    FrontierSuiteInventory {
        version: 1,
        generated_at: timestamp(),
        cases: (0..2)
            .map(|index| FrontierCaseInventoryEntry {
                key: key(index),
                drive: CaseDrive::ExistingHarness {
                    command: CommandDefinition {
                        program: "true".to_owned(),
                        arguments: Vec::new(),
                        working_directory: None,
                    },
                },
                is_holdout: index == 0,
            })
            .collect(),
    }
}

fn reviews() -> FrontierSuiteReviewSet {
    FrontierSuiteReviewSet {
        version: 1,
        inventory_sha256: "1".repeat(64),
        records: ["panel-a", "panel-b"]
            .into_iter()
            .map(|reviewer| FrontierCaseReviewRecord {
                key: key(0),
                reviewer: reviewer.to_owned(),
                reviewed_at: timestamp(),
                decision: FrontierCaseReviewDecision::Eligible {
                    relative_difficulty_basis_points: 1,
                    group: FrontierCaseGroup::Normal,
                    is_confirmation: true,
                    evidence: vec!["review evidence".to_owned()],
                },
            })
            .collect(),
    }
}

fn ready_proposal() -> FrontierSuiteProposal {
    proposal(150, FrontierSuiteProposalStatus::Ready)
}

fn blocked_proposal() -> FrontierSuiteProposal {
    proposal(128, FrontierSuiteProposalStatus::Blocked)
}

fn proposal(count: usize, status: FrontierSuiteProposalStatus) -> FrontierSuiteProposal {
    let mut proposed_tiers = BTreeMap::new();
    let mut tier_capacity = BTreeMap::new();
    let tiers = [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5];
    let mut cursor = 0;
    for (index, tier) in tiers.into_iter().enumerate() {
        let end = if index == 4 {
            count
        } else {
            (cursor + 30).min(count)
        };
        let cases = (cursor..end).map(reference).collect::<Vec<_>>();
        cursor = end;
        let accepted = u16::try_from(cases.len()).unwrap();
        let shortfall = 30_u16.saturating_sub(accepted);
        let groups = cases
            .iter()
            .map(|case| case.group)
            .collect::<std::collections::BTreeSet<_>>();
        let is_complete = shortfall == 0 && groups.len() == 4;
        proposed_tiers.insert(
            tier,
            FrontierTierSuite {
                group_weights_basis_points: weights(),
                cases,
            },
        );
        tier_capacity.insert(
            tier,
            FrontierTierCapacity {
                required_unique_cases: 30,
                accepted_unique_cases: accepted,
                shortfall,
                duplicate_cases: 0,
                rejected_cases: 0,
                is_complete,
            },
        );
    }
    let holdout_cases = (0..count)
        .filter(|index| index.is_multiple_of(15))
        .map(key)
        .collect();
    FrontierSuiteProposal {
        version: 1,
        inventory_sha256: "1".repeat(64),
        review_set_sha256: "2".repeat(64),
        policy: policy(),
        proposed_tiers,
        calibration_anchors: Vec::new(),
        holdout_cases,
        tier_capacity,
        status,
    }
}

fn reference(index: usize) -> FrontierCaseReference {
    FrontierCaseReference {
        artifact_path: PathBuf::from("skills/a"),
        artifact_revision: "revision-1".to_owned(),
        case: CaseId(format!("case-{index:03}")),
        group: match index % 4 {
            0 => FrontierCaseGroup::Normal,
            1 => FrontierCaseGroup::Edge,
            2 => FrontierCaseGroup::Adversarial,
            _ => FrontierCaseGroup::Critical,
        },
        is_confirmation: index.is_multiple_of(15),
    }
}

fn key(index: usize) -> FrontierCaseKey {
    FrontierCaseKey {
        artifact_path: PathBuf::from("skills/a"),
        artifact_revision: "revision-1".to_owned(),
        case: CaseId(format!("case-{index:03}")),
    }
}

fn reference_key(reference: &FrontierCaseReference) -> FrontierCaseKey {
    FrontierCaseKey {
        artifact_path: reference.artifact_path.clone(),
        artifact_revision: reference.artifact_revision.clone(),
        case: reference.case.clone(),
    }
}

fn weights() -> BTreeMap<FrontierCaseGroup, u16> {
    BTreeMap::from([
        (FrontierCaseGroup::Normal, 4_000),
        (FrontierCaseGroup::Edge, 2_000),
        (FrontierCaseGroup::Adversarial, 2_000),
        (FrontierCaseGroup::Critical, 2_000),
    ])
}

fn timestamp() -> Timestamp {
    Timestamp("2026-08-27T15:30:00-0400".to_owned())
}

fn digest_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn assert_no_temporary_files(root: &Path) {
    fn visit(path: &Path) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(&entry.path());
            } else {
                assert!(!entry.file_name().to_string_lossy().ends_with(".tmp"));
            }
        }
    }
    visit(root);
}
