use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    Decision, FrontierBaseline, FrontierBaselineChange, FrontierCellEvidence, FrontierCellStatus,
    FrontierModelProgress, FrontierModelReport, FrontierReport, FrontierRunState,
    FrontierRunStatus, ModelIdentity, SkillEvalError, Tier, TrialUsage,
};

pub(crate) fn active_frontier_routes(
    state: &FrontierRunState,
    baseline: &FrontierBaseline,
) -> Result<BTreeMap<Tier, Vec<ModelIdentity>>, SkillEvalError> {
    let decision = state
        .decision
        .as_ref()
        .filter(|decision| {
            decision.decision == Decision::Accepted
                && !decision.reason.trim().is_empty()
                && !decision.decided_at.0.trim().is_empty()
        })
        .ok_or_else(|| invalid("active routes require an accepted decision"))?;
    let expected_path = std::path::Path::new(".map/skill-eval/frontier")
        .join(&state.configuration.run_id.0)
        .join("state.json");
    if state.status != FrontierRunStatus::Accepted
        || baseline.run_id != state.configuration.run_id
        || baseline.accepted_at != decision.decided_at
        || baseline.run_evidence.path != expected_path
    {
        return Err(invalid("accepted baseline authority differs from its run"));
    }

    let tiers = [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5];
    if baseline.pools.keys().copied().collect::<Vec<_>>() != tiers {
        return Err(invalid(
            "accepted baseline does not contain every tier pool",
        ));
    }
    let expected = state
        .models
        .iter()
        .flat_map(|model| &model.selected_routes)
        .map(route_identity)
        .collect::<BTreeSet<_>>();
    let mut qualified = BTreeSet::new();
    let mut active = BTreeMap::new();
    for tier in tiers {
        let memberships = &baseline.pools[&tier];
        let active_count = memberships.iter().filter(|item| item.is_active).count();
        if memberships.is_empty()
            || active_count
                != usize::from(state.configuration.plan.policy.active_pool_size)
                    .min(memberships.len())
        {
            return Err(invalid("accepted baseline active pool size is invalid"));
        }
        let mut routes = Vec::with_capacity(active_count);
        for (index, membership) in memberships.iter().enumerate() {
            let rank = u16::try_from(index + 1)
                .map_err(|_| invalid("accepted baseline rank exceeds the supported range"))?;
            let route = &membership.model;
            if membership.rank != rank
                || membership.is_active != (index < active_count)
                || route.tier != tier
                || !matches!(route.provider.as_str(), "anthropic" | "openai-codex")
                || route.model.trim().is_empty()
                || route.thinking.trim().is_empty()
                || !expected.contains(&route_identity(route))
                || !qualified.insert(route_identity(route))
            {
                return Err(invalid("accepted baseline route is invalid or foreign"));
            }
            if membership.is_active {
                routes.push(route.clone());
            }
        }
        active.insert(tier, routes);
    }
    if qualified != expected {
        return Err(invalid("accepted baseline omits qualified run evidence"));
    }
    Ok(active)
}

fn route_identity(route: &ModelIdentity) -> (String, String, Tier, String) {
    (
        route.provider.clone(),
        route.model.clone(),
        route.tier,
        route.thinking.clone(),
    )
}

