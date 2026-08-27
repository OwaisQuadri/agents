use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    CaseId, CheckStatus, ConfidenceInterval, EvidenceRole, FrontierCaseGroup, FrontierCellEvidence,
    FrontierCellStatus, FrontierEntrant, FrontierModelProgress, FrontierModelReport,
    FrontierPolicy, FrontierPoolMembership, FrontierScore, FrontierTierSuite, ModelIdentity,
    PoolEntrant, PoolEntrantEvidence, PoolPolicy, PoolStage, QualificationBoundary,
    QualificationPolicy, RankedPool, SkillEvalError, ThinkingDecision, Tier, TierEvidence,
    TierStatus, TrialRecord, TrialUsage,
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
    let minimum_reliability_basis_points = match stage {
        PoolStage::Calibration => policy.calibration_minimum_reliability_basis_points,
        PoolStage::Qualification => policy.qualification_minimum_reliability_basis_points,
    };
    let is_passing = mean_score >= f64::from(policy.minimum_score) / 10.0
        && reliability_basis_points >= u64::from(minimum_reliability_basis_points)
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
        || policy.calibration_minimum_reliability_basis_points > 10_000
        || policy.qualification_minimum_reliability_basis_points != 10_000
        || policy.spending_limit_millionths_of_dollar == 0
        || !policy.is_provider_limit_enforced
    {
        return Err(invalid("pool policy has invalid calibration values"));
    }
    Ok(())
}

pub(crate) fn select_qualification_thinking_level(
    entrant: &PoolEntrant,
    calibration: &[PoolEntrantEvidence],
    qualification: &[PoolEntrantEvidence],
) -> Result<ThinkingDecision, SkillEvalError> {
    validate_thinking_entrant(entrant)?;
    if calibration.iter().any(|item| {
        item.harnesses.len() != 5 || item.completed_trials != 5 || item.expected_trials != 5
    }) {
        return Err(invalid(
            "qualification requires calibration evidence from five complete cases",
        ));
    }
    let calibrated = validate_thinking_evidence(entrant, calibration)?;
    if calibrated.len() != entrant.thinking_levels.len()
        || calibrated
            .iter()
            .enumerate()
            .any(|(expected, (actual, _))| expected != *actual)
    {
        return Err(invalid(
            "qualification requires complete cheapest-to-strongest calibration evidence",
        ));
    }
    let attempted = validate_qualification_thinking_evidence(entrant, qualification)?;
    let Some(retained_index) = retained_lower_index(entrant)? else {
        return select_normal_qualification_thinking_level(entrant, &calibrated, &attempted);
    };
    select_retained_qualification_thinking_level(entrant, &calibrated, &attempted, retained_index)
}

fn select_normal_qualification_thinking_level(
    entrant: &PoolEntrant,
    calibrated: &[(usize, bool)],
    attempted: &[(usize, bool)],
) -> Result<ThinkingDecision, SkillEvalError> {
    let start_index = calibrated
        .iter()
        .position(|(_, is_passing)| *is_passing)
        .ok_or_else(|| invalid("qualification requires at least one calibration pass"))?;
    validate_contiguous_qualification_attempts(attempted, start_index)?;
    if attempted
        .last()
        .is_some_and(|(_, is_reliable)| *is_reliable)
    {
        return complete_thinking_decision(
            entrant,
            attempted.last().map(|(index, _)| *index),
            None,
        );
    }
    let next_index = start_index
        .checked_add(attempted.len())
        .ok_or_else(|| invalid("qualification thinking evidence index overflow"))?;
    if next_index < entrant.thinking_levels.len() {
        next_thinking_decision(next_index, None)
    } else {
        complete_thinking_decision(entrant, None, None)
    }
}

fn select_retained_qualification_thinking_level(
    entrant: &PoolEntrant,
    calibrated: &[(usize, bool)],
    attempted: &[(usize, bool)],
    retained_index: usize,
) -> Result<ThinkingDecision, SkillEvalError> {
    let is_lower_screen_passing = calibrated[retained_index].1;
    let lower_attempt = attempted
        .first()
        .filter(|(index, _)| *index == retained_index);
    if !is_lower_screen_passing && attempted.iter().any(|(index, _)| *index == retained_index) {
        return Err(invalid(
            "retained lower qualification evidence requires a passing lower screen",
        ));
    }
    if is_lower_screen_passing && lower_attempt.is_none() {
        if !attempted.is_empty() {
            return Err(invalid(
                "retained lower qualification evidence must precede stronger evidence",
            ));
        }
        return next_thinking_decision(retained_index, None);
    }

    let retained_lower = lower_attempt
        .filter(|(_, is_reliable)| *is_reliable)
        .map(|_| thinking_identity(entrant, retained_index))
        .transpose()?;
    let stronger_attempts = if lower_attempt.is_some() {
        &attempted[1..]
    } else {
        attempted
    };
    if stronger_attempts
        .iter()
        .any(|(index, _)| *index <= retained_index)
    {
        return Err(invalid(
            "retained qualification evidence uses a wrong lower level",
        ));
    }
    let stronger_start = calibrated
        .iter()
        .skip(retained_index + 1)
        .find(|(_, is_passing)| *is_passing)
        .map(|(index, _)| *index);
    let Some(stronger_start) = stronger_start else {
        if stronger_attempts.is_empty() {
            return complete_thinking_decision(entrant, None, retained_lower);
        }
        return Err(invalid(
            "retained qualification evidence has no stronger screening pass",
        ));
    };
    validate_contiguous_qualification_attempts(stronger_attempts, stronger_start)?;
    if stronger_attempts
        .last()
        .is_some_and(|(_, is_reliable)| *is_reliable)
    {
        return complete_thinking_decision(
            entrant,
            stronger_attempts.last().map(|(index, _)| *index),
            retained_lower,
        );
    }
    let next_index = stronger_start
        .checked_add(stronger_attempts.len())
        .ok_or_else(|| invalid("qualification thinking evidence index overflow"))?;
    if next_index < entrant.thinking_levels.len() {
        next_thinking_decision(next_index, retained_lower)
    } else {
        complete_thinking_decision(entrant, None, retained_lower)
    }
}

