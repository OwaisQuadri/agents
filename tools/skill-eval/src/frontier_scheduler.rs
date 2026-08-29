use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::model::{
    ArtifactName, FRONTIER_WORKER_LIMIT, FrontierCellEvidence, FrontierCellStatus, FrontierEntrant,
    FrontierPlan, FrontierRunState, FrontierRunStatus, FrontierScheduleAction,
    FrontierScheduledTrial, FrontierSuite, ModelIdentity, PoolPauseReason, SkillEvalError,
    TrialKey, TrialRecord,
};
use crate::statistics::{advance_frontier_model, evaluate_frontier_cell};

pub(crate) fn next_frontier_wave(
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
    let skipped_routes = state
        .cells
        .iter()
        .filter(|cell| cell.status == FrontierCellStatus::Skipped)
        .map(|cell| cell.model.clone())
        .collect::<BTreeSet<_>>();
    let mut consumed = trials
        .iter()
        .filter(|trial| skipped_routes.contains(&trial.model))
        .map(|trial| (trial.model.clone(), trial.key.clone()))
        .collect::<BTreeSet<_>>();
    let mut scheduled = Vec::new();

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
            if let Some(skipped) = state
                .cells
                .iter()
                .find(|cell| cell.model == model && cell.status == FrontierCellStatus::Skipped)
            {
                cells.push(skipped.clone());
                continue;
            }
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
            consumed.extend(
                route_trials
                    .iter()
                    .map(|trial| (trial.model.clone(), trial.key.clone())),
            );
            let checkpoint = completed_checkpoint(tier_suite.cases.len(), &route_trials)?;
            let target = if let Some(attempts) = checkpoint {
                let evidence =
                    evaluate_frontier_cell(tier_suite, &model, &route_trials, &plan.policy)?;
                let is_screen_promising = attempts
                    == usize::from(plan.policy.screening_trials_per_case)
                    && evidence.status != FrontierCellStatus::Failed;
                let is_confirmation_uncertain = attempts
                    == usize::from(plan.policy.confirmation_trials_per_case)
                    && evidence.status == FrontierCellStatus::Pending;
                if is_screen_promising {
                    plan.policy.confirmation_trials_per_case
                } else if is_confirmation_uncertain {
                    plan.policy.maximum_trials_per_case
                } else {
                    validate_persisted_cell(state, &evidence)?;
                    cells.push(evidence);
                    continue;
                }
            } else {
                active_target(tier_suite.cases.len(), &route_trials, &plan.policy)?
            };
            let keys = earliest_missing_keys(tier_suite, tier, route_index, target, &route_trials)?;
            if keys.is_empty() {
                return Err(invalid("frontier incomplete cell has no missing trial"));
            }
            for key in keys {
                match scheduled_trial(state, plan, &model, key)? {
                    Ok(trial) => scheduled.push(trial),
                    Err(reason) => return Ok(FrontierScheduleAction::Pause { reason }),
                }
            }
            break;
        }
    }

    ensure_no_unconsumed_trials(trials, &consumed)?;
    if scheduled.is_empty() {
        return Ok(FrontierScheduleAction::Complete);
    }
    scheduled.sort();
    if scheduled.windows(2).any(|window| window[0] == window[1]) {
        return Err(invalid("frontier wave contains a duplicate trial"));
    }
    scheduled = bounded_frontier_wave(scheduled, state, trials);
    let reserved_cost_per_trial_millionths_of_dollar =
        plan.policy.maximum_trial_cost_millionths_of_dollar;
    if reserved_cost_per_trial_millionths_of_dollar == 0 {
        return Err(invalid("frontier trial cost reservation is zero"));
    }
    let trial_count = u64::try_from(scheduled.len())
        .map_err(|_| invalid("frontier wave trial count overflow"))?;
    let wave_reservation = trial_count
        .checked_mul(reserved_cost_per_trial_millionths_of_dollar)
        .ok_or_else(|| invalid("frontier wave reservation overflow"))?;
    let projected_spend = state
        .spent_millionths_of_dollar
        .checked_add(wave_reservation)
        .ok_or_else(|| invalid("frontier projected spend overflow"))?;
    if projected_spend > plan.policy.spending_limit_millionths_of_dollar {
        return Ok(FrontierScheduleAction::Pause {
            reason: PoolPauseReason::SpendingLimit {
                spent_millionths_of_dollar: state.spent_millionths_of_dollar,
                limit_millionths_of_dollar: plan.policy.spending_limit_millionths_of_dollar,
            },
        });
    }
    Ok(FrontierScheduleAction::Dispatch {
        trials: scheduled,
        reserved_cost_per_trial_millionths_of_dollar,
    })
}