pub(crate) fn derive_frontier_report(
    state: &FrontierRunState,
    progress: &[FrontierModelProgress],
    evidence: &[FrontierCellEvidence],
    baseline: Option<&FrontierBaseline>,
) -> Result<FrontierReport, SkillEvalError> {
    if progress.len() != state.configuration.plan.entrants.len() {
        return Err(invalid("canonical model progress is incomplete"));
    }
    validate_infrastructure_events(state)?;
    let mut reports = Vec::with_capacity(state.configuration.plan.entrants.len());
    let mut seen_evidence = BTreeSet::new();
    for (entrant, model_progress) in state.configuration.plan.entrants.iter().zip(progress) {
        if model_progress.provider != entrant.provider
            || model_progress.model != entrant.model
            || model_progress.entry_tier != entrant.entry_tier
        {
            return Err(invalid("canonical model progress identity drifted"));
        }
        let model_evidence = evidence
            .iter()
            .filter(|cell| {
                cell.model.provider == entrant.provider && cell.model.model == entrant.model
            })
            .cloned()
            .collect::<Vec<_>>();
        for cell in &model_evidence {
            if !seen_evidence.insert((
                cell.model.provider.clone(),
                cell.model.model.clone(),
                cell.model.tier,
                cell.model.thinking.clone(),
            )) {
                return Err(invalid("canonical frontier evidence is duplicated"));
            }
        }
        let cells = matrix_cells(entrant, model_progress, &model_evidence)?;
        let total_usage = cells.iter().try_fold(empty_usage(), |mut total, cell| {
            add_usage(&mut total, &cell.total_usage)?;
            Ok(total)
        })?;
        let highest_passing_tier = cells
            .iter()
            .filter(|cell| cell.status == FrontierCellStatus::Passed)
            .map(|cell| cell.model.tier)
            .max();
        let selected_routes = cells
            .iter()
            .filter(|cell| cell.status == FrontierCellStatus::Passed)
            .map(|cell| cell.model.clone())
            .collect::<Vec<_>>();
        if selected_routes != model_progress.selected_routes {
            return Err(invalid("canonical selected routes are inconsistent"));
        }
        reports.push(FrontierModelReport {
            provider: entrant.provider.clone(),
            model: entrant.model.clone(),
            supported_thinking_levels: entrant.thinking_levels.clone(),
            cells,
            highest_passing_tier,
            selected_routes,
            pool_memberships: BTreeMap::new(),
            baseline_change: FrontierBaselineChange::NotCompared,
            total_usage,
        });
    }
    if seen_evidence.len() != evidence.len() {
        return Err(invalid(
            "canonical frontier evidence belongs to a foreign entrant",
        ));
    }

    let ranking_input = reports
        .iter()
        .cloned()
        .map(|mut report| {
            report
                .cells
                .retain(|cell| cell.status != FrontierCellStatus::Pending);
            report.total_usage =
                report
                    .cells
                    .iter()
                    .try_fold(empty_usage(), |mut total, cell| {
                        add_usage(&mut total, &cell.total_usage)?;
                        Ok(total)
                    })?;
            Ok(report)
        })
        .collect::<Result<Vec<_>, SkillEvalError>>()?;
    let pools = crate::statistics::rank_frontier_pools(
        &ranking_input,
        state.configuration.plan.policy.active_pool_size,
    )?;
    for report in &mut reports {
        for (tier, memberships) in &pools {
            if let Some(membership) = memberships.iter().find(|membership| {
                membership.model.provider == report.provider
                    && membership.model.model == report.model
            }) {
                report.pool_memberships.insert(*tier, membership.clone());
            }
        }
        report.baseline_change = baseline_change(report, baseline)?;
    }

    Ok(FrontierReport {
        run_id: state.configuration.run_id.clone(),
        status: state.status,
        models: reports,
        pause: state.pause.clone(),
        decision: state.decision.clone(),
        spent_millionths_of_dollar: state.spent_millionths_of_dollar,
    })
}

fn matrix_cells(
    entrant: &crate::model::FrontierEntrant,
    progress: &FrontierModelProgress,
    evidence: &[FrontierCellEvidence],
) -> Result<Vec<FrontierCellEvidence>, SkillEvalError> {
    if evidence.iter().any(|cell| {
        !entrant.thinking_levels.contains(&cell.model.thinking)
            || !matches!(
                cell.status,
                FrontierCellStatus::Passed
                    | FrontierCellStatus::Failed
                    | FrontierCellStatus::Indeterminate
            )
    }) {
        return Err(invalid(
            "frontier matrix evidence is nonterminal or foreign",
        ));
    }
    let initial = FrontierModelProgress {
        provider: entrant.provider.clone(),
        model: entrant.model.clone(),
        entry_tier: entrant.entry_tier,
        selected_routes: Vec::new(),
        next_tier: Some(entrant.entry_tier),
        next_thinking_index: Some(0),
        is_exhausted: false,
    };
    let derived = crate::statistics::advance_frontier_model(entrant, &initial, evidence)?;
    if &derived != progress {
        return Err(invalid(
            "frontier matrix progress differs from its evidence",
        ));
    }
    Ok(evidence.to_vec())
}

