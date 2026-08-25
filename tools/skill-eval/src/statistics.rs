use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    CaseId, CheckStatus, ConfidenceInterval, EvidenceRole, ModelIdentity, PoolEntrant,
    PoolEntrantEvidence, PoolPolicy, PoolStage, QualificationBoundary, QualificationPolicy,
    RankedPool, SkillEvalError, ThinkingDecision, Tier, TierEvidence, TierStatus, TrialRecord,
    TrialUsage,
};

pub(crate) fn evaluate_calibration(
    requested: &ModelIdentity,
    expected_cases: &[CaseId],
    trials: &[TrialRecord],
    policy: &PoolPolicy,
) -> Result<PoolEntrantEvidence, SkillEvalError> {
    validate_calibration_policy(policy)?;
    evaluate_pool_evidence(
        requested,
        expected_cases,
        trials,
        policy,
        PoolStage::Calibration,
        policy.calibration_repeats_per_case,
        "calibration",
    )
}

pub(crate) fn evaluate_qualification(
    requested: &ModelIdentity,
    expected_cases: &[CaseId],
    trials: &[TrialRecord],
    policy: &PoolPolicy,
) -> Result<PoolEntrantEvidence, SkillEvalError> {
    validate_calibration_policy(policy)?;
    evaluate_pool_evidence(
        requested,
        expected_cases,
        trials,
        policy,
        PoolStage::Qualification,
        policy.qualification_repeats_per_case,
        "qualification",
    )
}

