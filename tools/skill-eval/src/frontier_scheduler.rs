use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    ArtifactName, FrontierCellEvidence, FrontierCellStatus, FrontierEntrant, FrontierPlan,
    FrontierRunState, FrontierRunStatus, FrontierScheduleAction, FrontierSuite, ModelIdentity,
    PoolPauseReason, SkillEvalError, TrialKey, TrialRecord,
};
use crate::statistics::{advance_frontier_model, evaluate_frontier_cell};

pub(crate) fn next_frontier_trial(
    plan: &FrontierPlan,
    suite: &FrontierSuite,
    state: &FrontierRunState,
    trials: &[TrialRecord],
) -> Result<FrontierScheduleAction, SkillEvalError> {
    validate_inputs(plan, suite, state, trials)?;
    if let Some(reason) = &state.pause {
        return Ok(FrontierScheduleAction::Pause {
            reason: reason.clone(),
        });
    }
    match state.status {
        FrontierRunStatus::Pending | FrontierRunStatus::Running => {}
        FrontierRunStatus::AwaitingDecision
        | FrontierRunStatus::Accepted
        | FrontierRunStatus::Rejected
        | FrontierRunStatus::Failed => {
            return Ok(FrontierScheduleAction::Terminal {
                status: state.status,
            });
        }
        FrontierRunStatus::Paused => {
            return Err(invalid("paused frontier run has no pause reason"));
        }
    }

    let mut entrants = plan.entrants.iter().collect::<Vec<_>>();
    entrants.sort_by_key(|entrant| entrant_key(entrant));
    let route_indices = entrants
        .iter()
        .enumerate()
        .map(|(index, entrant)| {
            u16::try_from(index)
                .map(|route_index| (base_key(entrant), route_index))
                .map_err(|_| invalid("frontier entrant index overflow"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut consumed = BTreeSet::new();

    for entrant in entrants {
        let progress = progress_for(state, entrant)?;
        let route_index = route_indices[&base_key(entrant)];
        let mut cells = Vec::new();

        loop {
            let derived = advance_frontier_model(entrant, progress, &cells)?;
            if derived.is_exhausted {
                break;
            }
            let tier = derived
                .next_tier
                .ok_or_else(|| invalid("frontier progress has no next tier"))?;
            let thinking_index = usize::from(
                derived
                    .next_thinking_index
                    .ok_or_else(|| invalid("frontier progress has no next thinking level"))?,
            );
            let thinking = entrant
                .thinking_levels
                .get(thinking_index)
                .ok_or_else(|| invalid("frontier thinking index overflow"))?;
            let model = ModelIdentity {
                provider: entrant.provider.clone(),
                model: entrant.model.clone(),
                tier,
                thinking: thinking.clone(),
            };
            let tier_suite = suite
                .tiers
                .get(&tier)
                .ok_or_else(|| invalid("frontier suite is missing a scheduled tier"))?;
            let mut route_trials = trials
                .iter()
                .filter(|trial| trial.model == model)
                .cloned()
                .collect::<Vec<_>>();
            route_trials.sort_by(|left, right| left.key.cmp(&right.key));
            if route_trials
                .iter()
                .any(|trial| trial.key.route_index != route_index)
            {
                return Err(invalid("frontier trial has the wrong route index"));
            }
            let checkpoint = completed_checkpoint(tier_suite.cases.len(), &route_trials)?;

            if let Some(attempts) = checkpoint {
                let evidence =
                    evaluate_frontier_cell(tier_suite, &model, &route_trials, &plan.policy)?;
                consumed.extend(route_trials.iter().map(|trial| trial.key.clone()));
                let is_screen_promising = attempts
                    == usize::from(plan.policy.screening_trials_per_case)
                    && evidence.status != FrontierCellStatus::Failed;
                let is_confirmation_uncertain = attempts
                    == usize::from(plan.policy.confirmation_trials_per_case)
                    && evidence.status == FrontierCellStatus::Pending;
                if is_screen_promising || is_confirmation_uncertain {
                    let target = if is_screen_promising {
                        plan.policy.confirmation_trials_per_case
                    } else {
                        plan.policy.maximum_trials_per_case
                    };
                    if let Some(key) =
                        first_missing_key(tier_suite, tier, route_index, target, &route_trials)?
                    {
                        return schedule_or_stop(state, plan, trials, consumed, model, key);
                    }
                    return Err(invalid("frontier expandable cell has no missing trial"));
                }
                validate_persisted_cell(state, &evidence)?;
                cells.push(evidence);
                continue;
            }

            let target = active_target(tier_suite.cases.len(), &route_trials, &plan.policy)?;
            let key = first_missing_key(tier_suite, tier, route_index, target, &route_trials)?
                .ok_or_else(|| invalid("frontier incomplete cell has no missing trial"))?;
            consumed.extend(route_trials.iter().map(|trial| trial.key.clone()));
            return schedule_or_stop(state, plan, trials, consumed, model, key);
        }
    }

    ensure_no_unconsumed_trials(trials, &consumed)?;
    Ok(FrontierScheduleAction::Complete)
}

fn schedule_or_stop(
    state: &FrontierRunState,
    plan: &FrontierPlan,
    trials: &[TrialRecord],
    consumed: BTreeSet<TrialKey>,
    model: ModelIdentity,
    key: TrialKey,
) -> Result<FrontierScheduleAction, SkillEvalError> {
    ensure_no_unconsumed_trials(trials, &consumed)?;
    let mut events = state
        .infrastructure_events
        .iter()
        .filter(|event| {
            event.model == model
                && event.artifact == key.artifact
                && event.case == key.case
                && event.attempt == key.attempt
        })
        .collect::<Vec<_>>();
    events.sort_by_key(|event| event.infrastructure_attempt);
    if events
        .windows(2)
        .any(|window| window[0].infrastructure_attempt == window[1].infrastructure_attempt)
    {
        return Err(invalid("frontier infrastructure attempt is duplicated"));
    }
    let attempts = events
        .iter()
        .map(|event| event.infrastructure_attempt)
        .collect::<Vec<_>>();
    let expected = (1..=u8::try_from(attempts.len())
        .map_err(|_| invalid("frontier infrastructure attempt count overflow"))?)
        .collect::<Vec<_>>();
    if attempts != expected {
        return Err(invalid(
            "frontier infrastructure attempts are not contiguous",
        ));
    }
    if events.len() >= usize::from(plan.policy.maximum_infrastructure_attempts) {
        let message = events
            .last()
            .ok_or_else(|| invalid("frontier infrastructure pause has no event"))?
            .message
            .clone();
        return Ok(FrontierScheduleAction::Pause {
            reason: PoolPauseReason::Infrastructure { message },
        });
    }
    let reserved_cost_millionths_of_dollar = plan.policy.maximum_trial_cost_millionths_of_dollar;
    if reserved_cost_millionths_of_dollar == 0 {
        return Err(invalid("frontier trial cost reservation is zero"));
    }
    let projected_spend = state
        .spent_millionths_of_dollar
        .checked_add(reserved_cost_millionths_of_dollar)
        .ok_or_else(|| invalid("frontier projected spend overflow"))?;
    if projected_spend > plan.policy.spending_limit_millionths_of_dollar {
        return Ok(FrontierScheduleAction::Pause {
            reason: PoolPauseReason::SpendingLimit {
                spent_millionths_of_dollar: state.spent_millionths_of_dollar,
                limit_millionths_of_dollar: plan.policy.spending_limit_millionths_of_dollar,
            },
        });
    }
    let infrastructure_attempt = u8::try_from(events.len())
        .map_err(|_| invalid("frontier infrastructure attempt count overflow"))?
        .checked_add(1)
        .ok_or_else(|| invalid("frontier infrastructure attempt overflow"))?;
    Ok(FrontierScheduleAction::Dispatch {
        model,
        key,
        infrastructure_attempt,
        reserved_cost_millionths_of_dollar,
    })
}

fn validate_inputs(
    plan: &FrontierPlan,
    suite: &FrontierSuite,
    state: &FrontierRunState,
    trials: &[TrialRecord],
) -> Result<(), SkillEvalError> {
    if state.configuration.plan != *plan
        || plan.suite.version != suite.version
        || plan.policy.maximum_infrastructure_attempts != 2
    {
        return Err(invalid(
            "frontier scheduler input drifted from the frozen plan",
        ));
    }
    let mut entrants = BTreeSet::new();
    for entrant in &plan.entrants {
        if !entrants.insert(base_key(entrant)) {
            return Err(invalid("frontier plan contains a duplicate entrant"));
        }
    }
    if state.models.len() != plan.entrants.len() {
        return Err(invalid("frontier state has incomplete model progress"));
    }
    let mut keys = BTreeSet::new();
    for trial in trials {
        if !keys.insert(trial.key.clone()) {
            return Err(invalid("frontier trials contain a duplicate key"));
        }
        if trial.key.tier != trial.model.tier {
            return Err(invalid("frontier trial tier differs from its model"));
        }
        if !entrants.contains(&(trial.model.provider.clone(), trial.model.model.clone())) {
            return Err(invalid("frontier trial belongs to a foreign entrant"));
        }
    }
    Ok(())
}

fn progress_for<'a>(
    state: &'a FrontierRunState,
    entrant: &FrontierEntrant,
) -> Result<&'a crate::model::FrontierModelProgress, SkillEvalError> {
    let matching = state
        .models
        .iter()
        .filter(|progress| progress.provider == entrant.provider && progress.model == entrant.model)
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err(invalid(
            "frontier entrant has missing or duplicate progress",
        ));
    }
    Ok(matching[0])
}

fn completed_checkpoint(
    case_count: usize,
    trials: &[TrialRecord],
) -> Result<Option<usize>, SkillEvalError> {
    if trials.is_empty() {
        return Ok(None);
    }
    if case_count == 0 || !trials.len().is_multiple_of(case_count) {
        return Ok(None);
    }
    let attempts = trials.len() / case_count;
    if [1, 3, 5].contains(&attempts) {
        Ok(Some(attempts))
    } else {
        Ok(None)
    }
}

fn active_target(
    case_count: usize,
    trials: &[TrialRecord],
    policy: &crate::model::FrontierPolicy,
) -> Result<u8, SkillEvalError> {
    if case_count == 0 {
        return Err(invalid("frontier suite tier has no cases"));
    }
    let completed = trials.len();
    let screening = case_count
        .checked_mul(usize::from(policy.screening_trials_per_case))
        .ok_or_else(|| invalid("frontier screening trial count overflow"))?;
    let confirmation = case_count
        .checked_mul(usize::from(policy.confirmation_trials_per_case))
        .ok_or_else(|| invalid("frontier confirmation trial count overflow"))?;
    if completed < screening {
        Ok(policy.screening_trials_per_case)
    } else if completed < confirmation {
        Ok(policy.confirmation_trials_per_case)
    } else {
        Ok(policy.maximum_trials_per_case)
    }
}

fn first_missing_key(
    suite: &crate::model::FrontierTierSuite,
    tier: crate::model::Tier,
    route_index: u16,
    target: u8,
    trials: &[TrialRecord],
) -> Result<Option<TrialKey>, SkillEvalError> {
    let present = trials
        .iter()
        .map(|trial| trial.key.clone())
        .collect::<BTreeSet<_>>();
    let mut cases = suite
        .cases
        .iter()
        .map(|case| {
            let artifact = case
                .artifact_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| invalid("frontier artifact path has no valid name"))?;
            Ok((ArtifactName(artifact.to_string()), case.case.clone()))
        })
        .collect::<Result<Vec<_>, SkillEvalError>>()?;
    cases.sort();
    if cases.windows(2).any(|window| window[0] == window[1]) {
        return Err(invalid("frontier suite contains a duplicate case"));
    }
    for attempt in 1..=u16::from(target) {
        for (artifact, case) in &cases {
            let key = TrialKey {
                artifact: artifact.clone(),
                tier,
                route_index,
                case: case.clone(),
                attempt,
            };
            if !present.contains(&key) {
                return Ok(Some(key));
            }
        }
    }
    Ok(None)
}

