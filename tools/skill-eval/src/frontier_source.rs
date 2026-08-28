use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use sha2::{Digest, Sha256};

use crate::model::{
    ArtifactDefinition, CaseDrive, CommandDefinition, FrontierCaseGroup,
    FrontierCaseInventoryEntry, FrontierCaseKey, FrontierCaseReference, FrontierCaseReviewDecision,
    FrontierSuite, FrontierSuiteConstructionPlan, FrontierSuiteConstructionPolicy,
    FrontierSuiteInventory, FrontierSuiteProposal, FrontierSuiteProposalStatus,
    FrontierSuiteReviewSet, FrontierTierCapacity, FrontierTierSuite, SkillEvalError, Tier,
    Timestamp,
};

const VERSION: u64 = 1;
const BASIS_POINTS_TOTAL: u16 = 10_000;
const REQUIRED_TIERS: [Tier; 5] = [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5];
const GROUPS: [FrontierCaseGroup; 4] = [
    FrontierCaseGroup::Normal,
    FrontierCaseGroup::Edge,
    FrontierCaseGroup::Adversarial,
    FrontierCaseGroup::Critical,
];

type ScoredFrontierCase = (Vec<u16>, FrontierCaseKey, FrontierCaseReference);

/// Builds the complete executable-case inventory for offline tier review.
///
/// The inputs are the frozen construction plan, loaded artifacts, and generation time. The output
/// is one source-revision-bound inventory. The function does not write files or run a case.
///
/// # Errors
///
/// Returns an error for an invalid plan, duplicate artifact or case identity, source drift,
/// unsupported drive, missing fixture, or unsafe path.
pub(crate) fn build_frontier_suite_inventory(
    plan: &FrontierSuiteConstructionPlan,
    artifacts: &[ArtifactDefinition],
    generated_at: &Timestamp,
) -> Result<FrontierSuiteInventory, SkillEvalError> {
    validate_plan(plan)?;
    if generated_at.0.trim().is_empty() {
        return Err(invalid("frontier inventory generation time is empty"));
    }

    let roots = plan.artifact_roots.iter().collect::<BTreeSet<_>>();
    if artifacts.len() != roots.len() {
        return Err(invalid(
            "frontier artifact-root coverage differs from the plan",
        ));
    }

    let mut covered_roots = BTreeSet::new();
    let mut artifact_roots = BTreeSet::new();
    let mut artifact_names = BTreeSet::new();
    let mut cases = Vec::new();
    let mut case_keys = BTreeSet::new();
    for artifact in artifacts {
        validate_loaded_path(&artifact.root, "frontier loaded artifact root")?;
        let matching_roots = plan
            .artifact_roots
            .iter()
            .filter(|root| artifact.root == **root || artifact.root.ends_with(root))
            .collect::<Vec<_>>();
        if matching_roots.len() != 1 || !covered_roots.insert(matching_roots[0].clone()) {
            return Err(invalid(format!(
                "frontier artifact root {} is foreign, stale, or ambiguous",
                artifact.root.display()
            )));
        }
        let repository_root = matching_roots[0];
        if !artifact_roots.insert(artifact.root.clone())
            || !artifact_names.insert(artifact.name.clone())
        {
            return Err(invalid("frontier artifacts are duplicate"));
        }
        if artifact.revision.trim().is_empty() {
            return Err(invalid(format!(
                "frontier artifact {} has an empty current revision",
                artifact.root.display()
            )));
        }

        for case in &artifact.cases {
            if case.id.0.trim().is_empty() {
                return Err(invalid(format!(
                    "frontier artifact {} has an empty case identity",
                    artifact.root.display()
                )));
            }
            validate_drive(&case.execution.drive, &artifact.root, &case.id.0)?;
            if case.execution.timeout_seconds == 0 {
                return Err(invalid(format!(
                    "frontier case {} has a zero execution timeout",
                    case.id.0
                )));
            }
            let key = FrontierCaseKey {
                artifact_path: repository_root.clone(),
                artifact_revision: artifact.revision.clone(),
                case: case.id.clone(),
            };
            if !case_keys.insert(key.clone()) {
                return Err(invalid(format!(
                    "frontier case {} is duplicate",
                    case_identity(&key)
                )));
            }
            cases.push(FrontierCaseInventoryEntry {
                key,
                drive: case.execution.drive.clone(),
                is_holdout: case.is_holdout,
            });
        }
    }
    if covered_roots != roots.into_iter().cloned().collect() {
        return Err(invalid(
            "frontier artifact-root coverage differs from the plan",
        ));
    }

    cases.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(FrontierSuiteInventory {
        version: VERSION,
        generated_at: generated_at.clone(),
        cases,
    })
}