fn baseline_change(
    report: &FrontierModelReport,
    baseline: Option<&FrontierBaseline>,
) -> Result<FrontierBaselineChange, SkillEvalError> {
    let Some(baseline) = baseline else {
        return Ok(FrontierBaselineChange::NotCompared);
    };
    validate_baseline_pools(baseline)?;
    let mut incumbent = baseline
        .pools
        .iter()
        .flat_map(|(tier, memberships)| {
            memberships
                .iter()
                .filter(move |membership| {
                    membership.model.provider == report.provider
                        && membership.model.model == report.model
                })
                .map(move |membership| (*tier, membership.model.clone()))
        })
        .collect::<Vec<_>>();
    if incumbent.is_empty() {
        return Ok(FrontierBaselineChange::New);
    }
    incumbent.sort_by(compare_routes);
    let mut current = report
        .selected_routes
        .iter()
        .map(|model| (model.tier, model.clone()))
        .collect::<Vec<_>>();
    current.sort_by(compare_routes);
    if current == incumbent {
        let current_rank = current
            .last()
            .and_then(|(tier, _)| report.pool_memberships.get(tier))
            .map(|membership| membership.rank);
        let incumbent_rank = incumbent.last().and_then(|(tier, model)| {
            baseline
                .pools
                .get(tier)
                .and_then(|memberships| memberships.iter().find(|item| &item.model == model))
                .map(|membership| membership.rank)
        });
        return Ok(match (current_rank, incumbent_rank) {
            (Some(current), Some(incumbent)) if current < incumbent => {
                FrontierBaselineChange::Better
            }
            (Some(current), Some(incumbent)) if current > incumbent => {
                FrontierBaselineChange::Worse
            }
            _ => FrontierBaselineChange::Unchanged,
        });
    }
    let current_best = current.last();
    let incumbent_best = incumbent.last();
    Ok(match (current_best, incumbent_best) {
        (Some((current_tier, _)), Some((incumbent_tier, _))) if current_tier > incumbent_tier => {
            FrontierBaselineChange::Better
        }
        (Some((current_tier, _)), Some((incumbent_tier, _))) if current_tier < incumbent_tier => {
            FrontierBaselineChange::Worse
        }
        (Some((_, current)), Some((_, incumbent))) => {
            let current_index = thinking_index(&current.thinking)?;
            let incumbent_index = thinking_index(&incumbent.thinking)?;
            if current_index < incumbent_index {
                FrontierBaselineChange::Better
            } else if current_index > incumbent_index {
                FrontierBaselineChange::Worse
            } else {
                FrontierBaselineChange::Unchanged
            }
        }
        (None, Some(_)) => FrontierBaselineChange::Worse,
        (Some(_), None) => FrontierBaselineChange::Better,
        (None, None) => FrontierBaselineChange::Unchanged,
    })
}

fn compare_routes(
    left: &(Tier, ModelIdentity),
    right: &(Tier, ModelIdentity),
) -> std::cmp::Ordering {
    left.0
        .cmp(&right.0)
        .then_with(|| left.1.provider.cmp(&right.1.provider))
        .then_with(|| left.1.model.cmp(&right.1.model))
        .then_with(|| left.1.thinking.cmp(&right.1.thinking))
}

