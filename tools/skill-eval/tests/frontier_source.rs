#![expect(
    dead_code,
    reason = "the test imports private production modules to exercise crate-private suite construction"
)]
#![expect(
    clippy::large_enum_variant,
    reason = "the test imports frozen production model declarations without changing their shapes"
)]

#[path = "../src/frontier_source.rs"]
mod frontier_source;
#[path = "../src/model.rs"]
mod model;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use frontier_source::{
    build_frontier_suite_inventory, build_frontier_suite_proposal,
    frontier_suite_from_ready_proposal, validate_frontier_suite_review_set,
};
use model::{
    ArtifactDefinition, ArtifactKind, ArtifactName, CaseDefinition, CaseDrive, CaseId,
    CommandDefinition, ExecutionDefinition, FrontierCaseGroup, FrontierCaseKey,
    FrontierCaseReviewDecision, FrontierCaseReviewRecord, FrontierSuiteConstructionPlan,
    FrontierSuiteConstructionPolicy, FrontierSuiteInventory, FrontierSuiteProposalStatus,
    FrontierSuiteReviewSet, SkillEvalError, Tier, Timestamp,
};
use sha2::{Digest, Sha256};

const ROOT: &str = "tools/skill-eval/tests/fixtures/frontier-source";

#[test]
fn inventory_is_deterministic_and_source_revision_bound() {
    let plan = plan();
    let mut artifact = artifact(3);
    artifact.cases.reverse();

    let first = build_frontier_suite_inventory(&plan, &[artifact.clone()], &timestamp()).unwrap();
    let second = build_frontier_suite_inventory(&plan, &[artifact], &timestamp()).unwrap();

    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert!(first.cases.windows(2).all(|pair| pair[0].key < pair[1].key));
    assert!(
        first
            .cases
            .iter()
            .all(|entry| entry.key.artifact_revision == "revision-1")
    );
}

#[test]
fn inventory_maps_loaded_absolute_roots_to_repository_relative_keys() {
    let plan = plan();
    let loaded_root = PathBuf::from("/repository").join(ROOT);
    let mut artifact = artifact(2);
    artifact.root = loaded_root.clone();
    for case in &mut artifact.cases {
        if let CaseDrive::Fixture { source, .. } = &mut case.execution.drive {
            *source = loaded_root.join("input.txt");
        }
    }

    let inventory = build_frontier_suite_inventory(&plan, &[artifact], &timestamp()).unwrap();

    assert!(
        inventory
            .cases
            .iter()
            .all(|entry| entry.key.artifact_path == Path::new(ROOT))
    );
    assert!(inventory.cases.iter().all(|entry| match &entry.drive {
        CaseDrive::Fixture { source, .. } => source.is_absolute(),
        _ => false,
    }));
}

#[test]
fn inventory_rejects_duplicate_stale_unsafe_missing_and_unsupported_inputs() {
    let plan = plan();
    let artifact = artifact(2);
    assert_named_error(
        build_frontier_suite_inventory(&plan, &[artifact.clone(), artifact.clone()], &timestamp()),
        "coverage",
    );

    let mut duplicate_case = artifact.clone();
    duplicate_case.cases.push(duplicate_case.cases[0].clone());
    assert_named_error(
        build_frontier_suite_inventory(&plan, &[duplicate_case], &timestamp()),
        "case tools/skill-eval/tests/fixtures/frontier-source@revision-1:case-000 is duplicate",
    );

    let mut stale = artifact.clone();
    stale.revision.clear();
    assert_named_error(
        build_frontier_suite_inventory(&plan, &[stale], &timestamp()),
        "revision",
    );

    let mut unsafe_plan = plan.clone();
    unsafe_plan.artifact_roots[0] = PathBuf::from("../escape");
    assert_named_error(
        build_frontier_suite_inventory(&unsafe_plan, std::slice::from_ref(&artifact), &timestamp()),
        "safe repository-relative",
    );

    let mut missing_fixture = artifact.clone();
    missing_fixture.cases[0].execution.drive = CaseDrive::Fixture {
        source: PathBuf::new(),
        verify_commands: Vec::new(),
    };
    assert_named_error(
        build_frontier_suite_inventory(&plan, &[missing_fixture], &timestamp()),
        "missing fixture",
    );

    let mut unsupported = artifact;
    unsupported.cases[0].execution.drive = CaseDrive::Response;
    assert_named_error(
        build_frontier_suite_inventory(&plan, &[unsupported], &timestamp()),
        "unsupported response drive",
    );
}