/// Validates complete independent review coverage for one inventory.
///
/// The inputs are a frozen inventory and review set. The function produces no value after all
/// identities, reviewer counts, decisions, and evidence satisfy the construction policy.
///
/// # Errors
///
/// Returns an error for digest drift, missing or foreign cases, duplicate reviewers, incomplete
/// coverage, invalid confirmation flags, or unresolved reviewer disagreement.
pub(crate) fn validate_frontier_suite_review_set(
    plan: &FrontierSuiteConstructionPlan,
    inventory: &FrontierSuiteInventory,
    reviews: &FrontierSuiteReviewSet,
) -> Result<(), SkillEvalError> {
    validate_plan(plan)?;
    validate_inventory(inventory)?;
    if reviews.version != VERSION {
        return Err(invalid("frontier review-set version must be 1"));
    }
    if reviews.inventory_sha256 != digest(inventory, "frontier inventory")? {
        return Err(invalid("frontier review-set inventory digest differs"));
    }

    let inventory_cases = inventory
        .cases
        .iter()
        .map(|entry| (&entry.key, entry.is_holdout))
        .collect::<BTreeMap<_, _>>();
    let mut records = BTreeMap::<&FrontierCaseKey, Vec<_>>::new();
    for record in &reviews.records {
        let Some(is_holdout) = inventory_cases.get(&record.key) else {
            return Err(invalid(format!(
                "frontier review names foreign or stale case {}",
                case_identity(&record.key)
            )));
        };
        if record.reviewer.trim().is_empty() {
            return Err(invalid(format!(
                "frontier case {} has an empty reviewer",
                case_identity(&record.key)
            )));
        }
        if record.reviewed_at.0.trim().is_empty() {
            return Err(invalid(format!(
                "frontier case {} has an empty review time",
                case_identity(&record.key)
            )));
        }
        validate_decision(&record.key, &record.decision, *is_holdout)?;
        records.entry(&record.key).or_default().push(record);
    }

    for entry in &inventory.cases {
        let case_records = records.get(&entry.key).ok_or_else(|| {
            invalid(format!(
                "frontier case {} has no reviews",
                case_identity(&entry.key)
            ))
        })?;
        if case_records.len() < usize::from(plan.policy.minimum_reviewers_per_case) {
            return Err(invalid(format!(
                "frontier case {} has fewer than {} independent reviewers",
                case_identity(&entry.key),
                plan.policy.minimum_reviewers_per_case
            )));
        }
        let mut reviewers = BTreeSet::new();
        for record in case_records {
            if !reviewers.insert(record.reviewer.as_str()) {
                return Err(invalid(format!(
                    "frontier case {} repeats reviewer {}",
                    case_identity(&entry.key),
                    record.reviewer
                )));
            }
        }
        validate_agreement(&entry.key, case_records)?;
    }
    Ok(())
}