fn validate_persisted_cell(
    state: &FrontierRunState,
    derived: &FrontierCellEvidence,
) -> Result<(), SkillEvalError> {
    let matching = state
        .cells
        .iter()
        .filter(|cell| cell.model == derived.model)
        .collect::<Vec<_>>();
    if matching.len() > 1 {
        return Err(invalid("frontier state contains a duplicate cell"));
    }
    if matching.first().is_some_and(|cell| *cell != derived) {
        return Err(invalid(
            "frontier cell differs from terminal trial evidence",
        ));
    }
    Ok(())
}

fn ensure_no_unconsumed_trials(
    trials: &[TrialRecord],
    consumed: &BTreeSet<TrialKey>,
) -> Result<(), SkillEvalError> {
    if trials.iter().any(|trial| !consumed.contains(&trial.key)) {
        return Err(invalid("frontier trial evidence skips the legal frontier"));
    }
    Ok(())
}

fn entrant_key(entrant: &FrontierEntrant) -> (crate::model::Tier, String, String) {
    (
        entrant.entry_tier,
        entrant.provider.clone(),
        entrant.model.clone(),
    )
}

fn base_key(entrant: &FrontierEntrant) -> (String, String) {
    (entrant.provider.clone(), entrant.model.clone())
}

fn invalid(message: &str) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(message.to_string())
}