fn validate_contiguous_qualification_attempts(
    attempted: &[(usize, bool)],
    start_index: usize,
) -> Result<(), SkillEvalError> {
    for (offset, &(index, is_fully_reliable)) in attempted.iter().enumerate() {
        let expected = start_index
            .checked_add(offset)
            .ok_or_else(|| invalid("qualification thinking evidence index overflow"))?;
        if index != expected {
            return Err(invalid(
                "qualification evidence must start at the cheapest calibration pass and advance contiguously",
            ));
        }
        if is_fully_reliable && offset + 1 != attempted.len() {
            return Err(invalid(
                "qualification evidence cannot continue after a fully reliable result",
            ));
        }
    }
    Ok(())
}

pub(crate) fn select_thinking_level(
    entrant: &PoolEntrant,
    evidence: &[PoolEntrantEvidence],
) -> Result<ThinkingDecision, SkillEvalError> {
    validate_thinking_entrant(entrant)?;
    let attempted = validate_thinking_evidence(entrant, evidence)?;
    for (expected_index, &(index, _)) in attempted.iter().enumerate() {
        if index != expected_index {
            return Err(invalid(
                "thinking evidence must be contiguous from the cheapest level",
            ));
        }
    }

    if attempted.len() < entrant.thinking_levels.len() {
        return next_thinking_decision(attempted.len(), None);
    }

    let selected_index = attempted.iter().position(|(_, is_passing)| *is_passing);
    complete_thinking_decision(entrant, selected_index, None)
}

fn validate_thinking_entrant(entrant: &PoolEntrant) -> Result<(), SkillEvalError> {
    const PI_THINKING_LEVELS: [&str; 7] =
        ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

    if entrant.thinking_levels.is_empty()
        || entrant.thinking_levels.len() > PI_THINKING_LEVELS.len()
    {
        return Err(invalid(
            "pool entrant must declare one to seven thinking levels",
        ));
    }

    let mut previous_rank = None;
    let mut start_index = None;
    for (index, level) in entrant.thinking_levels.iter().enumerate() {
        let rank = PI_THINKING_LEVELS
            .iter()
            .position(|supported| *supported == level)
            .ok_or_else(|| invalid("pool entrant has an unsupported thinking level"))?;
        if previous_rank.is_some_and(|previous| rank <= previous) {
            return Err(invalid(
                "pool entrant thinking levels must be unique and ordered cheapest to strongest",
            ));
        }
        if level == &entrant.model.thinking && start_index.replace(index).is_some() {
            return Err(invalid(
                "pool entrant starting thinking must appear exactly once",
            ));
        }
        previous_rank = Some(rank);
    }

    start_index
        .ok_or_else(|| invalid("pool entrant starting thinking must appear exactly once"))?;
    retained_lower_index(entrant).map(|_| ())
}

fn retained_lower_index(entrant: &PoolEntrant) -> Result<Option<usize>, SkillEvalError> {
    let Some(retained) = &entrant.retained_lower_thinking_level else {
        return Ok(None);
    };
    let matching = entrant
        .thinking_levels
        .iter()
        .enumerate()
        .filter(|(_, level)| *level == retained)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err(invalid(
            "retained lower thinking level must appear exactly once",
        ));
    }
    let index = matching[0];
    if index + 1 >= entrant.thinking_levels.len() {
        return Err(invalid(
            "retained lower thinking level must be below a declared stronger level",
        ));
    }
    Ok(Some(index))
}

fn validate_thinking_evidence(
    entrant: &PoolEntrant,
    evidence: &[PoolEntrantEvidence],
) -> Result<Vec<(usize, bool)>, SkillEvalError> {
    validate_thinking_stage_evidence(entrant, evidence, PoolStage::Calibration, |item| {
        item.is_passing
    })
}

fn validate_qualification_thinking_evidence(
    entrant: &PoolEntrant,
    evidence: &[PoolEntrantEvidence],
) -> Result<Vec<(usize, bool)>, SkillEvalError> {
    if evidence.iter().any(|item| {
        item.harnesses.len() != 5 || item.completed_trials != 15 || item.expected_trials != 15
    }) {
        return Err(invalid(
            "qualification thinking evidence must contain five complete cases and 15 trials",
        ));
    }
    validate_thinking_stage_evidence(entrant, evidence, PoolStage::Qualification, |item| {
        is_fully_reliable(item)
    })
}

fn validate_thinking_stage_evidence(
    entrant: &PoolEntrant,
    evidence: &[PoolEntrantEvidence],
    stage: PoolStage,
    result: impl Fn(&PoolEntrantEvidence) -> bool,
) -> Result<Vec<(usize, bool)>, SkillEvalError> {
    let mut attempted = Vec::with_capacity(evidence.len());
    let mut attempted_indices = BTreeSet::new();

    for item in evidence {
        if item.stage != stage {
            return Err(invalid("thinking evidence uses the wrong stage"));
        }
        if item.requested_model != item.effective_model {
            return Err(invalid(
                "thinking evidence requested and effective identities differ",
            ));
        }
        if item.requested_model.tier != entrant.model.tier
            || item.requested_model.provider != entrant.model.provider
            || item.requested_model.model != entrant.model.model
        {
            return Err(invalid("thinking evidence belongs to a foreign model"));
        }
        let index = entrant
            .thinking_levels
            .iter()
            .position(|level| level == &item.requested_model.thinking)
            .ok_or_else(|| invalid("thinking evidence uses an undeclared thinking level"))?;
        if !attempted_indices.insert(index) {
            return Err(invalid("thinking evidence contains a duplicate level"));
        }
        if is_same_model(&item.judge_model, &item.effective_model) {
            return Err(invalid("thinking evidence candidate cannot judge itself"));
        }
        if item.harnesses.is_empty()
            || item.expected_trials == 0
            || item.completed_trials != item.expected_trials
            || item.failed_trials > item.completed_trials
            || item.catastrophic_trials > item.completed_trials
            || (item.is_passing && item.catastrophic_trials != 0)
        {
            return Err(invalid("thinking evidence has impossible trial structure"));
        }
        if !is_valid_interval(&item.score) {
            return Err(invalid("thinking evidence has invalid score metrics"));
        }
        let mut expected_total_usage = item.candidate_usage.clone();
        add_usage(&mut expected_total_usage, &item.judge_usage)?;
        if expected_total_usage != item.total_usage {
            return Err(invalid("thinking evidence has inconsistent total usage"));
        }
        attempted.push((index, result(item)));
    }

    Ok(attempted)
}