/// Builds every proposed difficulty tier and its capacity verdict together.
///
/// The inputs are the frozen plan, complete inventory, and validated review set. The output is a
/// ready or blocked all-tier proposal with exact capacity evidence and calibration anchors.
///
/// # Errors
///
/// Returns an error for policy or digest drift, invalid difficulty evidence, arithmetic overflow,
/// cross-tier reuse, group-weight mismatch, or inconsistent reviewer decisions.
pub(crate) fn build_frontier_suite_proposal(
    plan: &FrontierSuiteConstructionPlan,
    inventory: &FrontierSuiteInventory,
    reviews: &FrontierSuiteReviewSet,
) -> Result<FrontierSuiteProposal, SkillEvalError> {
    validate_frontier_suite_review_set(plan, inventory, reviews)?;

    let mut by_case = BTreeMap::<&FrontierCaseKey, Vec<_>>::new();
    for record in &reviews.records {
        by_case.entry(&record.key).or_default().push(record);
    }

    let mut eligible = Vec::new();
    let mut rejected = 0_u16;
    for entry in &inventory.cases {
        let records = by_case
            .get(&entry.key)
            .ok_or_else(|| invalid("frontier validated review coverage disappeared"))?;
        match &records[0].decision {
            FrontierCaseReviewDecision::Eligible {
                group,
                is_confirmation,
                ..
            } => {
                let mut difficulties = records
                    .iter()
                    .map(|record| match record.decision {
                        FrontierCaseReviewDecision::Eligible {
                            relative_difficulty_basis_points,
                            ..
                        } => Ok(relative_difficulty_basis_points),
                        FrontierCaseReviewDecision::Rejected { .. } => Err(invalid(format!(
                            "frontier case {} dropped reviewer disagreement",
                            case_identity(&entry.key)
                        ))),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                difficulties.sort_unstable();
                eligible.push((
                    difficulties,
                    entry.key.clone(),
                    FrontierCaseReference {
                        artifact_path: entry.key.artifact_path.clone(),
                        artifact_revision: entry.key.artifact_revision.clone(),
                        case: entry.key.case.clone(),
                        group: *group,
                        is_confirmation: *is_confirmation,
                    },
                ));
            }
            FrontierCaseReviewDecision::Rejected { .. } => {
                rejected = rejected
                    .checked_add(1)
                    .ok_or_else(|| invalid("frontier rejected-case count overflow"))?;
            }
        }
    }
    eligible.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    let required = usize::from(plan.policy.minimum_unique_cases_per_tier);
    let mut tier_cases = BTreeMap::new();
    let mut cursor = 0_usize;
    for (index, tier) in plan.policy.required_tiers.iter().enumerate() {
        let end = if index + 1 == plan.policy.required_tiers.len() {
            eligible.len()
        } else {
            cursor.saturating_add(required).min(eligible.len())
        };
        tier_cases.insert(*tier, eligible[cursor..end].to_vec());
        cursor = end;
    }
    if cursor != eligible.len() {
        return Err(invalid("frontier proposal dropped an eligible scored case"));
    }
    rebalance_group_coverage(&mut tier_cases);

    let mut proposed_tiers = BTreeMap::new();
    let mut tier_capacity = BTreeMap::new();
    for tier in &plan.policy.required_tiers {
        let references = tier_cases
            .get(tier)
            .ok_or_else(|| invalid("frontier proposal tier allocation disappeared"))?
            .iter()
            .map(|(_, _, reference)| reference.clone())
            .collect::<Vec<_>>();
        let accepted_unique_cases = u16::try_from(references.len())
            .map_err(|_| invalid("frontier accepted-case count overflow"))?;
        let shortfall = plan
            .policy
            .minimum_unique_cases_per_tier
            .saturating_sub(accepted_unique_cases);
        let is_complete = is_tier_complete(
            plan.policy.minimum_unique_cases_per_tier,
            accepted_unique_cases,
            &references,
        );
        proposed_tiers.insert(
            *tier,
            FrontierTierSuite {
                group_weights_basis_points: plan.policy.group_weights_basis_points.clone(),
                cases: references,
            },
        );
        tier_capacity.insert(
            *tier,
            FrontierTierCapacity {
                required_unique_cases: plan.policy.minimum_unique_cases_per_tier,
                accepted_unique_cases,
                shortfall,
                duplicate_cases: 0,
                rejected_cases: rejected,
                is_complete,
            },
        );
    }

    let mut holdout_cases = inventory
        .cases
        .iter()
        .filter(|entry| entry.is_holdout)
        .map(|entry| entry.key.clone())
        .collect::<Vec<_>>();
    holdout_cases.sort();
    holdout_cases.dedup();
    let status = if tier_capacity.values().all(|capacity| capacity.is_complete) {
        FrontierSuiteProposalStatus::Ready
    } else {
        FrontierSuiteProposalStatus::Blocked
    };

    Ok(FrontierSuiteProposal {
        version: VERSION,
        inventory_sha256: digest(inventory, "frontier inventory")?,
        review_set_sha256: digest(reviews, "frontier review set")?,
        policy: plan.policy.clone(),
        proposed_tiers,
        calibration_anchors: Vec::new(),
        holdout_cases,
        tier_capacity,
        status,
    })
}

fn rebalance_group_coverage(tiers: &mut BTreeMap<Tier, Vec<ScoredFrontierCase>>) {
    let tier_order = tiers.keys().copied().collect::<Vec<_>>();
    for recipient_tier in &tier_order {
        for group in GROUPS {
            let Some(recipient) = tiers.get(recipient_tier) else {
                continue;
            };
            if recipient.iter().any(|case| case.2.group == group) {
                continue;
            }
            let recipient_group_counts = group_counts(recipient);
            let mut best = None;
            for donor_tier in &tier_order {
                if donor_tier == recipient_tier {
                    continue;
                }
                let Some(donor) = tiers.get(donor_tier) else {
                    continue;
                };
                let donor_group_counts = group_counts(donor);
                for (donor_index, donor_case) in donor.iter().enumerate() {
                    if donor_case.2.group != group
                        || donor_group_counts.get(&group).copied().unwrap_or(0) < 2
                    {
                        continue;
                    }
                    for (recipient_index, recipient_case) in recipient.iter().enumerate() {
                        if recipient_group_counts
                            .get(&recipient_case.2.group)
                            .copied()
                            .unwrap_or(0)
                            < 2
                        {
                            continue;
                        }
                        let candidate = (
                            difficulty_distance(&donor_case.0, &recipient_case.0),
                            donor_case.1.clone(),
                            recipient_case.1.clone(),
                            *donor_tier,
                            donor_index,
                            recipient_index,
                        );
                        if best.as_ref().is_none_or(|current| candidate < *current) {
                            best = Some(candidate);
                        }
                    }
                }
            }
            let Some((_, _, _, donor_tier, donor_index, recipient_index)) = best else {
                continue;
            };
            let donor_case = tiers
                .get(&donor_tier)
                .and_then(|cases| cases.get(donor_index))
                .cloned();
            let recipient_case = tiers
                .get(recipient_tier)
                .and_then(|cases| cases.get(recipient_index))
                .cloned();
            let (Some(donor_case), Some(recipient_case)) = (donor_case, recipient_case) else {
                continue;
            };
            if let Some(case) = tiers
                .get_mut(&donor_tier)
                .and_then(|cases| cases.get_mut(donor_index))
            {
                *case = recipient_case;
            }
            if let Some(case) = tiers
                .get_mut(recipient_tier)
                .and_then(|cases| cases.get_mut(recipient_index))
            {
                *case = donor_case;
            }
        }
    }
}

fn group_counts(cases: &[ScoredFrontierCase]) -> BTreeMap<FrontierCaseGroup, usize> {
    GROUPS
        .into_iter()
        .map(|group| {
            (
                group,
                cases.iter().filter(|case| case.2.group == group).count(),
            )
        })
        .collect()
}

fn difficulty_distance(left: &[u16], right: &[u16]) -> u64 {
    let score_distance = left
        .iter()
        .zip(right)
        .fold(0_u64, |distance, (left, right)| {
            distance.saturating_add(u64::from(left.abs_diff(*right)))
        });
    let length_distance = u64::try_from(left.len().abs_diff(right.len()))
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::from(BASIS_POINTS_TOTAL));
    score_distance.saturating_add(length_distance)
}

/// Converts one ready all-tier proposal into the publishable frontier suite.
///
/// The input is a validated proposal. The output is a complete versioned suite with no calibration
/// anchor counted in a scored tier.
///
/// # Errors
///
/// Returns an error when the proposal is blocked, a required tier is absent or short, a case is
/// reused, a weight is invalid, a review is stale, or a digest disagrees.
pub(crate) fn frontier_suite_from_ready_proposal(
    proposal: &FrontierSuiteProposal,
) -> Result<FrontierSuite, SkillEvalError> {
    if proposal.version != VERSION {
        return Err(invalid("frontier proposal version must be 1"));
    }
    if proposal.status != FrontierSuiteProposalStatus::Ready {
        return Err(invalid("frontier proposal is blocked"));
    }
    validate_policy(&proposal.policy)?;
    validate_digest(&proposal.inventory_sha256, "frontier proposal inventory")?;
    validate_digest(&proposal.review_set_sha256, "frontier proposal review set")?;
    validate_sorted_unique_keys(&proposal.holdout_cases, "frontier proposal holdout cases")?;
    validate_sorted_unique_keys(
        &proposal.calibration_anchors,
        "frontier proposal calibration anchors",
    )?;

    if proposal.proposed_tiers.len() != proposal.policy.required_tiers.len()
        || proposal.tier_capacity.len() != proposal.policy.required_tiers.len()
    {
        return Err(invalid(
            "frontier proposal does not contain every required tier",
        ));
    }

    let holdouts = proposal.holdout_cases.iter().collect::<BTreeSet<_>>();
    let anchors = proposal.calibration_anchors.iter().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut rejected_cases = None;
    for tier in &proposal.policy.required_tiers {
        let suite = proposal
            .proposed_tiers
            .get(tier)
            .ok_or_else(|| invalid(format!("frontier proposal is missing tier {tier:?}")))?;
        validate_weights(&suite.group_weights_basis_points)?;
        if suite.group_weights_basis_points != proposal.policy.group_weights_basis_points {
            return Err(invalid(format!(
                "frontier tier {tier:?} group weights drift from policy"
            )));
        }
        let capacity = proposal.tier_capacity.get(tier).ok_or_else(|| {
            invalid(format!(
                "frontier proposal is missing tier {tier:?} capacity"
            ))
        })?;
        let unique_cases = suite
            .cases
            .iter()
            .map(reference_key)
            .collect::<BTreeSet<_>>();
        let accepted = u16::try_from(unique_cases.len())
            .map_err(|_| invalid("frontier proposal case count overflow"))?;
        let total = u16::try_from(suite.cases.len())
            .map_err(|_| invalid("frontier proposal case count overflow"))?;
        let duplicates = total.saturating_sub(accepted);
        let shortfall = proposal
            .policy
            .minimum_unique_cases_per_tier
            .saturating_sub(accepted);
        let is_complete = is_tier_complete(
            proposal.policy.minimum_unique_cases_per_tier,
            accepted,
            &suite.cases,
        );
        if capacity.required_unique_cases != proposal.policy.minimum_unique_cases_per_tier
            || capacity.accepted_unique_cases != accepted
            || capacity.shortfall != shortfall
            || capacity.duplicate_cases != duplicates
            || capacity.is_complete != is_complete
            || rejected_cases.is_some_and(|count| count != capacity.rejected_cases)
        {
            return Err(invalid(format!(
                "frontier tier {tier:?} capacity is forged or inconsistent"
            )));
        }
        rejected_cases = Some(capacity.rejected_cases);
        if shortfall != 0 {
            return Err(invalid(format!(
                "frontier tier {tier:?} is below minimum capacity"
            )));
        }

        let mut present_groups = BTreeSet::new();
        for case in &suite.cases {
            let key = reference_key(case);
            validate_safe_path(&key.artifact_path, "frontier proposal artifact path")?;
            if key.artifact_revision.trim().is_empty() || key.case.0.trim().is_empty() {
                return Err(invalid("frontier proposal contains a stale case identity"));
            }
            if !seen.insert(key.clone()) {
                return Err(invalid(format!(
                    "frontier proposal reuses case {} across tiers",
                    case_identity(&key)
                )));
            }
            if anchors.contains(&key) {
                return Err(invalid(format!(
                    "frontier calibration anchor {} is counted in a scored tier",
                    case_identity(&key)
                )));
            }
            if case.is_confirmation && !holdouts.contains(&key) {
                return Err(invalid(format!(
                    "frontier confirmation {} is absent from holdout cases",
                    case_identity(&key)
                )));
            }
            present_groups.insert(case.group);
        }
        if present_groups != GROUPS.into_iter().collect() {
            return Err(invalid(format!(
                "frontier tier {tier:?} does not contain every case group"
            )));
        }
    }

    Ok(FrontierSuite {
        version: VERSION,
        tiers: proposal.proposed_tiers.clone(),
    })
}

fn validate_plan(plan: &FrontierSuiteConstructionPlan) -> Result<(), SkillEvalError> {
    if plan.version != VERSION {
        return Err(invalid("frontier construction plan version must be 1"));
    }
    if plan.artifact_roots.is_empty() {
        return Err(invalid("frontier construction plan has no artifact roots"));
    }
    let mut roots = BTreeSet::new();
    for root in &plan.artifact_roots {
        validate_safe_path(root, "frontier construction artifact root")?;
        if !roots.insert(root) {
            return Err(invalid(
                "frontier construction artifact roots are duplicate",
            ));
        }
    }
    validate_policy(&plan.policy)
}

fn validate_policy(policy: &FrontierSuiteConstructionPolicy) -> Result<(), SkillEvalError> {
    if policy.required_tiers != REQUIRED_TIERS {
        return Err(invalid(
            "frontier policy must require tiers T1 through T5 in order",
        ));
    }
    if policy.minimum_unique_cases_per_tier < 30 {
        return Err(invalid(
            "frontier policy requires at least 30 unique cases per tier",
        ));
    }
    if policy.minimum_reviewers_per_case < 2 {
        return Err(invalid(
            "frontier policy requires at least two reviewers per case",
        ));
    }
    validate_weights(&policy.group_weights_basis_points)?;
    if !policy.is_unanimous_eligibility_required {
        return Err(invalid(
            "frontier policy must require unanimous eligibility",
        ));
    }
    if policy.is_cross_tier_reuse_allowed {
        return Err(invalid("frontier policy must forbid cross-tier reuse"));
    }
    if policy.is_calibration_anchor_counted_toward_minimum {
        return Err(invalid(
            "frontier policy must not count calibration anchors",
        ));
    }
    Ok(())
}

fn validate_weights(weights: &BTreeMap<FrontierCaseGroup, u16>) -> Result<(), SkillEvalError> {
    if weights.len() != GROUPS.len()
        || GROUPS
            .iter()
            .any(|group| weights.get(group).is_none_or(|weight| *weight == 0))
    {
        return Err(invalid(
            "frontier policy must assign all four groups positive weights",
        ));
    }
    let total = weights
        .values()
        .try_fold(0_u16, |total, weight| total.checked_add(*weight))
        .ok_or_else(|| invalid("frontier group weight overflow"))?;
    if total != BASIS_POINTS_TOTAL {
        return Err(invalid(
            "frontier group weights must total 10000 basis points",
        ));
    }
    Ok(())
}

fn validate_inventory(inventory: &FrontierSuiteInventory) -> Result<(), SkillEvalError> {
    if inventory.version != VERSION {
        return Err(invalid("frontier inventory version must be 1"));
    }
    if inventory.generated_at.0.trim().is_empty() {
        return Err(invalid("frontier inventory generation time is empty"));
    }
    let mut previous = None;
    for entry in &inventory.cases {
        if previous.is_some_and(|key| key >= &entry.key) {
            return Err(invalid(
                "frontier inventory cases are duplicate or unsorted",
            ));
        }
        validate_safe_path(&entry.key.artifact_path, "frontier inventory artifact path")?;
        if entry.key.artifact_revision.trim().is_empty() || entry.key.case.0.trim().is_empty() {
            return Err(invalid(
                "frontier inventory contains an incomplete case identity",
            ));
        }
        validate_inventory_drive(&entry.drive, &entry.key.case.0)?;
        previous = Some(&entry.key);
    }
    Ok(())
}

fn validate_drive(
    drive: &CaseDrive,
    artifact_root: &Path,
    case: &str,
) -> Result<(), SkillEvalError> {
    match drive {
        CaseDrive::Response => Err(invalid(format!(
            "frontier case {case} uses unsupported response drive"
        ))),
        CaseDrive::Fixture {
            source,
            verify_commands,
        } => {
            if source.as_os_str().is_empty() {
                return Err(invalid(format!(
                    "frontier case {case} has a missing fixture"
                )));
            }
            validate_loaded_path(source, "frontier fixture source")?;
            if !source.starts_with(artifact_root) {
                return Err(invalid(format!(
                    "frontier case {case} fixture is outside its artifact root"
                )));
            }
            for command in verify_commands {
                validate_command(command, case)?;
            }
            Ok(())
        }
        CaseDrive::ExistingHarness { command } => validate_command(command, case),
    }
}

fn validate_command(command: &CommandDefinition, case: &str) -> Result<(), SkillEvalError> {
    if command.program.trim().is_empty() || command.program.contains('\0') {
        return Err(invalid(format!(
            "frontier case {case} has an invalid executable"
        )));
    }
    if command
        .arguments
        .iter()
        .any(|argument| argument.contains('\0'))
    {
        return Err(invalid(format!(
            "frontier case {case} has an invalid executable argument"
        )));
    }
    if let Some(directory) = &command.working_directory {
        validate_loaded_path(directory, "frontier command working directory")?;
    }
    Ok(())
}

fn validate_inventory_drive(drive: &CaseDrive, case: &str) -> Result<(), SkillEvalError> {
    match drive {
        CaseDrive::Response => Err(invalid(format!(
            "frontier case {case} uses unsupported response drive"
        ))),
        CaseDrive::Fixture {
            source,
            verify_commands,
        } => {
            if source.as_os_str().is_empty() {
                return Err(invalid(format!(
                    "frontier case {case} has a missing fixture"
                )));
            }
            validate_loaded_path(source, "frontier fixture source")?;
            for command in verify_commands {
                validate_command(command, case)?;
            }
            Ok(())
        }
        CaseDrive::ExistingHarness { command } => validate_command(command, case),
    }
}

fn validate_decision(
    key: &FrontierCaseKey,
    decision: &FrontierCaseReviewDecision,
    is_holdout: bool,
) -> Result<(), SkillEvalError> {
    let evidence = match decision {
        FrontierCaseReviewDecision::Eligible {
            relative_difficulty_basis_points,
            is_confirmation,
            evidence,
            ..
        } => {
            if !(1..=BASIS_POINTS_TOTAL).contains(relative_difficulty_basis_points) {
                return Err(invalid(format!(
                    "frontier case {} has invalid difficulty basis points",
                    case_identity(key)
                )));
            }
            if *is_confirmation && !is_holdout {
                return Err(invalid(format!(
                    "frontier case {} is a confirmation but not a holdout",
                    case_identity(key)
                )));
            }
            evidence
        }
        FrontierCaseReviewDecision::Rejected { evidence, .. } => evidence,
    };
    if evidence.is_empty() || evidence.iter().any(|item| item.trim().is_empty()) {
        return Err(invalid(format!(
            "frontier case {} has empty review evidence",
            case_identity(key)
        )));
    }
    Ok(())
}

fn validate_agreement(
    key: &FrontierCaseKey,
    records: &[&crate::model::FrontierCaseReviewRecord],
) -> Result<(), SkillEvalError> {
    let first = &records[0].decision;
    for record in &records[1..] {
        let is_agreed = match (first, &record.decision) {
            (
                FrontierCaseReviewDecision::Eligible {
                    group: first_group,
                    is_confirmation: first_confirmation,
                    ..
                },
                FrontierCaseReviewDecision::Eligible {
                    group,
                    is_confirmation,
                    ..
                },
            ) => first_group == group && first_confirmation == is_confirmation,
            (
                FrontierCaseReviewDecision::Rejected { .. },
                FrontierCaseReviewDecision::Rejected { .. },
            ) => true,
            _ => false,
        };
        if !is_agreed {
            return Err(invalid(format!(
                "frontier reviewers disagree on case {}",
                case_identity(key)
            )));
        }
    }
    Ok(())
}

fn validate_loaded_path(path: &Path, label: &str) -> Result<(), SkillEvalError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        || path.to_string_lossy().chars().any(char::is_control)
    {
        return Err(invalid(format!("{label} is unsafe")));
    }
    Ok(())
}