fn evaluate_pool_evidence(
    requested: &ModelIdentity,
    expected_cases: &[CaseId],
    trials: &[TrialRecord],
    policy: &PoolPolicy,
    stage: PoolStage,
    repeats_per_case: u16,
    label: &str,
) -> Result<PoolEntrantEvidence, SkillEvalError> {
    let expected_case_set = expected_cases.iter().cloned().collect::<BTreeSet<_>>();
    if expected_cases.is_empty() || expected_case_set.len() != expected_cases.len() {
        return Err(invalid(&format!(
            "{label} expected cases must be nonempty and unique"
        )));
    }

    let first = trials
        .first()
        .ok_or_else(|| invalid(&format!("{label} trial set is empty")))?;
    if first.model != *requested {
        return Err(invalid(&format!(
            "{label} effective model differs from the requested model"
        )));
    }
    if first.key.tier != requested.tier {
        return Err(invalid(&format!(
            "{label} trial tier differs from its model"
        )));
    }
    if is_same_model(&first.judge_model, &first.model) {
        return Err(invalid(&format!("{label} candidate cannot judge itself")));
    }

    let mut attempts_by_case = expected_case_set
        .iter()
        .cloned()
        .map(|case| (case, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut harness_by_case = BTreeMap::new();
    let mut candidate_usage = empty_usage();
    let mut judge_usage = empty_usage();
    let mut total_score = 0_u64;
    let mut failed_trials = 0_u32;
    let mut catastrophic_trials = 0_u32;

    for trial in trials {
        if trial.key.artifact != first.key.artifact || trial.key.tier != first.key.tier {
            return Err(invalid(&format!("{label} trial set mixes exams or tiers")));
        }
        if trial.model != first.model || trial.model != *requested {
            return Err(invalid(&format!(
                "{label} trial set mixes candidate identities"
            )));
        }
        if trial.model.tier != trial.key.tier {
            return Err(invalid(&format!(
                "{label} trial tier differs from its model"
            )));
        }
        if trial.judge_model != first.judge_model {
            return Err(invalid(&format!(
                "{label} trial set mixes judge identities"
            )));
        }
        if is_same_model(&trial.judge_model, &trial.model) {
            return Err(invalid(&format!("{label} candidate cannot judge itself")));
        }
        if trial.harness.runner_version != first.harness.runner_version
            || trial.harness.pi_version != first.harness.pi_version
            || trial.harness.artifact_revision != first.harness.artifact_revision
        {
            return Err(invalid(&format!(
                "{label} trial set has common harness drift"
            )));
        }
        if trial.verdict.score > 10 {
            return Err(invalid(&format!(
                "{label} trial score is outside 0 through 10"
            )));
        }

        let Some(attempts) = attempts_by_case.get_mut(&trial.key.case) else {
            return Err(invalid(&format!(
                "{label} trial set contains an unknown case"
            )));
        };
        if !attempts.insert(trial.key.attempt) {
            return Err(invalid(&format!(
                "{label} trial set contains a duplicate case attempt"
            )));
        }
        match harness_by_case.entry(trial.key.case.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(trial.harness.clone());
            }
            std::collections::btree_map::Entry::Occupied(entry)
                if entry.get() != &trial.harness =>
            {
                return Err(invalid(&format!("{label} case has harness identity drift")));
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
        }

        add_usage(&mut candidate_usage, &trial.candidate_usage)?;
        add_usage(&mut judge_usage, &trial.judge_usage)?;
        total_score = total_score
            .checked_add(u64::from(trial.verdict.score))
            .ok_or_else(|| invalid(&format!("{label} score arithmetic overflow")))?;

        let has_failed_check = trial
            .verdict
            .checks
            .iter()
            .any(|check| check.status == CheckStatus::Failed);
        if trial.verdict.score < policy.minimum_score || has_failed_check {
            failed_trials = failed_trials
                .checked_add(1)
                .ok_or_else(|| invalid(&format!("{label} failed trial count overflow")))?;
        }
        if trial.verdict.is_catastrophic {
            catastrophic_trials = catastrophic_trials
                .checked_add(1)
                .ok_or_else(|| invalid(&format!("{label} catastrophic trial count overflow")))?;
        }
    }

    let required_attempts = (1..=repeats_per_case).collect::<BTreeSet<_>>();
    if attempts_by_case
        .values()
        .any(|attempts| attempts != &required_attempts)
    {
        return Err(invalid(&format!("{label} trial set is incomplete")));
    }

    let expected_trials = u32::try_from(expected_cases.len())
        .ok()
        .and_then(|cases| cases.checked_mul(u32::from(repeats_per_case)))
        .ok_or_else(|| invalid(&format!("{label} expected trial count overflow")))?;
    let completed_trials = u32::try_from(trials.len())
        .map_err(|_| invalid(&format!("{label} trial count overflow")))?;
    if completed_trials != expected_trials {
        return Err(invalid(&format!("{label} trial set is incomplete")));
    }

    let passing_trials = expected_trials
        .checked_sub(failed_trials)
        .ok_or_else(|| invalid(&format!("{label} reliability denominator is invalid")))?;
    let reliability_basis_points = u64::from(passing_trials)
        .checked_mul(10_000)
        .ok_or_else(|| invalid(&format!("{label} reliability arithmetic overflow")))?
        .checked_div(u64::from(expected_trials))
        .ok_or_else(|| invalid(&format!("{label} reliability denominator is invalid")))?;
    let mean_score = total_score as f64 / f64::from(expected_trials) / 10.0;
    let score = ConfidenceInterval {
        lower: mean_score,
        estimate: mean_score,
        upper: mean_score,
    };
    let is_passing = mean_score >= f64::from(policy.minimum_score) / 10.0
        && reliability_basis_points >= u64::from(policy.minimum_reliability_basis_points)
        && catastrophic_trials == 0;
    let harnesses = expected_cases
        .iter()
        .map(|case| {
            harness_by_case
                .get(case)
                .cloned()
                .ok_or_else(|| invalid(&format!("{label} trial set is incomplete")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut total_usage = candidate_usage.clone();
    add_usage(&mut total_usage, &judge_usage)?;

    Ok(PoolEntrantEvidence {
        stage,
        requested_model: requested.clone(),
        effective_model: first.model.clone(),
        judge_model: first.judge_model.clone(),
        harnesses,
        is_passing,
        completed_trials,
        expected_trials,
        failed_trials,
        catastrophic_trials,
        score,
        candidate_usage,
        judge_usage,
        total_usage,
    })
}

fn is_same_model(left: &ModelIdentity, right: &ModelIdentity) -> bool {
    left.provider == right.provider && left.model == right.model
}

fn validate_calibration_policy(policy: &PoolPolicy) -> Result<(), SkillEvalError> {
    if policy.calibration_repeats_per_case == 0
        || policy.qualification_repeats_per_case == 0
        || policy.promotion_count != 2
        || policy.minimum_score > 10
        || policy.minimum_reliability_basis_points > 10_000
        || policy.spending_limit_millionths_of_dollar == 0
        || !policy.is_provider_limit_enforced
    {
        return Err(invalid("pool policy has invalid calibration values"));
    }
    Ok(())
}

// TODO(AGNT-0032.T104): Select each model's lowest passing bounded thinking level.
pub(crate) fn select_thinking_level(
    _entrant: &PoolEntrant,
    _evidence: &[PoolEntrantEvidence],
) -> Result<ThinkingDecision, SkillEvalError> {
    unimplemented!("AGNT-0032.T104")
}

pub(crate) fn rank_pool(
    tier: Tier,
    calibration: &[PoolEntrantEvidence],
    qualification: &[PoolEntrantEvidence],
    policy: &PoolPolicy,
) -> Result<RankedPool, SkillEvalError> {
    validate_calibration_policy(policy)?;
    let promotion_count = usize::from(policy.promotion_count);
    if calibration.len() != 3 {
        return Err(invalid(
            "pool calibration must contain exactly three entrants",
        ));
    }
    if qualification.len() > promotion_count {
        return Err(invalid(
            "pool qualification contains more than two finalists",
        ));
    }

    validate_pool_stage(
        calibration,
        tier,
        PoolStage::Calibration,
        policy.calibration_repeats_per_case,
        policy,
    )?;

    let mut passing = calibration
        .iter()
        .filter(|evidence| evidence.is_passing)
        .collect::<Vec<_>>();
    passing.sort_by(|left, right| compare_pool_evidence(left, right));
    let promoted = passing
        .into_iter()
        .take(promotion_count)
        .map(|evidence| evidence.requested_model.clone())
        .collect::<Vec<_>>();

    validate_pool_stage(
        qualification,
        tier,
        PoolStage::Qualification,
        policy.qualification_repeats_per_case,
        policy,
    )?;
    for evidence in qualification {
        if !promoted.contains(&evidence.requested_model) {
            return Err(invalid(
                "pool qualification contains a non-promoted entrant",
            ));
        }
    }

    let is_complete = promoted.len() == promotion_count
        && qualification.len() == promotion_count
        && qualification.iter().all(|evidence| evidence.is_passing)
        && promoted.iter().all(|model| {
            qualification
                .iter()
                .any(|evidence| evidence.requested_model == *model)
        });
    let ranked = if is_complete {
        let mut finalists = qualification.iter().collect::<Vec<_>>();
        finalists.sort_by(|left, right| compare_pool_evidence(left, right));
        finalists
            .into_iter()
            .map(|evidence| evidence.requested_model.clone())
            .collect()
    } else {
        Vec::new()
    };

    // TODO(AGNT-0032.T103): Initialize empty thinking selections before adaptive ranking.
    Ok(RankedPool {
        tier,
        calibration: calibration.to_vec(),
        promoted,
        qualification: qualification.to_vec(),
        ranked,
        is_complete,
    })
}

fn validate_pool_stage(
    evidence: &[PoolEntrantEvidence],
    tier: Tier,
    stage: PoolStage,
    repeats_per_case: u16,
    policy: &PoolPolicy,
) -> Result<(), SkillEvalError> {
    let mut identities = Vec::with_capacity(evidence.len());
    let mut expected_harnesses = None;

    for item in evidence {
        if item.stage != stage {
            return Err(invalid("pool evidence has the wrong stage"));
        }
        if item.requested_model.tier != tier || item.effective_model.tier != tier {
            return Err(invalid("pool evidence has the wrong tier"));
        }
        if item.requested_model != item.effective_model {
            return Err(invalid(
                "pool requested and effective model identities differ",
            ));
        }
        if identities.contains(&item.requested_model) {
            return Err(invalid("pool evidence contains a duplicate entrant"));
        }
        identities.push(item.requested_model.clone());
        if is_same_model(&item.judge_model, &item.effective_model) {
            return Err(invalid("pool entrant cannot judge itself"));
        }
        if item.expected_trials == 0 || item.completed_trials != item.expected_trials {
            return Err(invalid("pool evidence has incomplete trial counts"));
        }
        if item.failed_trials > item.completed_trials
            || item.catastrophic_trials > item.completed_trials
        {
            return Err(invalid("pool evidence has impossible trial counts"));
        }

        let harness_count = u32::try_from(item.harnesses.len())
            .map_err(|_| invalid("pool evidence harness count overflow"))?;
        let evidence_expected_trials = harness_count
            .checked_mul(u32::from(repeats_per_case))
            .ok_or_else(|| invalid("pool evidence expected trial count overflow"))?;
        if evidence_expected_trials != item.expected_trials {
            return Err(invalid("pool evidence has inconsistent trial counts"));
        }
        match expected_harnesses {
            Some(harnesses) if harnesses != item.harnesses.as_slice() => {
                return Err(invalid("pool evidence mixes exam harnesses"));
            }
            None => expected_harnesses = Some(item.harnesses.as_slice()),
            Some(_) => {}
        }

        if !is_valid_interval(&item.score) {
            return Err(invalid("pool evidence has invalid score metrics"));
        }
        let passing_trials = item
            .completed_trials
            .checked_sub(item.failed_trials)
            .ok_or_else(|| invalid("pool evidence has invalid failure metrics"))?;
        let reliability_numerator = u64::from(passing_trials)
            .checked_mul(10_000)
            .ok_or_else(|| invalid("pool evidence reliability arithmetic overflow"))?;
        let reliability_floor = u64::from(item.completed_trials)
            .checked_mul(u64::from(policy.minimum_reliability_basis_points))
            .ok_or_else(|| invalid("pool evidence reliability arithmetic overflow"))?;
        if item.is_passing
            && (item.score.estimate < f64::from(policy.minimum_score) / 10.0
                || reliability_numerator < reliability_floor
                || item.catastrophic_trials != 0)
        {
            return Err(invalid("passing pool evidence does not meet policy"));
        }

        let mut expected_total_usage = item.candidate_usage.clone();
        add_usage(&mut expected_total_usage, &item.judge_usage)?;
        if expected_total_usage != item.total_usage {
            return Err(invalid("pool evidence has inconsistent total usage"));
        }
    }

    Ok(())
}

fn compare_pool_evidence(left: &PoolEntrantEvidence, right: &PoolEntrantEvidence) -> Ordering {
    compare_rate(
        left.candidate_usage.cost_millionths_of_dollar,
        left.completed_trials,
        right.candidate_usage.cost_millionths_of_dollar,
        right.completed_trials,
    )
    .then_with(|| {
        compare_rate(
            left.candidate_usage.elapsed_milliseconds,
            left.completed_trials,
            right.candidate_usage.elapsed_milliseconds,
            right.completed_trials,
        )
    })
    .then_with(|| {
        compare_rate(
            u64::from(left.failed_trials),
            left.completed_trials,
            u64::from(right.failed_trials),
            right.completed_trials,
        )
    })
    .then_with(|| compare_model_identity(&left.requested_model, &right.requested_model))
}

fn compare_rate(
    left_numerator: u64,
    left_denominator: u32,
    right_numerator: u64,
    right_denominator: u32,
) -> Ordering {
    (u128::from(left_numerator) * u128::from(right_denominator))
        .cmp(&(u128::from(right_numerator) * u128::from(left_denominator)))
}

fn compare_model_identity(left: &ModelIdentity, right: &ModelIdentity) -> Ordering {
    left.tier
        .cmp(&right.tier)
        .then_with(|| left.provider.cmp(&right.provider))
        .then_with(|| left.model.cmp(&right.model))
        .then_with(|| left.thinking.cmp(&right.thinking))
}

pub(crate) fn evaluate_tier(
    role: EvidenceRole,
    trials: &[TrialRecord],
    reference: Option<&TierEvidence>,
    policy: &QualificationPolicy,
) -> Result<TierEvidence, SkillEvalError> {
    validate_policy(policy)?;
    match role {
        EvidenceRole::Reference if reference.is_some() => {
            return Err(invalid(
                "reference evaluation must not include reference evidence",
            ));
        }
        EvidenceRole::Candidate => {
            let reference = reference
                .ok_or_else(|| invalid("candidate evaluation requires reference evidence"))?;
            validate_evidence(reference, policy, EvidenceRole::Reference)?;
        }
        EvidenceRole::Reference => {}
    }

    let first = trials
        .first()
        .ok_or_else(|| invalid("tier trial set is empty"))?;
    let is_expected_tier = match role {
        EvidenceRole::Reference => first.key.tier == policy.reference_tier,
        EvidenceRole::Candidate => policy.candidate_tiers.contains(&first.key.tier),
    };
    if !is_expected_tier {
        return Err(invalid("trial tier does not match its evidence role"));
    }
    if first.model.tier != first.key.tier {
        return Err(invalid("trial model tier differs from its trial key"));
    }

    let mut attempts_by_case = BTreeMap::<_, BTreeSet<_>>::new();
    let mut harnesses = Vec::new();
    let mut candidate_usage = empty_usage();
    let mut judge_usage = empty_usage();
    let mut scores = Vec::with_capacity(trials.len());
    let mut passed_trials = 0_u32;
    let mut is_catastrophic = false;

    for trial in trials {
        if trial.key.artifact != first.key.artifact || trial.key.tier != first.key.tier {
            return Err(invalid("tier trial set mixes artifacts or tiers"));
        }
        if trial.model != first.model
            || trial.harness.runner_version != first.harness.runner_version
            || trial.harness.pi_version != first.harness.pi_version
            || trial.harness.artifact_revision != first.harness.artifact_revision
        {
            return Err(invalid("tier trial set has common identity drift"));
        }
        if trial.model.tier != trial.key.tier {
            return Err(invalid("trial model tier differs from its trial key"));
        }
        if trial.verdict.score > 10 {
            return Err(invalid("trial score is outside 0 through 10"));
        }

        let attempts = attempts_by_case.entry(trial.key.case.clone()).or_default();
        if !attempts.insert(trial.key.attempt) {
            return Err(invalid("tier trial set contains a duplicate trial key"));
        }
        if !harnesses.contains(&trial.harness) {
            harnesses.push(trial.harness.clone());
        }

        add_usage(&mut candidate_usage, &trial.candidate_usage)?;
        add_usage(&mut judge_usage, &trial.judge_usage)?;
        scores.push(f64::from(trial.verdict.score) / 10.0);
        if trial.verdict.score >= policy.minimum_score && !trial.verdict.is_catastrophic {
            passed_trials = passed_trials
                .checked_add(1)
                .ok_or_else(|| invalid("passed trial count overflow"))?;
        }
        is_catastrophic |= trial.verdict.is_catastrophic;
    }

    let required_attempts = (1..=policy.repeats_per_case).collect::<BTreeSet<_>>();
    if attempts_by_case
        .values()
        .any(|attempts| attempts != &required_attempts)
    {
        return Err(invalid("tier trial set is incomplete"));
    }

    let expected_trials = u32::try_from(attempts_by_case.len())
        .ok()
        .and_then(|cases| cases.checked_mul(u32::from(policy.repeats_per_case)))
        .ok_or_else(|| invalid("expected trial count overflow"))?;
    let completed_trials =
        u32::try_from(trials.len()).map_err(|_| invalid("completed trial count overflow"))?;
    if completed_trials != expected_trials {
        return Err(invalid("tier trial set is incomplete"));
    }

    let score = confidence_interval(&scores, policy.confidence_level)?;
    let minimum = f64::from(policy.minimum_score) / 10.0;
    let is_noninferior = match reference {
        Some(reference) => score.lower + policy.noninferiority_margin >= reference.score.lower,
        None => true,
    };
    let status = if !is_catastrophic && score.lower >= minimum && is_noninferior {
        TierStatus::Accepted
    } else {
        TierStatus::Failed
    };
    let mut total_usage = candidate_usage.clone();
    add_usage(&mut total_usage, &judge_usage)?;

    Ok(TierEvidence {
        role,
        tier: first.key.tier,
        model: first.model.clone(),
        harnesses,
        status,
        completed_trials,
        expected_trials,
        passed_trials,
        score,
        candidate_usage,
        judge_usage,
        total_usage,
    })
}

pub(crate) fn find_boundary(
    evidence: &[TierEvidence],
    policy: &QualificationPolicy,
) -> Result<Option<QualificationBoundary>, SkillEvalError> {
    validate_policy(policy)?;

    let mut by_tier = BTreeMap::new();
    for item in evidence {
        validate_evidence(item, policy, EvidenceRole::Candidate)?;
        if by_tier.insert(item.tier, item).is_some() {
            return Err(invalid("qualification evidence contains a duplicate tier"));
        }
    }

    let mut has_accepted = false;
    for item in by_tier.values() {
        match item.status {
            TierStatus::Accepted => has_accepted = true,
            TierStatus::Failed if has_accepted => {
                return Err(invalid("qualification evidence is non-monotonic"));
            }
            TierStatus::Failed => {}
            _ => return Err(invalid("qualification evidence is not terminal")),
        }
    }

    let Some(accepted) = by_tier
        .values()
        .find(|item| item.status == TierStatus::Accepted)
    else {
        return Ok(None);
    };

    if accepted.tier == Tier::T1 {
        return Ok(Some(QualificationBoundary {
            failing: None,
            accepted: (*accepted).clone(),
        }));
    }

    let Some(failing) = by_tier.get(&previous_tier(accepted.tier)) else {
        return Ok(None);
    };
    if failing.status != TierStatus::Failed {
        return Ok(None);
    }

    Ok(Some(QualificationBoundary {
        failing: Some((*failing).clone()),
        accepted: (*accepted).clone(),
    }))
}

fn validate_policy(policy: &QualificationPolicy) -> Result<(), SkillEvalError> {
    if policy.repeats_per_case == 0
        || policy.minimum_score > 10
        || !policy.confidence_level.is_finite()
        || policy.confidence_level <= 0.0
        || policy.confidence_level >= 1.0
        || !policy.noninferiority_margin.is_finite()
        || policy.noninferiority_margin < 0.0
    {
        return Err(invalid(
            "qualification policy has invalid statistical values",
        ));
    }

    let unique_tiers = policy
        .candidate_tiers
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if unique_tiers.len() != policy.candidate_tiers.len() {
        return Err(invalid(
            "qualification policy has duplicate candidate tiers",
        ));
    }
    Ok(())
}

fn validate_evidence(
    evidence: &TierEvidence,
    policy: &QualificationPolicy,
    expected_role: EvidenceRole,
) -> Result<(), SkillEvalError> {
    let is_expected_tier = match expected_role {
        EvidenceRole::Reference => evidence.tier == policy.reference_tier,
        EvidenceRole::Candidate => policy.candidate_tiers.contains(&evidence.tier),
    };
    if evidence.role != expected_role || !is_expected_tier {
        return Err(invalid("qualification evidence does not match the policy"));
    }
    if evidence.expected_trials == 0 || evidence.completed_trials != evidence.expected_trials {
        return Err(invalid("qualification evidence is incomplete"));
    }
    if evidence.passed_trials > evidence.completed_trials {
        return Err(invalid(
            "qualification evidence has an invalid passed count",
        ));
    }
    if !matches!(evidence.status, TierStatus::Accepted | TierStatus::Failed) {
        return Err(invalid("qualification evidence is not terminal"));
    }
    if !is_valid_interval(&evidence.score) {
        return Err(invalid(
            "qualification evidence has an invalid confidence interval",
        ));
    }
    if expected_role == EvidenceRole::Candidate
        && evidence.status == TierStatus::Accepted
        && evidence.score.lower < f64::from(policy.minimum_score) / 10.0
    {
        return Err(invalid(
            "accepted evidence does not meet the confidence-adjusted minimum score",
        ));
    }
    Ok(())
}

fn is_valid_interval(interval: &ConfidenceInterval) -> bool {
    interval.lower.is_finite()
        && interval.estimate.is_finite()
        && interval.upper.is_finite()
        && interval.lower >= 0.0
        && interval.lower <= interval.estimate
        && interval.estimate <= interval.upper
        && interval.upper <= 1.0
}

fn confidence_interval(
    scores: &[f64],
    confidence_level: f64,
) -> Result<ConfidenceInterval, SkillEvalError> {
    let count = scores.len();
    if count == 0 {
        return Err(invalid("cannot calculate an interval without scores"));
    }
    let estimate = scores.iter().sum::<f64>() / count as f64;
    let variance = if count == 1 {
        0.0
    } else {
        scores
            .iter()
            .map(|score| (score - estimate).powi(2))
            .sum::<f64>()
            / (count - 1) as f64
    };
    let probability = 0.5 + confidence_level / 2.0;
    let critical_value = inverse_standard_normal(probability)?;
    let half_width = critical_value * (variance / count as f64).sqrt();

    Ok(ConfidenceInterval {
        lower: (estimate - half_width).max(0.0),
        estimate,
        upper: (estimate + half_width).min(1.0),
    })
}

fn inverse_standard_normal(probability: f64) -> Result<f64, SkillEvalError> {
    if !probability.is_finite() || probability <= 0.0 || probability >= 1.0 {
        return Err(invalid("confidence level cannot produce a finite interval"));
    }

    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    const LOWER: f64 = 0.024_25;
    const UPPER: f64 = 1.0 - LOWER;

    let value = if probability < LOWER {
        let q = (-2.0 * probability.ln()).sqrt();
        polynomial(q, &C) / polynomial_with_one(q, &D)
    } else if probability <= UPPER {
        let q = probability - 0.5;
        let r = q * q;
        q * polynomial(r, &A) / polynomial_with_one(r, &B)
    } else {
        let q = (-2.0 * (1.0 - probability).ln()).sqrt();
        -polynomial(q, &C) / polynomial_with_one(q, &D)
    };

    if value.is_finite() {
        Ok(value)
    } else {
        Err(invalid("confidence level cannot produce a finite interval"))
    }
}

fn polynomial(value: f64, coefficients: &[f64]) -> f64 {
    coefficients
        .iter()
        .copied()
        .reduce(|result, coefficient| result * value + coefficient)
        .unwrap_or(0.0)
}

fn polynomial_with_one(value: f64, coefficients: &[f64]) -> f64 {
    polynomial(value, coefficients) * value + 1.0
}

fn previous_tier(tier: Tier) -> Tier {
    match tier {
        Tier::T1 => Tier::T1,
        Tier::T2 => Tier::T1,
        Tier::T3 => Tier::T2,
        Tier::T4 => Tier::T3,
        Tier::T5 => Tier::T4,
    }
}

fn empty_usage() -> TrialUsage {
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

fn add_usage(total: &mut TrialUsage, usage: &TrialUsage) -> Result<(), SkillEvalError> {
    *total = TrialUsage {
        input_tokens: checked_add_u64(total.input_tokens, usage.input_tokens)?,
        output_tokens: checked_add_u64(total.output_tokens, usage.output_tokens)?,
        cache_read_tokens: checked_add_u64(total.cache_read_tokens, usage.cache_read_tokens)?,
        cache_write_tokens: checked_add_u64(total.cache_write_tokens, usage.cache_write_tokens)?,
        turns: checked_add_u32(total.turns, usage.turns)?,
        tool_calls: checked_add_u32(total.tool_calls, usage.tool_calls)?,
        elapsed_milliseconds: checked_add_u64(
            total.elapsed_milliseconds,
            usage.elapsed_milliseconds,
        )?,
        cost_millionths_of_dollar: checked_add_u64(
            total.cost_millionths_of_dollar,
            usage.cost_millionths_of_dollar,
        )?,
    };
    Ok(())
}

fn checked_add_u32(left: u32, right: u32) -> Result<u32, SkillEvalError> {
    left.checked_add(right)
        .ok_or_else(|| invalid("usage arithmetic overflow"))
}

fn checked_add_u64(left: u64, right: u64) -> Result<u64, SkillEvalError> {
    left.checked_add(right)
        .ok_or_else(|| invalid("usage arithmetic overflow"))
}

fn invalid(message: &str) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(message.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::model::{
        ArtifactName, CaseId, ConfidenceInterval, EvidenceRole, HarnessIdentity, ModelIdentity,
        QualificationPolicy, QualificationPurpose, Tier, TierEvidence, TierStatus, TrialKey,
        TrialRecord, TrialUsage, TrialVerdict,
    };

    use super::{evaluate_tier, find_boundary};

    #[test]
    fn statistics_aggregates_reference_evidence_without_a_reference_input() {
        let trials = trials(Tier::T4);

        let result = evaluate_tier(EvidenceRole::Reference, &trials, None, &policy()).unwrap();

        assert_eq!(result.role, EvidenceRole::Reference);
        assert_eq!(result.tier, Tier::T4);
        assert_eq!(result.status, TierStatus::Accepted);
        assert_eq!(result.completed_trials, 4);
        assert_eq!(result.expected_trials, 4);
        assert_eq!(result.passed_trials, 4);
        assert_eq!(result.score.estimate, 0.8);
    }

    #[test]
    fn statistics_candidate_requires_reference_evidence() {
        let candidate_trials = trials(Tier::T2);

        assert!(
            evaluate_tier(EvidenceRole::Candidate, &candidate_trials, None, &policy(),).is_err()
        );
        assert!(
            evaluate_tier(
                EvidenceRole::Reference,
                &trials(Tier::T4),
                Some(&reference()),
                &policy(),
            )
            .is_err()
        );

        let mut candidate = reference();
        candidate.role = EvidenceRole::Candidate;
        assert!(
            evaluate_tier(
                EvidenceRole::Candidate,
                &candidate_trials,
                Some(&candidate),
                &policy(),
            )
            .is_err()
        );
    }

    #[test]
    fn statistics_preserves_policy_digest_variation_and_separate_usage() {
        let mut trials = trials(Tier::T2);
        trials[2].harness.tool_policy_digest = "other-policy".to_string();
        trials[3].harness.tool_policy_digest = "other-policy".to_string();
        for trial in &mut trials {
            trial.candidate_usage.input_tokens = 2;
            trial.judge_usage.input_tokens = 3;
        }

        let result = evaluate_tier(
            EvidenceRole::Candidate,
            &trials,
            Some(&reference()),
            &policy(),
        )
        .unwrap();

        assert_eq!(result.harnesses.len(), 2);
        assert_eq!(result.harnesses[0], trials[0].harness);
        assert_eq!(result.harnesses[1], trials[2].harness);
        assert_eq!(result.candidate_usage.input_tokens, 8);
        assert_eq!(result.judge_usage.input_tokens, 12);
        assert_eq!(result.total_usage.input_tokens, 20);
    }

    #[test]
    fn statistics_rejects_incomplete_trials_and_common_identity_drift() {
        let incomplete = vec![trial(Tier::T2, "a", 1, 8)];
        assert!(
            evaluate_tier(
                EvidenceRole::Candidate,
                &incomplete,
                Some(&reference()),
                &policy(),
            )
            .is_err()
        );

        for change in [
            |trial: &mut TrialRecord| trial.model.model = "other".to_string(),
            |trial: &mut TrialRecord| trial.harness.runner_version = "other".to_string(),
            |trial: &mut TrialRecord| trial.harness.pi_version = "other".to_string(),
            |trial: &mut TrialRecord| trial.harness.artifact_revision = "other".to_string(),
        ] {
            let mut mixed = vec![trial(Tier::T2, "a", 1, 8), trial(Tier::T2, "a", 2, 8)];
            change(&mut mixed[1]);
            assert!(
                evaluate_tier(
                    EvidenceRole::Candidate,
                    &mixed,
                    Some(&reference()),
                    &policy(),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn statistics_retains_score_noninferiority_and_catastrophic_rules() {
        let mut low = vec![trial(Tier::T2, "a", 1, 6), trial(Tier::T2, "a", 2, 6)];
        let result =
            evaluate_tier(EvidenceRole::Candidate, &low, Some(&reference()), &policy()).unwrap();
        assert_eq!(result.status, TierStatus::Failed);

        for trial in &mut low {
            trial.verdict.score = 7;
        }
        let mut strong_reference = reference();
        strong_reference.score.lower = 0.9;
        strong_reference.score.estimate = 0.9;
        strong_reference.score.upper = 0.9;
        let result = evaluate_tier(
            EvidenceRole::Candidate,
            &low,
            Some(&strong_reference),
            &policy(),
        )
        .unwrap();
        assert_eq!(result.status, TierStatus::Failed);

        let mut catastrophic = vec![trial(Tier::T2, "a", 1, 10), trial(Tier::T2, "a", 2, 10)];
        catastrophic[1].verdict.is_catastrophic = true;
        let result = evaluate_tier(
            EvidenceRole::Candidate,
            &catastrophic,
            Some(&reference()),
            &policy(),
        )
        .unwrap();
        assert_eq!(result.status, TierStatus::Failed);
        assert_eq!(result.passed_trials, 1);
    }

    #[test]
    fn statistics_usage_overflow_fails_for_each_sum() {
        let mut candidate = vec![trial(Tier::T2, "a", 1, 8), trial(Tier::T2, "a", 2, 8)];
        candidate[0].candidate_usage.input_tokens = u64::MAX;
        assert_evaluation_fails(candidate);

        let mut judge = vec![trial(Tier::T2, "a", 1, 8), trial(Tier::T2, "a", 2, 8)];
        judge[0].judge_usage.input_tokens = u64::MAX;
        assert_evaluation_fails(judge);

        let mut total = vec![trial(Tier::T2, "a", 1, 8), trial(Tier::T2, "a", 2, 8)];
        total[0].candidate_usage.input_tokens = u64::MAX;
        total[1].candidate_usage.input_tokens = 0;
        total[0].judge_usage.input_tokens = 0;
        total[1].judge_usage.input_tokens = 1;
        assert_evaluation_fails(total);
    }

    #[test]
    fn statistics_finds_t1_and_immediate_lower_boundaries() {
        let failing = evidence(Tier::T1, TierStatus::Failed);
        let accepted = evidence(Tier::T2, TierStatus::Accepted);

        let boundary = find_boundary(&[accepted.clone(), failing.clone()], &policy()).unwrap();
        assert_eq!(boundary.unwrap().failing, Some(failing));
        assert!(find_boundary(&[accepted], &policy()).unwrap().is_none());

        let t1 = evidence(Tier::T1, TierStatus::Accepted);
        let boundary = find_boundary(std::slice::from_ref(&t1), &policy())
            .unwrap()
            .unwrap();
        assert_eq!(boundary.accepted, t1);
        assert_eq!(boundary.failing, None);
    }

    #[test]
    fn statistics_stops_on_non_monotonic_evidence() {
        let cheaper = evidence(Tier::T1, TierStatus::Accepted);
        let capable = evidence(Tier::T2, TierStatus::Failed);

        assert!(find_boundary(&[capable, cheaper], &policy()).is_err());
    }

    fn assert_evaluation_fails(trials: Vec<TrialRecord>) {
        assert!(
            evaluate_tier(
                EvidenceRole::Candidate,
                &trials,
                Some(&reference()),
                &policy(),
            )
            .is_err()
        );
    }

    fn policy() -> QualificationPolicy {
        QualificationPolicy {
            purpose: QualificationPurpose::Artifact,
            candidate_tiers: vec![Tier::T1, Tier::T2, Tier::T3],
            reference_tier: Tier::T4,
            judge_tier: Tier::T5,
            repeats_per_case: 2,
            minimum_score: 7,
            noninferiority_margin: 0.1,
            confidence_level: 0.95,
        }
    }

    fn reference() -> TierEvidence {
        evidence_with_role(EvidenceRole::Reference, Tier::T4, TierStatus::Accepted)
    }

    fn evidence(tier: Tier, status: TierStatus) -> TierEvidence {
        evidence_with_role(EvidenceRole::Candidate, tier, status)
    }

    fn evidence_with_role(role: EvidenceRole, tier: Tier, status: TierStatus) -> TierEvidence {
        let total_usage = usage(2);
        TierEvidence {
            role,
            tier,
            model: model(tier),
            harnesses: vec![harness("policy")],
            status,
            completed_trials: 2,
            expected_trials: 2,
            passed_trials: u32::from(status == TierStatus::Accepted) * 2,
            score: ConfidenceInterval {
                lower: 0.8,
                estimate: 0.8,
                upper: 0.8,
            },
            candidate_usage: usage(1),
            judge_usage: usage(1),
            total_usage,
        }
    }

    fn trials(tier: Tier) -> Vec<TrialRecord> {
        vec![
            trial(tier, "a", 1, 8),
            trial(tier, "a", 2, 8),
            trial(tier, "b", 1, 8),
            trial(tier, "b", 2, 8),
        ]
    }

    fn trial(tier: Tier, case: &str, attempt: u16, score: u8) -> TrialRecord {
        TrialRecord {
            key: TrialKey {
                artifact: ArtifactName("artifact".to_string()),
                tier,
                case: CaseId(case.to_string()),
                attempt,
            },
            model: model(tier),
            harness: harness("policy"),
            artifact_path: PathBuf::from("artifact.txt"),
            transcript_path: PathBuf::from("transcript.jsonl"),
            candidate_usage: usage(1),
            judge_model: model(Tier::T5),
            judge_usage: usage(1),
            verdict: TrialVerdict {
                score,
                is_catastrophic: false,
                failure_mode: None,
                checks: Vec::new(),
            },
        }
    }

    fn model(tier: Tier) -> ModelIdentity {
        ModelIdentity {
            tier,
            provider: "provider".to_string(),
            model: "model".to_string(),
            thinking: "low".to_string(),
        }
    }

    fn harness(tool_policy_digest: &str) -> HarnessIdentity {
        HarnessIdentity {
            runner_version: "1".to_string(),
            pi_version: "1".to_string(),
            artifact_revision: "revision".to_string(),
            tool_policy_digest: tool_policy_digest.to_string(),
        }
    }

    fn usage(input_tokens: u64) -> TrialUsage {
        TrialUsage {
            input_tokens,
            output_tokens: input_tokens,
            cache_read_tokens: input_tokens,
            cache_write_tokens: input_tokens,
            turns: input_tokens as u32,
            tool_calls: input_tokens as u32,
            elapsed_milliseconds: input_tokens,
            cost_millionths_of_dollar: input_tokens,
        }
    }
}