fn is_fully_reliable(evidence: &PoolEntrantEvidence) -> bool {
    evidence.is_passing
        && evidence.completed_trials == 15
        && evidence.expected_trials == 15
        && evidence.failed_trials == 0
        && evidence.catastrophic_trials == 0
}

fn next_thinking_decision(
    index: usize,
    retained_lower: Option<ModelIdentity>,
) -> Result<ThinkingDecision, SkillEvalError> {
    let next_thinking_index =
        u8::try_from(index).map_err(|_| invalid("thinking evidence index overflow"))?;
    Ok(ThinkingDecision {
        selected: None,
        retained_lower,
        next_thinking_index: Some(next_thinking_index),
        is_complete: false,
    })
}

fn thinking_identity(entrant: &PoolEntrant, index: usize) -> Result<ModelIdentity, SkillEvalError> {
    let thinking = entrant
        .thinking_levels
        .get(index)
        .ok_or_else(|| invalid("thinking selection index overflow"))?;
    let mut model = entrant.model.clone();
    model.thinking.clone_from(thinking);
    Ok(model)
}

fn complete_thinking_decision(
    entrant: &PoolEntrant,
    selected_index: Option<usize>,
    retained_lower: Option<ModelIdentity>,
) -> Result<ThinkingDecision, SkillEvalError> {
    let selected = selected_index
        .map(|index| thinking_identity(entrant, index))
        .transpose()?;
    Ok(ThinkingDecision {
        selected,
        retained_lower,
        next_thinking_index: None,
        is_complete: true,
    })
}