#[test]
fn inventory_rejects_zero_timeout_and_invalid_commands() {
    let plan = plan();

    let mut zero_timeout = artifact(1);
    zero_timeout.cases[0].execution.timeout_seconds = 0;
    assert_named_error(
        build_frontier_suite_inventory(&plan, &[zero_timeout], &timestamp()),
        "zero execution timeout",
    );

    let mut empty_program = artifact(1);
    empty_program.cases[0].execution.drive = CaseDrive::ExistingHarness {
        command: CommandDefinition {
            program: String::new(),
            arguments: Vec::new(),
            working_directory: None,
        },
    };
    assert_named_error(
        build_frontier_suite_inventory(&plan, &[empty_program], &timestamp()),
        "invalid executable",
    );

    let mut unsafe_directory = artifact(1);
    let CaseDrive::Fixture {
        verify_commands, ..
    } = &mut unsafe_directory.cases[0].execution.drive
    else {
        unreachable!();
    };
    verify_commands[0].working_directory = Some(PathBuf::from("../escape"));
    assert_named_error(
        build_frontier_suite_inventory(&plan, &[unsafe_directory], &timestamp()),
        "command working directory is unsafe",
    );
}

#[test]
fn plan_policy_and_artifact_coverage_are_frozen() {
    let artifact = artifact(1);
    let mut invalid = plan();
    invalid.version = 2;
    assert_named_error(
        build_frontier_suite_inventory(&invalid, std::slice::from_ref(&artifact), &timestamp()),
        "version",
    );

    let mut invalid = plan();
    invalid.policy.required_tiers.swap(0, 1);
    assert_named_error(
        build_frontier_suite_inventory(&invalid, std::slice::from_ref(&artifact), &timestamp()),
        "T1 through T5",
    );

    let mut invalid = plan();
    invalid.policy.is_cross_tier_reuse_allowed = true;
    assert_named_error(
        build_frontier_suite_inventory(&invalid, std::slice::from_ref(&artifact), &timestamp()),
        "forbid cross-tier reuse",
    );

    let mut foreign = artifact;
    foreign.root = PathBuf::from("skills/foreign");
    assert_named_error(
        build_frontier_suite_inventory(&plan(), &[foreign], &timestamp()),
        "foreign",
    );
}

#[test]
fn complete_review_set_requires_two_independent_records() {
    let (plan, inventory, reviews) = reviewed_bank(12);
    validate_frontier_suite_review_set(&plan, &inventory, &reviews).unwrap();

    let proposal = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();
    let expected = inventory
        .cases
        .iter()
        .filter(|entry| entry.is_holdout)
        .map(|entry| entry.key.clone())
        .collect::<Vec<_>>();
    assert_eq!(proposal.holdout_cases, expected);

    let mut missing = reviews.clone();
    missing.records.pop();
    assert_named_error(
        validate_frontier_suite_review_set(&plan, &inventory, &missing),
        "fewer than 2 independent reviewers",
    );

    let mut duplicate = reviews.clone();
    duplicate.records[1].reviewer = duplicate.records[0].reviewer.clone();
    assert_named_error(
        validate_frontier_suite_review_set(&plan, &inventory, &duplicate),
        "repeats reviewer",
    );

    let mut evidence_free = reviews.clone();
    set_evidence(&mut evidence_free.records[0].decision, Vec::new());
    assert_named_error(
        validate_frontier_suite_review_set(&plan, &inventory, &evidence_free),
        "empty review evidence",
    );

    let mut invalid_confirmation = reviews;
    let record = invalid_confirmation
        .records
        .iter_mut()
        .find(|record| {
            !inventory
                .cases
                .iter()
                .find(|entry| entry.key == record.key)
                .unwrap()
                .is_holdout
        })
        .unwrap();
    set_confirmation(&mut record.decision, true);
    assert_named_error(
        validate_frontier_suite_review_set(&plan, &inventory, &invalid_confirmation),
        "not a holdout",
    );
}