fn bounded_frontier_wave(
    scheduled: Vec<FrontierScheduledTrial>,
    state: &FrontierRunState,
    trials: &[TrialRecord],
) -> Vec<FrontierScheduledTrial> {
    let mut queues = BTreeMap::<ModelIdentity, VecDeque<FrontierScheduledTrial>>::new();
    for trial in scheduled {
        queues
            .entry(trial.model.clone())
            .or_default()
            .push_back(trial);
    }
    let mut rows = queues
        .into_iter()
        .map(|(model, queue)| {
            let trial_count = trials
                .iter()
                .filter(|trial| same_frontier_entrant(&trial.model, &model))
                .count();
            let event_count = state
                .infrastructure_events
                .iter()
                .filter(|event| same_frontier_entrant(&event.model, &model))
                .count();
            (trial_count.saturating_add(event_count), model, queue)
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));

    let mut wave = Vec::new();
    while wave.len() < FRONTIER_WORKER_LIMIT {
        let mut added = false;
        for (_, _, queue) in &mut rows {
            if let Some(trial) = queue.pop_front() {
                wave.push(trial);
                added = true;
            }
            if wave.len() == FRONTIER_WORKER_LIMIT {
                break;
            }
        }
        if !added {
            break;
        }
    }
    wave.sort();
    wave
}

fn same_frontier_entrant(left: &ModelIdentity, right: &ModelIdentity) -> bool {
    left.provider == right.provider && left.model == right.model
}

fn scheduled_trial(
    state: &FrontierRunState,
    plan: &FrontierPlan,
    model: &ModelIdentity,
    key: TrialKey,
) -> Result<Result<FrontierScheduledTrial, PoolPauseReason>, SkillEvalError> {
    let mut events = state
        .infrastructure_events
        .iter()
        .filter(|event| {
            event.model == *model
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
        return Ok(Err(PoolPauseReason::Infrastructure { message }));
    }
    let infrastructure_attempt = u8::try_from(events.len())
        .map_err(|_| invalid("frontier infrastructure attempt count overflow"))?
        .checked_add(1)
        .ok_or_else(|| invalid("frontier infrastructure attempt overflow"))?;
    Ok(Ok(FrontierScheduledTrial {
        model: model.clone(),
        key,
        infrastructure_attempt,
    }))
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
    let mut identities = BTreeSet::new();
    for trial in trials {
        if !identities.insert((trial.model.clone(), trial.key.clone())) {
            return Err(invalid("frontier trials contain a duplicate identity"));
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

fn earliest_missing_keys(
    suite: &crate::model::FrontierTierSuite,
    tier: crate::model::Tier,
    route_index: u16,
    target: u8,
    trials: &[TrialRecord],
) -> Result<Vec<TrialKey>, SkillEvalError> {
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
        let missing = cases
            .iter()
            .filter_map(|(artifact, case)| {
                let key = TrialKey {
                    artifact: artifact.clone(),
                    tier,
                    route_index,
                    case: case.clone(),
                    attempt,
                };
                (!present.contains(&key)).then_some(key)
            })
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            if trials.iter().any(|trial| trial.key.attempt > attempt) {
                return Err(invalid(
                    "frontier trial evidence crosses an incomplete attempt barrier",
                ));
            }
            return Ok(missing);
        }
    }
    Ok(Vec::new())
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
    consumed: &BTreeSet<(ModelIdentity, TrialKey)>,
) -> Result<(), SkillEvalError> {
    if trials
        .iter()
        .any(|trial| !consumed.contains(&(trial.model.clone(), trial.key.clone())))
    {
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