pub(crate) fn rank_pool(
    tier: Tier,
    entrants: &[PoolEntrant],
    calibration: &[PoolEntrantEvidence],
    qualification: &[PoolEntrantEvidence],
    policy: &PoolPolicy,
) -> Result<RankedPool, SkillEvalError> {
    validate_calibration_policy(policy)?;
    validate_pool_stage(
        calibration,
        tier,
        PoolStage::Calibration,
        policy.calibration_repeats_per_case,
        policy,
    )?;
    validate_pool_stage(
        qualification,
        tier,
        PoolStage::Qualification,
        policy.qualification_repeats_per_case,
        policy,
    )?;
    if entrants.len() < 3 {
        return Err(invalid(
            "pool calibration must contain at least three complete models",
        ));
    }
    let mut configured_bases = BTreeSet::new();
    let mut configured_calibration = Vec::new();
    for entrant in entrants {
        validate_thinking_entrant(entrant)?;
        if entrant.model.tier != tier
            || !configured_bases.insert((
                entrant.model.provider.as_str(),
                entrant.model.model.as_str(),
            ))
        {
            return Err(invalid(
                "pool entrants contain a foreign or duplicate model",
            ));
        }
        configured_calibration.extend(entrant.thinking_levels.iter().map(|thinking| {
            let mut identity = entrant.model.clone();
            identity.thinking.clone_from(thinking);
            identity
        }));
    }
    if calibration
        .iter()
        .map(|item| &item.requested_model)
        .ne(configured_calibration.iter())
    {
        return Err(invalid(
            "pool calibration does not match the complete frozen entrant plan",
        ));
    }
    let mut previous_qualification_entrant = None;
    for item in qualification {
        let entrant_index = entrants
            .iter()
            .position(|entrant| is_same_base_identity(&item.requested_model, &entrant.model))
            .ok_or_else(|| invalid("pool qualification contains a foreign model"))?;
        if previous_qualification_entrant.is_some_and(|previous| entrant_index < previous) {
            return Err(invalid(
                "pool qualification does not match frozen entrant order",
            ));
        }
        previous_qualification_entrant = Some(entrant_index);
    }

    let mut promoted = Vec::new();
    let mut retained_lower_routes = Vec::new();
    let mut finalists = Vec::new();
    let mut is_every_walk_complete = true;
    let mut is_qualification_gap = false;
    for entrant in entrants {
        let calibration_evidence = evidence_for_model(calibration, &entrant.model);
        let screening = select_thinking_level(entrant, &calibration_evidence)?;
        if !screening.is_complete {
            return Err(invalid("pool calibration evidence is incomplete"));
        }
        let qualification_evidence = evidence_for_model(qualification, &entrant.model);
        if is_qualification_gap && !qualification_evidence.is_empty() {
            return Err(invalid(
                "pool qualification skips an incomplete frozen entrant",
            ));
        }
        let start_index = qualification_start_index(entrant, &calibration_evidence)?;
        let is_lower_goal = retained_lower_index(entrant)?
            .is_some_and(|index| calibration_evidence[index].is_passing);
        if start_index.is_none() && !is_lower_goal {
            if !qualification_evidence.is_empty() {
                return Err(invalid(
                    "pool qualification contains a model without an authorized calibration pass",
                ));
            }
            continue;
        }
        if let Some(index) = start_index {
            promoted.push(thinking_identity(entrant, index)?);
        }
        let decision =
            rank_qualification_decision(entrant, &calibration_evidence, &qualification_evidence)?;
        is_every_walk_complete &= decision.is_complete;
        is_qualification_gap |= !decision.is_complete;
        if let Some(retained) = decision.retained_lower {
            let evidence = qualification_evidence
                .iter()
                .find(|item| item.requested_model == retained)
                .ok_or_else(|| invalid("retained lower identity has no qualification evidence"))?;
            if !is_fully_reliable(evidence) {
                return Err(invalid("retained lower identity is not fully reliable"));
            }
            retained_lower_routes.push(retained);
        }
        if let Some(selected) = decision.selected {
            let evidence = qualification_evidence
                .iter()
                .find(|item| item.requested_model == selected)
                .ok_or_else(|| invalid("ranked identity has no qualification evidence"))?;
            if !is_fully_reliable(evidence) {
                return Err(invalid("ranked identity is not fully reliable"));
            }
            finalists.push(evidence.clone());
        }
    }
    if qualification.iter().any(|item| {
        !entrants
            .iter()
            .any(|entrant| is_same_base_identity(&item.requested_model, &entrant.model))
    }) {
        return Err(invalid("pool qualification contains a foreign model"));
    }

    let ranked = if is_every_walk_complete {
        finalists.sort_by(compare_pool_evidence);
        finalists
            .into_iter()
            .map(|evidence| evidence.requested_model.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let is_complete = is_every_walk_complete && ranked.len() >= usize::from(policy.promotion_count);

    Ok(RankedPool {
        tier,
        calibration: calibration.to_vec(),
        thinking_selections: Vec::new(),
        retained_lower_routes,
        promoted,
        qualification: qualification.to_vec(),
        ranked,
        is_complete,
    })
}

fn rank_qualification_decision(
    entrant: &PoolEntrant,
    calibration: &[PoolEntrantEvidence],
    qualification: &[PoolEntrantEvidence],
) -> Result<ThinkingDecision, SkillEvalError> {
    if calibration.iter().all(|item| {
        item.harnesses.len() == 5 && item.completed_trials == 5 && item.expected_trials == 5
    }) {
        return select_qualification_thinking_level(entrant, calibration, qualification);
    }
    if entrant.thinking_levels.len() != 1 || entrant.retained_lower_thinking_level.is_some() {
        return Err(invalid(
            "qualification requires calibration evidence from five complete cases",
        ));
    }
    let attempted = validate_qualification_thinking_evidence(entrant, qualification)?;
    match attempted.as_slice() {
        [] => next_thinking_decision(0, None),
        [(0, true)] => complete_thinking_decision(entrant, Some(0), None),
        [(0, false)] => complete_thinking_decision(entrant, None, None),
        _ => Err(invalid("fixed qualification evidence has invalid shape")),
    }
}

pub(crate) fn qualification_start_index(
    entrant: &PoolEntrant,
    calibration: &[PoolEntrantEvidence],
) -> Result<Option<usize>, SkillEvalError> {
    let calibrated = validate_thinking_evidence(entrant, calibration)?;
    if calibrated.len() != entrant.thinking_levels.len() {
        return Err(invalid(
            "qualification requires complete calibration evidence",
        ));
    }
    let start = retained_lower_index(entrant)?.map_or(0, |index| index + 1);
    Ok(calibrated
        .iter()
        .skip(start)
        .find(|(_, is_passing)| *is_passing)
        .map(|(index, _)| *index))
}

fn evidence_for_model(
    evidence: &[PoolEntrantEvidence],
    model: &ModelIdentity,
) -> Vec<PoolEntrantEvidence> {
    evidence
        .iter()
        .filter(|item| is_same_base_identity(&item.requested_model, model))
        .cloned()
        .collect()
}

fn is_same_base_identity(left: &ModelIdentity, right: &ModelIdentity) -> bool {
    left.tier == right.tier && left.provider == right.provider && left.model == right.model
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
        let minimum_reliability_basis_points = match item.stage {
            PoolStage::Calibration => policy.calibration_minimum_reliability_basis_points,
            PoolStage::Qualification => policy.qualification_minimum_reliability_basis_points,
        };
        let reliability_floor = u64::from(item.completed_trials)
            .checked_mul(u64::from(minimum_reliability_basis_points))
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

// TODO(AGNT-0032.T143): Reverify the implemented frontier statistics after D-144 closes.
/// Evaluates one exact model-tier-thinking cell under the frozen frontier policy.
///
/// The inputs are one tier suite, exact model, trial records, and policy. The output is cell evidence.
///
/// # Errors
///
/// Returns an error for incomplete groups, invalid trials, critical failure, or statistical overflow.
pub(crate) fn evaluate_frontier_cell(
    suite: &FrontierTierSuite,
    model: &ModelIdentity,
    trials: &[TrialRecord],
    policy: &FrontierPolicy,
) -> Result<FrontierCellEvidence, SkillEvalError> {
    validate_frontier_policy(policy)?;
    let groups = [
        FrontierCaseGroup::Normal,
        FrontierCaseGroup::Edge,
        FrontierCaseGroup::Adversarial,
        FrontierCaseGroup::Critical,
    ];
    if suite.group_weights_basis_points.len() != groups.len()
        || groups.iter().any(|group| {
            suite
                .group_weights_basis_points
                .get(group)
                .is_none_or(|weight| *weight == 0)
        })
    {
        return Err(invalid(
            "frontier suite must assign every case group a weight",
        ));
    }
    let weight_total = suite
        .group_weights_basis_points
        .values()
        .try_fold(0_u16, |total, weight| total.checked_add(*weight))
        .ok_or_else(|| invalid("frontier group weight overflow"))?;
    if weight_total != 10_000 {
        return Err(invalid(
            "frontier group weights must total 10000 basis points",
        ));
    }

    let mut expected = BTreeMap::new();
    let mut group_case_counts = BTreeMap::new();
    for case in &suite.cases {
        let artifact = case
            .artifact_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("frontier artifact path has no valid name"))?
            .to_string();
        let key = (artifact, case.case.clone());
        if expected.insert(key, case).is_some() {
            return Err(invalid("frontier suite contains a duplicate case"));
        }
        let count = group_case_counts.entry(case.group).or_insert(0_u32);
        *count = count
            .checked_add(1)
            .ok_or_else(|| invalid("frontier group case count overflow"))?;
    }
    if suite.cases.is_empty()
        || groups
            .iter()
            .any(|group| !group_case_counts.contains_key(group))
    {
        return Err(invalid("frontier suite is missing a case group"));
    }
    let first = trials
        .first()
        .ok_or_else(|| invalid("frontier trial set is empty"))?;
    let route_index = first.key.route_index;
    let judge = &first.judge_model;
    if first.model != *model || is_same_model(judge, model) {
        return Err(invalid(
            "frontier trial identity does not match the requested route",
        ));
    }

    let mut attempts_by_case = expected
        .keys()
        .cloned()
        .map(|key| (key, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut harness_by_case = BTreeMap::new();
    let mut outcomes = BTreeMap::<FrontierCaseGroup, BTreeMap<(String, CaseId), Vec<bool>>>::new();
    let mut total_usage = empty_usage();
    let mut failed_trials = 0_u32;
    for trial in trials {
        if trial.model != *model
            || trial.key.tier != model.tier
            || trial.key.route_index != route_index
            || trial.judge_model != *judge
            || is_same_model(&trial.judge_model, model)
        {
            return Err(invalid("frontier trial set has identity drift"));
        }
        if trial.verdict.score > 10 {
            return Err(invalid("frontier trial score is outside 0 through 10"));
        }
        let key = (trial.key.artifact.0.clone(), trial.key.case.clone());
        let case = expected
            .get(&key)
            .ok_or_else(|| invalid("frontier trial set contains a foreign case"))?;
        if trial.harness.artifact_revision != case.artifact_revision {
            return Err(invalid("frontier trial artifact revision drifted"));
        }
        let attempts = attempts_by_case
            .get_mut(&key)
            .ok_or_else(|| invalid("frontier trial set contains a foreign case"))?;
        if !attempts.insert(trial.key.attempt) {
            return Err(invalid("frontier trial set contains a duplicate attempt"));
        }
        match harness_by_case.entry(key.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(trial.harness.clone());
            }
            std::collections::btree_map::Entry::Occupied(entry)
                if entry.get() != &trial.harness =>
            {
                return Err(invalid("frontier case harness identity drifted"));
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
        }
        if trial.harness.runner_version != first.harness.runner_version
            || trial.harness.pi_version != first.harness.pi_version
        {
            return Err(invalid("frontier common harness identity drifted"));
        }
        add_usage(&mut total_usage, &trial.candidate_usage)?;
        add_usage(&mut total_usage, &trial.judge_usage)?;
        let is_failed_check = trial
            .verdict
            .checks
            .iter()
            .any(|check| check.status == CheckStatus::Failed);
        let is_passing = trial.verdict.score >= policy.minimum_trial_score
            && !trial.verdict.is_catastrophic
            && !is_failed_check;
        if !is_passing {
            failed_trials = failed_trials
                .checked_add(1)
                .ok_or_else(|| invalid("frontier failed trial count overflow"))?;
        }
        outcomes
            .entry(case.group)
            .or_default()
            .entry(key)
            .or_default()
            .push(is_passing);
    }

    let attempts_per_case = attempts_by_case
        .values()
        .next()
        .map(BTreeSet::len)
        .ok_or_else(|| invalid("frontier suite is empty"))?;
    if ![1, 3, 5].contains(&attempts_per_case) {
        return Err(invalid(
            "frontier trials must use exactly 1, 3, or 5 attempts per case",
        ));
    }
    let required_attempts = (1..=u16::try_from(attempts_per_case)
        .map_err(|_| invalid("frontier attempt count overflow"))?)
        .collect::<BTreeSet<_>>();
    if attempts_by_case
        .values()
        .any(|attempts| attempts != &required_attempts)
    {
        return Err(invalid(
            "frontier trial attempts are incomplete or malformed",
        ));
    }
    let expected_trials = u32::try_from(suite.cases.len())
        .ok()
        .and_then(|count| count.checked_mul(u32::try_from(attempts_per_case).ok()?))
        .ok_or_else(|| invalid("frontier expected trial count overflow"))?;
    let completed_trials = u32::try_from(trials.len())
        .map_err(|_| invalid("frontier completed trial count overflow"))?;
    if completed_trials != expected_trials {
        return Err(invalid("frontier trial set is incomplete"));
    }

    let weighted_pass_basis_points = frontier_weighted_rate(&outcomes, suite)?;
    let lower_bound_basis_points = frontier_bootstrap_lower_bound(&outcomes, suite, model, policy)?;
    let critical = outcomes
        .get(&FrontierCaseGroup::Critical)
        .ok_or_else(|| invalid("frontier trial set is missing the critical group"))?;
    let critical_expected_trials = critical.values().try_fold(0_u32, |total, values| {
        let count = u32::try_from(values.len())
            .map_err(|_| invalid("frontier critical trial count overflow"))?;
        total
            .checked_add(count)
            .ok_or_else(|| invalid("frontier critical trial count overflow"))
    })?;
    let critical_passed_trials = critical.values().try_fold(0_u32, |total, values| {
        let count = u32::try_from(values.iter().filter(|is_passing| **is_passing).count())
            .map_err(|_| invalid("frontier critical pass count overflow"))?;
        total
            .checked_add(count)
            .ok_or_else(|| invalid("frontier critical pass count overflow"))
    })?;
    let is_group_coverage_complete = groups.iter().all(|group| outcomes.contains_key(group));
    let score = FrontierScore {
        weighted_pass_basis_points,
        lower_bound_basis_points,
        critical_passed_trials,
        critical_expected_trials,
        is_group_coverage_complete,
    };
    let is_estimate_passing = weighted_pass_basis_points
        >= policy.minimum_weighted_pass_basis_points
        && critical_passed_trials == critical_expected_trials
        && is_group_coverage_complete;
    let status = if !is_estimate_passing {
        FrontierCellStatus::Failed
    } else if lower_bound_basis_points >= policy.minimum_lower_bound_basis_points {
        FrontierCellStatus::Passed
    } else if attempts_per_case < usize::from(policy.maximum_trials_per_case) {
        FrontierCellStatus::Pending
    } else {
        FrontierCellStatus::Indeterminate
    };

    Ok(FrontierCellEvidence {
        model: model.clone(),
        status,
        completed_trials,
        expected_trials,
        failed_trials,
        score: Some(score),
        total_usage,
    })
}

fn validate_frontier_policy(policy: &FrontierPolicy) -> Result<(), SkillEvalError> {
    if policy.screening_trials_per_case != 1
        || policy.confirmation_trials_per_case != 3
        || policy.maximum_trials_per_case != 5
        || policy.minimum_trial_score > 10
        || policy.minimum_weighted_pass_basis_points != 8_500
        || policy.minimum_lower_bound_basis_points != 8_000
        || policy.confidence_level_basis_points != 9_500
        || policy.confidence_resamples == 0
    {
        return Err(invalid("frontier policy has invalid statistical values"));
    }
    Ok(())
}

fn frontier_weighted_rate(
    outcomes: &BTreeMap<FrontierCaseGroup, BTreeMap<(String, CaseId), Vec<bool>>>,
    suite: &FrontierTierSuite,
) -> Result<u16, SkillEvalError> {
    let mut weighted = 0.0;
    for (group, cases) in outcomes {
        let expected = cases.values().try_fold(0_u32, |total, values| {
            let count = u32::try_from(values.len())
                .map_err(|_| invalid("frontier group trial count overflow"))?;
            total
                .checked_add(count)
                .ok_or_else(|| invalid("frontier group trial count overflow"))
        })?;
        if expected == 0 {
            return Err(invalid("frontier group has no trials"));
        }
        let passed = cases.values().try_fold(0_u32, |total, values| {
            let count = u32::try_from(values.iter().filter(|is_passing| **is_passing).count())
                .map_err(|_| invalid("frontier group pass count overflow"))?;
            total
                .checked_add(count)
                .ok_or_else(|| invalid("frontier group pass count overflow"))
        })?;
        let weight = suite
            .group_weights_basis_points
            .get(group)
            .ok_or_else(|| invalid("frontier group weight is missing"))?;
        weighted += f64::from(*weight) * f64::from(passed) / f64::from(expected);
    }
    if !weighted.is_finite() || !(0.0..=10_000.0).contains(&weighted) {
        return Err(invalid("frontier weighted score is invalid"));
    }
    u16::try_from(weighted.round() as u64).map_err(|_| invalid("frontier weighted score overflow"))
}

fn frontier_bootstrap_lower_bound(
    outcomes: &BTreeMap<FrontierCaseGroup, BTreeMap<(String, CaseId), Vec<bool>>>,
    suite: &FrontierTierSuite,
    model: &ModelIdentity,
    policy: &FrontierPolicy,
) -> Result<u16, SkillEvalError> {
    let resamples = usize::try_from(policy.confidence_resamples)
        .map_err(|_| invalid("frontier bootstrap resample count overflow"))?;
    let mut seed = 14_695_981_039_346_656_037_u64;
    for value in [
        model.provider.as_bytes(),
        model.model.as_bytes(),
        model.thinking.as_bytes(),
    ] {
        frontier_seed_bytes(&mut seed, value);
    }
    frontier_seed_bytes(&mut seed, &[model.tier as u8]);
    for (group, cases) in outcomes {
        frontier_seed_bytes(&mut seed, &[*group as u8]);
        for ((artifact, case), values) in cases {
            frontier_seed_bytes(&mut seed, artifact.as_bytes());
            frontier_seed_bytes(&mut seed, case.0.as_bytes());
            for is_passing in values {
                frontier_seed_bytes(&mut seed, &[u8::from(*is_passing)]);
            }
        }
    }
    let mut sampled_rates = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut weighted = 0.0;
        for (group, cases) in outcomes {
            if cases.is_empty() {
                return Err(invalid("frontier bootstrap group is empty"));
            }
            let case_values = cases.values().collect::<Vec<_>>();
            let mut passed = 0_u32;
            let mut expected = 0_u32;
            for _ in 0..case_values.len() {
                let index = usize::try_from(frontier_random(&mut seed))
                    .map_err(|_| invalid("frontier bootstrap index overflow"))?
                    % case_values.len();
                for is_passing in case_values[index] {
                    expected = expected
                        .checked_add(1)
                        .ok_or_else(|| invalid("frontier bootstrap trial count overflow"))?;
                    passed = passed
                        .checked_add(u32::from(*is_passing))
                        .ok_or_else(|| invalid("frontier bootstrap pass count overflow"))?;
                }
            }
            let weight = suite
                .group_weights_basis_points
                .get(group)
                .ok_or_else(|| invalid("frontier bootstrap group weight is missing"))?;
            weighted += f64::from(*weight) * f64::from(passed) / f64::from(expected);
        }
        if !weighted.is_finite() || !(0.0..=10_000.0).contains(&weighted) {
            return Err(invalid("frontier bootstrap produced an invalid score"));
        }
        sampled_rates.push(
            u16::try_from(weighted.round() as u64)
                .map_err(|_| invalid("frontier bootstrap score overflow"))?,
        );
    }
    sampled_rates.sort_unstable();
    let tail_basis_points = 10_000_u32
        .checked_sub(u32::from(policy.confidence_level_basis_points))
        .ok_or_else(|| invalid("frontier confidence level is invalid"))?;
    let index = u64::from(policy.confidence_resamples)
        .checked_mul(u64::from(tail_basis_points))
        .ok_or_else(|| invalid("frontier bootstrap percentile overflow"))?
        .checked_div(10_000)
        .ok_or_else(|| invalid("frontier bootstrap percentile is invalid"))?;
    let index = usize::try_from(index)
        .map_err(|_| invalid("frontier bootstrap percentile index overflow"))?
        .min(sampled_rates.len() - 1);
    sampled_rates
        .get(index)
        .copied()
        .ok_or_else(|| invalid("frontier bootstrap produced no samples"))
}

fn frontier_seed_bytes(seed: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *seed ^= u64::from(*byte);
        *seed = seed.wrapping_mul(1_099_511_628_211);
    }
}

fn frontier_random(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Selects the next tier and thinking level for one frontier model.
///
/// The inputs are a frozen entrant, current progress, and terminal cells. The output is new progress.
///
/// # Errors
///
/// Returns an error for identity drift, nonmonotonic progress, missing evidence, or invalid level order.
pub(crate) fn advance_frontier_model(
    entrant: &FrontierEntrant,
    progress: &FrontierModelProgress,
    cells: &[FrontierCellEvidence],
) -> Result<FrontierModelProgress, SkillEvalError> {
    validate_frontier_entrant(entrant)?;
    if progress.provider != entrant.provider
        || progress.model != entrant.model
        || progress.entry_tier != entrant.entry_tier
    {
        return Err(invalid("frontier progress belongs to a foreign entrant"));
    }
    let mut tier = entrant.entry_tier;
    let mut thinking_index = 0_usize;
    let mut selected_routes = Vec::new();
    let mut is_exhausted = false;
    let mut seen = BTreeSet::new();
    let mut reachable = vec![(Some(tier), Some(0_u8), selected_routes.clone(), false)];
    for (cell_index, cell) in cells.iter().enumerate() {
        if is_exhausted {
            return Err(invalid(
                "frontier evidence continues after terminal exhaustion",
            ));
        }
        if cell.model.provider != entrant.provider || cell.model.model != entrant.model {
            return Err(invalid("frontier evidence belongs to a foreign entrant"));
        }
        if !seen.insert(frontier_route_key(&cell.model)) {
            return Err(invalid("frontier evidence contains a duplicate route"));
        }
        let expected_thinking = entrant
            .thinking_levels
            .get(thinking_index)
            .ok_or_else(|| invalid("frontier thinking progression is exhausted"))?;
        if cell.model.tier != tier || &cell.model.thinking != expected_thinking {
            return Err(invalid(
                "frontier evidence skips or reorders the legal route",
            ));
        }
        validate_frontier_cell_shape(cell)?;
        match cell.status {
            FrontierCellStatus::Passed => {
                selected_routes.push(cell.model.clone());
                thinking_index = thinking_index
                    .checked_add(1)
                    .ok_or_else(|| invalid("frontier thinking index overflow"))?;
                match next_tier(tier) {
                    Some(next) if thinking_index < entrant.thinking_levels.len() => tier = next,
                    Some(_) | None => is_exhausted = true,
                }
            }
            FrontierCellStatus::Failed | FrontierCellStatus::Indeterminate => {
                thinking_index = thinking_index
                    .checked_add(1)
                    .ok_or_else(|| invalid("frontier thinking index overflow"))?;
                if thinking_index >= entrant.thinking_levels.len() {
                    is_exhausted = true;
                }
            }
            FrontierCellStatus::Pending => {
                if cell_index + 1 != cells.len() {
                    return Err(invalid("frontier evidence continues after a pending cell"));
                }
            }
            FrontierCellStatus::Running | FrontierCellStatus::Skipped => {
                return Err(invalid(
                    "frontier progression requires evaluated cell evidence",
                ));
            }
        }
        let reachable_route = if is_exhausted {
            (None, None)
        } else {
            (
                Some(tier),
                Some(
                    u8::try_from(thinking_index)
                        .map_err(|_| invalid("frontier thinking index overflow"))?,
                ),
            )
        };
        reachable.push((
            reachable_route.0,
            reachable_route.1,
            selected_routes.clone(),
            is_exhausted,
        ));
    }
    let is_progress_reachable = reachable.iter().any(
        |(next_tier, next_thinking_index, routes, is_reachable_exhausted)| {
            progress.next_tier == *next_tier
                && progress.next_thinking_index == *next_thinking_index
                && progress.selected_routes == *routes
                && progress.is_exhausted == *is_reachable_exhausted
        },
    );
    if !is_progress_reachable {
        return Err(invalid("frontier progress has an impossible next route"));
    }
    let (next_tier, next_thinking_index) = if is_exhausted {
        (None, None)
    } else {
        (
            Some(tier),
            Some(
                u8::try_from(thinking_index)
                    .map_err(|_| invalid("frontier thinking index overflow"))?,
            ),
        )
    };
    Ok(FrontierModelProgress {
        provider: entrant.provider.clone(),
        model: entrant.model.clone(),
        entry_tier: entrant.entry_tier,
        selected_routes,
        next_tier,
        next_thinking_index,
        is_exhausted,
    })
}

fn validate_frontier_entrant(entrant: &FrontierEntrant) -> Result<(), SkillEvalError> {
    const LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
    if entrant.thinking_levels.is_empty() {
        return Err(invalid("frontier entrant has no thinking levels"));
    }
    let mut previous = None;
    for level in &entrant.thinking_levels {
        let index = LEVELS
            .iter()
            .position(|supported| supported == level)
            .ok_or_else(|| invalid("frontier entrant has an unsupported thinking level"))?;
        if previous.is_some_and(|value| index <= value) {
            return Err(invalid(
                "frontier thinking levels are not strictly increasing",
            ));
        }
        previous = Some(index);
    }
    Ok(())
}

fn validate_frontier_cell_shape(cell: &FrontierCellEvidence) -> Result<(), SkillEvalError> {
    if cell.expected_trials == 0
        || cell.completed_trials != cell.expected_trials
        || cell.failed_trials > cell.completed_trials
        || cell.score.is_none()
    {
        return Err(invalid("frontier cell has incomplete evidence"));
    }
    let score = cell
        .score
        .as_ref()
        .ok_or_else(|| invalid("frontier cell score is missing"))?;
    if score.weighted_pass_basis_points > 10_000
        || score.lower_bound_basis_points > 10_000
        || score.critical_expected_trials == 0
        || score.critical_passed_trials > score.critical_expected_trials
        || !score.is_group_coverage_complete
    {
        return Err(invalid("frontier cell score is invalid"));
    }
    Ok(())
}

fn frontier_route_key(model: &ModelIdentity) -> (Tier, String, String, String) {
    (
        model.tier,
        model.provider.clone(),
        model.model.clone(),
        model.thinking.clone(),
    )
}

fn next_tier(tier: Tier) -> Option<Tier> {
    match tier {
        Tier::T1 => Some(Tier::T2),
        Tier::T2 => Some(Tier::T3),
        Tier::T3 => Some(Tier::T4),
        Tier::T4 => Some(Tier::T5),
        Tier::T5 => None,
    }
}

/// Ranks qualified routes and marks each tier's active routes.
///
/// The inputs are terminal model reports and active-pool size. The output is ranked memberships by tier.
///
/// # Errors
///
/// Returns an error for incomplete evidence, duplicate routes, invalid usage, or a zero pool size.
pub(crate) fn rank_frontier_pools(
    models: &[FrontierModelReport],
    active_pool_size: u8,
) -> Result<BTreeMap<Tier, Vec<FrontierPoolMembership>>, SkillEvalError> {
    if active_pool_size == 0 {
        return Err(invalid("frontier active pool size must be positive"));
    }
    let mut routes = BTreeMap::<Tier, Vec<&FrontierCellEvidence>>::new();
    let mut seen_models = BTreeSet::new();
    let mut seen_routes = BTreeSet::new();
    for report in models {
        if !seen_models.insert((report.provider.as_str(), report.model.as_str())) {
            return Err(invalid("frontier reports contain a duplicate model"));
        }
        let mut expected_usage = empty_usage();
        let mut passed_routes = BTreeSet::new();
        let mut highest_passing_tier = None;
        for cell in &report.cells {
            if cell.model.provider != report.provider || cell.model.model != report.model {
                return Err(invalid("frontier report contains a foreign cell"));
            }
            if !seen_routes.insert(frontier_route_key(&cell.model)) {
                return Err(invalid("frontier reports contain a duplicate route"));
            }
            if !matches!(
                cell.status,
                FrontierCellStatus::Passed
                    | FrontierCellStatus::Failed
                    | FrontierCellStatus::Indeterminate
                    | FrontierCellStatus::Skipped
            ) {
                return Err(invalid("frontier report contains nonterminal evidence"));
            }
            if cell.status == FrontierCellStatus::Skipped {
                if cell.completed_trials != 0
                    || cell.expected_trials != 0
                    || cell.failed_trials != 0
                    || cell.score.is_some()
                    || cell.total_usage != empty_usage()
                {
                    return Err(invalid("frontier skipped cell contains trial evidence"));
                }
            } else {
                validate_frontier_cell_shape(cell)?;
            }
            add_usage(&mut expected_usage, &cell.total_usage)?;
            if cell.status == FrontierCellStatus::Passed {
                if cell.total_usage.cost_millionths_of_dollar == 0 {
                    return Err(invalid("frontier passed route has no measured cost"));
                }
                passed_routes.insert(frontier_route_key(&cell.model));
                highest_passing_tier = Some(
                    highest_passing_tier
                        .map_or(cell.model.tier, |tier: Tier| tier.max(cell.model.tier)),
                );
                routes.entry(cell.model.tier).or_default().push(cell);
            }
        }
        let selected = report
            .selected_routes
            .iter()
            .map(frontier_route_key)
            .collect::<BTreeSet<_>>();
        if selected.len() != report.selected_routes.len()
            || selected != passed_routes
            || report.highest_passing_tier != highest_passing_tier
        {
            return Err(invalid("frontier report has inconsistent selected routes"));
        }
        if expected_usage != report.total_usage {
            return Err(invalid("frontier report has incomplete aggregate usage"));
        }
    }

    let mut pools = BTreeMap::new();
    for (tier, mut cells) in routes {
        cells.sort_by(|left, right| {
            let left_weighted = left
                .score
                .as_ref()
                .map(|score| score.weighted_pass_basis_points)
                .unwrap_or(0);
            let right_weighted = right
                .score
                .as_ref()
                .map(|score| score.weighted_pass_basis_points)
                .unwrap_or(0);
            let left_lower = left
                .score
                .as_ref()
                .map(|score| score.lower_bound_basis_points)
                .unwrap_or(0);
            let right_lower = right
                .score
                .as_ref()
                .map(|score| score.lower_bound_basis_points)
                .unwrap_or(0);
            right_weighted
                .cmp(&left_weighted)
                .then_with(|| right_lower.cmp(&left_lower))
                .then_with(|| {
                    compare_rate(
                        left.total_usage.cost_millionths_of_dollar,
                        left.completed_trials,
                        right.total_usage.cost_millionths_of_dollar,
                        right.completed_trials,
                    )
                })
                .then_with(|| compare_model_identity(&left.model, &right.model))
        });
        let memberships = cells
            .into_iter()
            .enumerate()
            .map(|(index, cell)| {
                let rank =
                    u16::try_from(index + 1).map_err(|_| invalid("frontier pool rank overflow"))?;
                Ok(FrontierPoolMembership {
                    model: cell.model.clone(),
                    rank,
                    is_active: index < usize::from(active_pool_size),
                })
            })
            .collect::<Result<Vec<_>, SkillEvalError>>()?;
        pools.insert(tier, memberships);
    }
    Ok(pools)
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
        assert!(evaluate_tier(
            EvidenceRole::Reference,
            &trials(Tier::T4),
            Some(&reference()),
            &policy(),
        )
        .is_err());

        let mut candidate = reference();
        candidate.role = EvidenceRole::Candidate;
        assert!(evaluate_tier(
            EvidenceRole::Candidate,
            &candidate_trials,
            Some(&candidate),
            &policy(),
        )
        .is_err());
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
        assert!(evaluate_tier(
            EvidenceRole::Candidate,
            &incomplete,
            Some(&reference()),
            &policy(),
        )
        .is_err());

        for change in [
            |trial: &mut TrialRecord| trial.model.model = "other".to_string(),
            |trial: &mut TrialRecord| trial.harness.runner_version = "other".to_string(),
            |trial: &mut TrialRecord| trial.harness.pi_version = "other".to_string(),
            |trial: &mut TrialRecord| trial.harness.artifact_revision = "other".to_string(),
        ] {
            let mut mixed = vec![trial(Tier::T2, "a", 1, 8), trial(Tier::T2, "a", 2, 8)];
            change(&mut mixed[1]);
            assert!(evaluate_tier(
                EvidenceRole::Candidate,
                &mixed,
                Some(&reference()),
                &policy(),
            )
            .is_err());
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
        assert!(evaluate_tier(
            EvidenceRole::Candidate,
            &trials,
            Some(&reference()),
            &policy(),
        )
        .is_err());
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
                route_index: 0,
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