#[test]
fn review_set_rejects_foreign_stale_digest_and_invalid_difficulty() {
    let (plan, inventory, reviews) = reviewed_bank(4);

    let mut foreign = reviews.clone();
    foreign.records[0].key.case = CaseId("foreign".to_owned());
    assert_named_error(
        validate_frontier_suite_review_set(&plan, &inventory, &foreign),
        "foreign or stale",
    );

    let mut stale = reviews.clone();
    stale.inventory_sha256 = "0".repeat(64);
    assert_named_error(
        validate_frontier_suite_review_set(&plan, &inventory, &stale),
        "digest differs",
    );

    let mut invalid_score = reviews;
    set_difficulty(&mut invalid_score.records[0].decision, 0);
    assert_named_error(
        validate_frontier_suite_review_set(&plan, &inventory, &invalid_score),
        "invalid difficulty",
    );
}

#[test]
fn unanimous_rejections_are_preserved_in_capacity() {
    let (plan, inventory, mut reviews) = reviewed_bank(150);
    let rejected_key = inventory.cases[149].key.clone();
    for record in reviews
        .records
        .iter_mut()
        .filter(|record| record.key == rejected_key)
    {
        record.decision = FrontierCaseReviewDecision::Rejected {
            reason: model::FrontierCaseRejectionReason::AtOrBelowTier,
            evidence: vec![format!("{} rejection evidence", record.reviewer)],
        };
    }

    validate_frontier_suite_review_set(&plan, &inventory, &reviews).unwrap();
    let proposal = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();
    assert_eq!(proposal.status, FrontierSuiteProposalStatus::Blocked);
    assert!(
        proposal
            .tier_capacity
            .values()
            .all(|capacity| capacity.rejected_cases == 1)
    );
}

#[test]
fn reviewer_disagreement_fails_closed() {
    let (plan, inventory, reviews) = reviewed_bank(4);

    let mut eligibility = reviews.clone();
    eligibility.records[1].decision = FrontierCaseReviewDecision::Rejected {
        reason: model::FrontierCaseRejectionReason::AtOrBelowTier,
        evidence: vec!["review evidence".to_owned()],
    };
    assert_named_error(
        validate_frontier_suite_review_set(&plan, &inventory, &eligibility),
        "reviewers disagree",
    );

    let mut confirmation = reviews.clone();
    let holdout_key = inventory
        .cases
        .iter()
        .find(|entry| entry.is_holdout)
        .unwrap()
        .key
        .clone();
    for record in confirmation
        .records
        .iter_mut()
        .filter(|record| record.key == holdout_key)
    {
        let value = record.reviewer == "panel-a";
        set_confirmation(&mut record.decision, value);
    }
    assert_named_error(
        validate_frontier_suite_review_set(&plan, &inventory, &confirmation),
        "reviewers disagree",
    );

    let mut different_scores = reviews;
    set_difficulty(&mut different_scores.records[0].decision, 9_999);
    validate_frontier_suite_review_set(&plan, &inventory, &different_scores).unwrap();
}

#[test]
fn ready_proposal_contains_five_disjoint_complete_tiers() {
    let (plan, inventory, reviews) = reviewed_bank(150);
    let first = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();
    let second = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.status, FrontierSuiteProposalStatus::Ready);
    assert_eq!(first.proposed_tiers.len(), 5);
    assert_eq!(first.holdout_cases.len(), 10);
    assert!(first.holdout_cases.windows(2).all(|pair| pair[0] < pair[1]));
    for tier in [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5] {
        let suite = &first.proposed_tiers[&tier];
        assert_eq!(suite.cases.len(), 30);
        assert_eq!(suite.group_weights_basis_points, weights());
        let capacity = &first.tier_capacity[&tier];
        assert_eq!(capacity.accepted_unique_cases, 30);
        assert_eq!(capacity.shortfall, 0);
        assert_eq!(capacity.duplicate_cases, 0);
        assert!(capacity.is_complete);
    }

    let suite = frontier_suite_from_ready_proposal(&first).unwrap();
    let mut keys = suite
        .tiers
        .values()
        .flat_map(|tier| tier.cases.iter().map(case_key))
        .collect::<Vec<_>>();
    let count = keys.len();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), count);
    assert!(
        suite
            .tiers
            .values()
            .flat_map(|tier| &tier.cases)
            .all(|case| {
                !case.is_confirmation || first.holdout_cases.binary_search(&case_key(case)).is_ok()
            })
    );
}