fn validate_safe_path(path: &Path, label: &str) -> Result<(), SkillEvalError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || path.to_string_lossy().chars().any(char::is_control)
    {
        return Err(invalid(format!(
            "{label} must be a safe repository-relative path"
        )));
    }
    Ok(())
}

fn validate_sorted_unique_keys(
    keys: &[FrontierCaseKey],
    label: &str,
) -> Result<(), SkillEvalError> {
    if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(invalid(format!(
            "{label} must be strictly sorted and unique"
        )));
    }
    for key in keys {
        validate_safe_path(&key.artifact_path, label)?;
        if key.artifact_revision.trim().is_empty() || key.case.0.trim().is_empty() {
            return Err(invalid(format!("{label} contains an incomplete identity")));
        }
    }
    Ok(())
}

fn validate_digest(value: &str, label: &str) -> Result<(), SkillEvalError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(format!("{label} digest is invalid")));
    }
    Ok(())
}

fn digest<T: serde::Serialize>(value: &T, label: &str) -> Result<String, SkillEvalError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| invalid(format!("{label} serialization failed: {error}")))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn is_tier_complete(
    required_unique_cases: u16,
    accepted_unique_cases: u16,
    cases: &[FrontierCaseReference],
) -> bool {
    accepted_unique_cases >= required_unique_cases
        && GROUPS
            .iter()
            .all(|group| cases.iter().any(|case| case.group == *group))
}

fn reference_key(reference: &FrontierCaseReference) -> FrontierCaseKey {
    FrontierCaseKey {
        artifact_path: reference.artifact_path.clone(),
        artifact_revision: reference.artifact_revision.clone(),
        case: reference.case.clone(),
    }
}

fn case_identity(key: &FrontierCaseKey) -> String {
    format!(
        "{}@{}:{}",
        key.artifact_path.display(),
        key.artifact_revision,
        key.case.0
    )
}

fn invalid(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(message.into())
}