fn validate_baseline_pools(baseline: &FrontierBaseline) -> Result<(), SkillEvalError> {
    let mut routes = BTreeSet::new();
    for (tier, memberships) in &baseline.pools {
        for (index, membership) in memberships.iter().enumerate() {
            let rank = u16::try_from(index + 1)
                .map_err(|_| invalid("baseline pool rank exceeds the supported range"))?;
            if membership.rank != rank
                || membership.model.tier != *tier
                || membership.is_active
                    != (index < memberships.iter().filter(|item| item.is_active).count())
                || !routes.insert((
                    membership.model.provider.clone(),
                    membership.model.model.clone(),
                    membership.model.tier,
                    membership.model.thinking.clone(),
                ))
            {
                return Err(invalid("baseline pool membership is invalid"));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_infrastructure_events(
    state: &FrontierRunState,
) -> Result<(), SkillEvalError> {
    let mut identities = BTreeSet::new();
    for event in &state.infrastructure_events {
        let entrant = state
            .configuration
            .plan
            .entrants
            .iter()
            .find(|entrant| {
                entrant.provider == event.model.provider && entrant.model == event.model.model
            })
            .ok_or_else(|| invalid("infrastructure event belongs to a foreign entrant"))?;
        if !entrant.thinking_levels.contains(&event.model.thinking)
            || event.model.tier < entrant.entry_tier
            || event.attempt == 0
            || event.attempt > u16::from(state.configuration.plan.policy.maximum_trials_per_case)
            || event.infrastructure_attempt == 0
            || event.infrastructure_attempt
                > state
                    .configuration
                    .plan
                    .policy
                    .maximum_infrastructure_attempts
            || event.artifact.0.trim().is_empty()
            || event.case.0.trim().is_empty()
            || event.message.trim().is_empty()
            || event.occurred_at.0.trim().is_empty()
            || !identities.insert((
                event.model.provider.clone(),
                event.model.model.clone(),
                event.model.tier,
                event.model.thinking.clone(),
                event.artifact.clone(),
                event.case.clone(),
                event.attempt,
                event.infrastructure_attempt,
            ))
        {
            return Err(invalid("infrastructure event is incomplete or duplicated"));
        }
    }
    Ok(())
}

pub(crate) fn validate_report_lifecycle(
    state: &FrontierRunState,
    action: &crate::model::FrontierScheduleAction,
) -> Result<(), SkillEvalError> {
    let is_dispatch = matches!(
        action,
        crate::model::FrontierScheduleAction::Dispatch { .. }
    );
    let is_complete = matches!(action, crate::model::FrontierScheduleAction::Complete);
    let is_decision_valid = state.decision.as_ref().is_some_and(|decision| {
        !decision.reason.trim().is_empty()
            && !decision.decided_at.0.trim().is_empty()
            && matches!(
                (state.status, decision.decision),
                (
                    FrontierRunStatus::Accepted,
                    crate::model::Decision::Accepted
                ) | (
                    FrontierRunStatus::Rejected,
                    crate::model::Decision::Rejected
                )
            )
    });
    let is_state_valid = match state.status {
        FrontierRunStatus::Pending => {
            is_dispatch
                && state.spent_millionths_of_dollar == 0
                && state.infrastructure_events.is_empty()
                && state.pause.is_none()
                && state.decision.is_none()
        }
        FrontierRunStatus::Running => {
            is_dispatch && state.pause.is_none() && state.decision.is_none()
        }
        FrontierRunStatus::Paused => {
            let is_pause_consistent = match (&state.pause, action) {
                (
                    Some(crate::model::PoolPauseReason::Quota { .. }),
                    crate::model::FrontierScheduleAction::Dispatch { .. },
                ) => true,
                (
                    Some(crate::model::PoolPauseReason::Infrastructure { .. }),
                    crate::model::FrontierScheduleAction::Dispatch { .. },
                ) => true,
                (Some(stored), crate::model::FrontierScheduleAction::Pause { reason }) => {
                    stored == reason
                }
                _ => false,
            };
            is_pause_consistent && state.decision.is_none()
        }
        FrontierRunStatus::AwaitingDecision => {
            is_complete && state.pause.is_none() && state.decision.is_none()
        }
        FrontierRunStatus::Accepted | FrontierRunStatus::Rejected => {
            is_complete && state.pause.is_none() && is_decision_valid
        }
        FrontierRunStatus::Failed => {
            matches!(
                action,
                crate::model::FrontierScheduleAction::Pause {
                    reason: crate::model::PoolPauseReason::Infrastructure { .. }
                }
            ) && state.pause.is_none()
                && state.decision.is_none()
        }
    };
    if !is_state_valid {
        return Err(invalid("frontier terminal or progress state is invalid"));
    }
    Ok(())
}

pub(crate) fn inspection_matches(
    selector: &crate::model::FrontierTrialSelector,
    inspection: &crate::model::FrontierInspection,
) -> bool {
    match inspection {
        crate::model::FrontierInspection::Trial { trial } => {
            trial.model.provider == selector.provider
                && trial.model.model == selector.model
                && trial.model.tier == selector.tier
                && trial.model.thinking == selector.thinking
                && trial.key.artifact == selector.artifact
                && trial.key.tier == selector.tier
                && trial.key.case == selector.case
                && trial.key.attempt == selector.attempt
        }
        crate::model::FrontierInspection::Infrastructure { event } => {
            event.model.provider == selector.provider
                && event.model.model == selector.model
                && event.model.tier == selector.tier
                && event.model.thinking == selector.thinking
                && event.artifact == selector.artifact
                && event.case == selector.case
                && event.attempt == selector.attempt
                && event.infrastructure_attempt > 0
        }
    }
}

fn thinking_index(thinking: &str) -> Result<usize, SkillEvalError> {
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
        .iter()
        .position(|level| *level == thinking)
        .ok_or_else(|| invalid("baseline thinking level is unsupported"))
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
    total.input_tokens = add_u64(total.input_tokens, usage.input_tokens)?;
    total.output_tokens = add_u64(total.output_tokens, usage.output_tokens)?;
    total.cache_read_tokens = add_u64(total.cache_read_tokens, usage.cache_read_tokens)?;
    total.cache_write_tokens = add_u64(total.cache_write_tokens, usage.cache_write_tokens)?;
    total.turns = total
        .turns
        .checked_add(usage.turns)
        .ok_or_else(|| invalid("frontier usage arithmetic overflow"))?;
    total.tool_calls = total
        .tool_calls
        .checked_add(usage.tool_calls)
        .ok_or_else(|| invalid("frontier usage arithmetic overflow"))?;
    total.elapsed_milliseconds = add_u64(total.elapsed_milliseconds, usage.elapsed_milliseconds)?;
    total.cost_millionths_of_dollar = add_u64(
        total.cost_millionths_of_dollar,
        usage.cost_millionths_of_dollar,
    )?;
    Ok(())
}

fn add_u64(left: u64, right: u64) -> Result<u64, SkillEvalError> {
    left.checked_add(right)
        .ok_or_else(|| invalid("frontier usage arithmetic overflow"))
}

fn invalid(message: &str) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(format!("frontier report: {message}"))
}