#[test]
fn full_count_tier_missing_group_rebalances_and_publishes() {
    let (plan, inventory, mut reviews) = reviewed_bank(150);
    for record in &mut reviews.records {
        if case_index(&record.key) < 30
            && matches!(
                record.decision,
                FrontierCaseReviewDecision::Eligible {
                    group: FrontierCaseGroup::Critical,
                    ..
                }
            )
        {
            set_group(&mut record.decision, FrontierCaseGroup::Adversarial);
        }
    }

    let proposal = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();

    assert_eq!(proposal.status, FrontierSuiteProposalStatus::Ready);
    assert_eq!(proposal.proposed_tiers[&Tier::T1].cases.len(), 30);
    assert!(
        proposal.proposed_tiers[&Tier::T1]
            .cases
            .iter()
            .any(|case| case.case.0 == "case-031")
    );
    assert!(
        proposal.proposed_tiers[&Tier::T2]
            .cases
            .iter()
            .any(|case| case.case.0 == "case-029")
    );
    assert_complete_tiers(&proposal, [30, 30, 30, 30, 30]);
    assert_proposal_keys_match_inventory(&proposal, &inventory);
    frontier_suite_from_ready_proposal(&proposal).unwrap();
}

#[test]
fn current_shaped_155_tail_missing_normal_rebalances_deterministically() {
    let (plan, inventory, mut reviews) = reviewed_bank(155);
    for record in &mut reviews.records {
        if case_index(&record.key) >= 120
            && matches!(
                record.decision,
                FrontierCaseReviewDecision::Eligible {
                    group: FrontierCaseGroup::Normal,
                    ..
                }
            )
        {
            set_group(&mut record.decision, FrontierCaseGroup::Edge);
        }
    }

    let first = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();
    let second = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.status, FrontierSuiteProposalStatus::Ready);
    assert_complete_tiers(&first, [30, 30, 30, 30, 35]);
    assert_proposal_keys_match_inventory(&first, &inventory);
    frontier_suite_from_ready_proposal(&first).unwrap();
}

#[test]
fn globally_missing_group_is_blocked_and_forged_ready_cannot_publish() {
    let (plan, inventory, mut reviews) = reviewed_bank(150);
    for record in &mut reviews.records {
        if matches!(
            record.decision,
            FrontierCaseReviewDecision::Eligible {
                group: FrontierCaseGroup::Normal,
                ..
            }
        ) {
            set_group(&mut record.decision, FrontierCaseGroup::Edge);
        }
    }

    let mut proposal = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();

    assert_eq!(proposal.status, FrontierSuiteProposalStatus::Blocked);
    assert!(
        proposal
            .tier_capacity
            .values()
            .all(|capacity| !capacity.is_complete)
    );
    assert_proposal_keys_match_inventory(&proposal, &inventory);
    proposal.status = FrontierSuiteProposalStatus::Ready;
    assert_named_error(
        frontier_suite_from_ready_proposal(&proposal),
        "does not contain every case group",
    );
}

#[test]
fn proposal_ranks_complete_reviewer_values_then_exact_key() {
    let (plan, inventory, mut reviews) = reviewed_bank(150);
    for record in &mut reviews.records {
        let index = case_index(&record.key);
        let score = if record.reviewer == "panel-a" {
            150_u16 - index
        } else {
            index + 1
        };
        set_difficulty(&mut record.decision, score);
    }
    let proposal = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();
    assert_complete_tiers(&proposal, [30, 30, 30, 30, 30]);
    let ordered = proposal
        .proposed_tiers
        .values()
        .flat_map(|suite| suite.cases.iter().map(|case| case.case.0.clone()))
        .collect::<Vec<_>>();
    let expected = (0..75)
        .flat_map(|index| {
            [
                format!("case-{index:03}"),
                format!("case-{:03}", 149 - index),
            ]
        })
        .collect::<Vec<_>>();
    assert_eq!(ordered, expected);
}

#[test]
fn blocked_short_capacity_is_exact_and_cannot_publish() {
    let (plan, inventory, reviews) = reviewed_bank(128);
    let proposal = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();

    assert_eq!(proposal.status, FrontierSuiteProposalStatus::Blocked);
    assert_eq!(proposal.tier_capacity[&Tier::T5].accepted_unique_cases, 8);
    assert_eq!(proposal.tier_capacity[&Tier::T5].shortfall, 22);
    assert_named_error(frontier_suite_from_ready_proposal(&proposal), "blocked");
}

#[test]
fn lower_tier_reuse_and_anchors_do_not_count() {
    let (plan, inventory, reviews) = reviewed_bank(150);
    let proposal = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();

    let mut duplicate = proposal.clone();
    duplicate.proposed_tiers.get_mut(&Tier::T4).unwrap().cases[0] =
        duplicate.proposed_tiers[&Tier::T2].cases[0].clone();
    assert_named_error(
        frontier_suite_from_ready_proposal(&duplicate),
        "reuses case",
    );

    let mut anchor = proposal;
    let key = case_key(&anchor.proposed_tiers[&Tier::T4].cases[0]);
    anchor.calibration_anchors.push(key);
    assert_named_error(
        frontier_suite_from_ready_proposal(&anchor),
        "calibration anchor",
    );
}

#[test]
fn publication_rejects_forged_capacity_duplicate_bad_weight_and_anchor_overlap() {
    let (plan, inventory, reviews) = reviewed_bank(150);
    let proposal = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();

    let mut forged = proposal.clone();
    forged
        .tier_capacity
        .get_mut(&Tier::T1)
        .unwrap()
        .accepted_unique_cases = 31;
    assert_named_error(
        frontier_suite_from_ready_proposal(&forged),
        "capacity is forged",
    );

    let mut rejected = proposal.clone();
    rejected
        .tier_capacity
        .get_mut(&Tier::T5)
        .unwrap()
        .rejected_cases = 1;
    assert_named_error(
        frontier_suite_from_ready_proposal(&rejected),
        "capacity is forged",
    );

    let mut duplicate = proposal.clone();
    duplicate.proposed_tiers.get_mut(&Tier::T2).unwrap().cases[0] =
        duplicate.proposed_tiers[&Tier::T1].cases[0].clone();
    assert_named_error(
        frontier_suite_from_ready_proposal(&duplicate),
        "reuses case",
    );

    let mut weight = proposal.clone();
    weight
        .proposed_tiers
        .get_mut(&Tier::T3)
        .unwrap()
        .group_weights_basis_points
        .insert(FrontierCaseGroup::Normal, 3_999);
    assert_named_error(frontier_suite_from_ready_proposal(&weight), "10000");

    let mut anchor = proposal;
    anchor.calibration_anchors = vec![case_key(&anchor.proposed_tiers[&Tier::T5].cases[0])];
    assert_named_error(
        frontier_suite_from_ready_proposal(&anchor),
        "calibration anchor",
    );
}

#[test]
fn publication_rejects_malformed_digests_paths_and_case_identities() {
    let (plan, inventory, reviews) = reviewed_bank(150);
    let proposal = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();

    let mut inventory_digest = proposal.clone();
    inventory_digest.inventory_sha256 = "not-a-digest".to_owned();
    assert_named_error(
        frontier_suite_from_ready_proposal(&inventory_digest),
        "proposal inventory digest is invalid",
    );

    let mut review_digest = proposal.clone();
    review_digest.review_set_sha256 = "A".repeat(64);
    assert_named_error(
        frontier_suite_from_ready_proposal(&review_digest),
        "proposal review set digest is invalid",
    );

    let mut unsafe_path = proposal.clone();
    unsafe_path.proposed_tiers.get_mut(&Tier::T1).unwrap().cases[0].artifact_path =
        PathBuf::from("../escape");
    assert_named_error(
        frontier_suite_from_ready_proposal(&unsafe_path),
        "proposal artifact path must be a safe repository-relative path",
    );

    let mut empty_revision = proposal.clone();
    empty_revision
        .proposed_tiers
        .get_mut(&Tier::T1)
        .unwrap()
        .cases[0]
        .artifact_revision
        .clear();
    assert_named_error(
        frontier_suite_from_ready_proposal(&empty_revision),
        "stale case identity",
    );

    let mut empty_case = proposal;
    empty_case.proposed_tiers.get_mut(&Tier::T1).unwrap().cases[0]
        .case
        .0
        .clear();
    assert_named_error(
        frontier_suite_from_ready_proposal(&empty_case),
        "stale case identity",
    );
}

#[test]
fn publication_rejects_unsorted_duplicate_holdouts_and_missing_confirmation_membership() {
    let (plan, inventory, reviews) = reviewed_bank(150);
    let proposal = build_frontier_suite_proposal(&plan, &inventory, &reviews).unwrap();

    let mut unsorted = proposal.clone();
    unsorted.holdout_cases.swap(0, 1);
    assert_named_error(
        frontier_suite_from_ready_proposal(&unsorted),
        "strictly sorted and unique",
    );

    let mut duplicate = proposal.clone();
    duplicate
        .holdout_cases
        .insert(1, duplicate.holdout_cases[0].clone());
    assert_named_error(
        frontier_suite_from_ready_proposal(&duplicate),
        "strictly sorted and unique",
    );

    let mut missing = proposal;
    let confirmation = missing
        .proposed_tiers
        .values()
        .flat_map(|tier| &tier.cases)
        .find(|case| case.is_confirmation)
        .map(case_key)
        .unwrap();
    missing.holdout_cases.retain(|key| key != &confirmation);
    assert_named_error(
        frontier_suite_from_ready_proposal(&missing),
        "absent from holdout cases",
    );
}

fn assert_complete_tiers(proposal: &model::FrontierSuiteProposal, expected_counts: [usize; 5]) {
    for (tier, expected_count) in [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5]
        .into_iter()
        .zip(expected_counts)
    {
        let suite = &proposal.proposed_tiers[&tier];
        assert_eq!(suite.cases.len(), expected_count);
        assert_eq!(
            suite
                .cases
                .iter()
                .map(|case| case.group)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                FrontierCaseGroup::Normal,
                FrontierCaseGroup::Edge,
                FrontierCaseGroup::Adversarial,
                FrontierCaseGroup::Critical,
            ])
        );
        let capacity = &proposal.tier_capacity[&tier];
        assert_eq!(capacity.accepted_unique_cases, expected_count as u16);
        assert_eq!(capacity.shortfall, 0);
        assert!(capacity.is_complete);
    }
}

fn assert_proposal_keys_match_inventory(
    proposal: &model::FrontierSuiteProposal,
    inventory: &FrontierSuiteInventory,
) {
    let proposed_keys = proposal
        .proposed_tiers
        .values()
        .flat_map(|tier| tier.cases.iter().map(case_key))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        proposed_keys.len(),
        proposal
            .proposed_tiers
            .values()
            .map(|tier| tier.cases.len())
            .sum::<usize>()
    );
    assert_eq!(
        proposed_keys,
        inventory
            .cases
            .iter()
            .map(|entry| entry.key.clone())
            .collect::<BTreeSet<_>>()
    );
}

fn reviewed_bank(
    count: usize,
) -> (
    FrontierSuiteConstructionPlan,
    FrontierSuiteInventory,
    FrontierSuiteReviewSet,
) {
    let plan = plan();
    let inventory =
        build_frontier_suite_inventory(&plan, &[artifact(count)], &timestamp()).unwrap();
    let inventory_sha256 = digest(&inventory);
    let mut records = Vec::new();
    for (index, entry) in inventory.cases.iter().enumerate() {
        let is_confirmation = entry.is_holdout;
        let group = group(index);
        for (reviewer, offset) in [("panel-a", 0_u16), ("panel-b", 1_u16)] {
            records.push(FrontierCaseReviewRecord {
                key: entry.key.clone(),
                reviewer: reviewer.to_owned(),
                reviewed_at: timestamp(),
                decision: FrontierCaseReviewDecision::Eligible {
                    relative_difficulty_basis_points: u16::try_from(index).unwrap() + offset + 1,
                    group,
                    is_confirmation,
                    evidence: vec![format!("{reviewer} executable evidence")],
                },
            });
        }
    }
    let reviews = FrontierSuiteReviewSet {
        version: 1,
        inventory_sha256,
        records,
    };
    (plan, inventory, reviews)
}

fn plan() -> FrontierSuiteConstructionPlan {
    FrontierSuiteConstructionPlan {
        version: 1,
        artifact_roots: vec![PathBuf::from(ROOT)],
        policy: FrontierSuiteConstructionPolicy {
            required_tiers: vec![Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5],
            minimum_unique_cases_per_tier: 30,
            minimum_reviewers_per_case: 2,
            group_weights_basis_points: weights(),
            is_unanimous_eligibility_required: true,
            is_cross_tier_reuse_allowed: false,
            is_calibration_anchor_counted_toward_minimum: false,
        },
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

fn artifact(count: usize) -> ArtifactDefinition {
    ArtifactDefinition {
        name: ArtifactName("frontier-source-fixture".to_owned()),
        kind: ArtifactKind::Skill,
        root: PathBuf::from(ROOT),
        revision: "revision-1".to_owned(),
        required_destinations: Vec::new(),
        current_tiers: Vec::new(),
        cases: (0..count).map(case).collect(),
    }
}

fn case(index: usize) -> CaseDefinition {
    let is_holdout = index.is_multiple_of(15);
    CaseDefinition {
        id: CaseId(format!("case-{index:03}")),
        input: "synthetic input".to_owned(),
        expect: "synthetic result".to_owned(),
        source: "repository-owned synthetic fixture".to_owned(),
        is_holdout,
        support_files: Vec::new(),
        execution: ExecutionDefinition {
            drive: CaseDrive::Fixture {
                source: PathBuf::from(ROOT).join("input.txt"),
                verify_commands: vec![CommandDefinition {
                    program: "true".to_owned(),
                    arguments: Vec::new(),
                    working_directory: None,
                }],
            },
            allowed_tools: vec!["read".to_owned()],
            timeout_seconds: 1,
        },
    }
}

fn group(index: usize) -> FrontierCaseGroup {
    match index % 4 {
        0 => FrontierCaseGroup::Normal,
        1 => FrontierCaseGroup::Edge,
        2 => FrontierCaseGroup::Adversarial,
        _ => FrontierCaseGroup::Critical,
    }
}

fn timestamp() -> Timestamp {
    Timestamp("2026-08-27T14:52:42-0400".to_owned())
}

fn digest<T: serde::Serialize>(value: &T) -> String {
    Sha256::digest(serde_json::to_vec(value).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn case_key(reference: &model::FrontierCaseReference) -> FrontierCaseKey {
    FrontierCaseKey {
        artifact_path: reference.artifact_path.clone(),
        artifact_revision: reference.artifact_revision.clone(),
        case: reference.case.clone(),
    }
}

fn case_index(key: &FrontierCaseKey) -> u16 {
    key.case.0.strip_prefix("case-").unwrap().parse().unwrap()
}

fn set_evidence(decision: &mut FrontierCaseReviewDecision, value: Vec<String>) {
    match decision {
        FrontierCaseReviewDecision::Eligible { evidence, .. }
        | FrontierCaseReviewDecision::Rejected { evidence, .. } => *evidence = value,
    }
}

fn set_confirmation(decision: &mut FrontierCaseReviewDecision, value: bool) {
    if let FrontierCaseReviewDecision::Eligible {
        is_confirmation, ..
    } = decision
    {
        *is_confirmation = value;
    }
}

fn set_group(decision: &mut FrontierCaseReviewDecision, value: FrontierCaseGroup) {
    if let FrontierCaseReviewDecision::Eligible { group, .. } = decision {
        *group = value;
    }
}

fn set_difficulty(decision: &mut FrontierCaseReviewDecision, value: u16) {
    if let FrontierCaseReviewDecision::Eligible {
        relative_difficulty_basis_points,
        ..
    } = decision
    {
        *relative_difficulty_basis_points = value;
    }
}

fn assert_named_error<T>(result: Result<T, SkillEvalError>, expected: &str) {
    let error = result.err().expect("expected a named error");
    let message = match error {
        SkillEvalError::InvalidArguments(message)
        | SkillEvalError::InvalidConfiguration(message)
        | SkillEvalError::Verification(message)
        | SkillEvalError::NotFound(message) => message,
        other => format!("{other:?}"),
    };
    assert!(
        message.contains(expected),
        "expected {expected:?} in error {message:?}"
    );
}
