use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use sha2::{Digest, Sha256};

use crate::model::{
    ArtifactChange, ArtifactDefinition, ArtifactDiscovery, ArtifactKind, ArtifactName,
    ArtifactQualificationState, ArtifactReport, ArtifactStatus, AuditBrief, AuditBriefRequest,
    CandidateArtifact, CandidateEnvironmentEntry, CaseDiscovery, CaseId, Decision, DecisionRecord,
    EvidenceRole, FrontierApplyReport, FrontierDecisionRequest, FrontierInspection,
    FrontierPreviewReport, FrontierReport, FrontierRunId, FrontierRunState, JudgeInput,
    ModelIdentity, ParentResponsibility, PauseReason, PoolChildRun, PoolChildStatus, PoolEntrant,
    PoolPauseReason, PoolQualifyRequest, PoolRunConfiguration, PoolRunId, PoolRunState,
    PoolRunStatus, PoolStage, PromptJudgeRequest, PromptJudgeResult, PublicationGate,
    PublicationStatus, QualificationBoundary, QualificationPolicy, QualificationPurpose,
    QualificationReport, QualifyRequest, RunConfiguration, RunEvent, RunId, RunMode, RunState,
    RunStatus, SkillEvalError, SkillRoutingDecision, T1ScreenAttemptEvidence,
    T1ScreenAttemptReport, T1ScreenCampaignState, T1ScreenCampaignStatus, T1ScreenCapExtension,
    T1ScreenCapExtensionRequest, T1ScreenCaseReport, T1ScreenChildRun, T1ScreenChildStatus,
    T1ScreenModelOutcome, T1ScreenModelReport, T1ScreenPauseReason, T1ScreenRankedRoute,
    T1ScreenRankingInputs, T1ScreenRankingReport, T1ScreenReport, T1ScreenRouteFailure,
    T1ScreenRouteFailureRequest, T1ScreenRunId, T1ScreenRunState, T1ScreenRunStatus, Tier,
    TierAssignment, TierDestination, TierEvidence, TierStatus, TrialKey, TrialRecord,
    TrialSelector, TrialUsage,
};
use crate::ports::{
    Clock, FrontierProgressSink, FrontierRuntime, PoolProgressSink, PoolRuntime, ProgressSink,
    QualificationRuntime, RunStore, T1ScreenProgressSink, T1ScreenRuntime, TierWriter,
};
use crate::statistics::{
    evaluate_calibration, evaluate_qualification, qualification_start_index, rank_pool,
    select_qualification_thinking_level, select_thinking_level,
};
use crate::t1_screen_store::{
    candidate_environment_manifest_digest, t1_screen_effective_caps, validate_t1_screen_state,
};

/// Builds the immutable pending form of a stored T1 screen for the T117 resume contract.
///
/// The input is one validated stored state. The output preserves frozen configuration and child
/// identities while clearing progress. It returns an error for invalid stored state.
pub(crate) fn pending_t1_screen_state(
    stored: &T1ScreenRunState,
) -> Result<T1ScreenRunState, SkillEvalError> {
    validate_t1_screen_state(stored)?;
    let infrastructure_failed_models = stored
        .models
        .iter()
        .enumerate()
        .filter_map(|(index, model)| {
            matches!(
                model.outcome,
                Some(T1ScreenModelOutcome::InfrastructureFailed { .. })
            )
            .then_some(index)
        })
        .collect::<BTreeSet<_>>();
    Ok(T1ScreenRunState {
        configuration: stored.configuration.clone(),
        cap_extensions: stored.cap_extensions.clone(),
        route_failures: stored.route_failures.clone(),
        status: T1ScreenRunStatus::Pending,
        child_runs: stored
            .child_runs
            .iter()
            .cloned()
            .map(|mut child| {
                if !infrastructure_failed_models
                    .contains(&usize::try_from(child.model_index).unwrap_or(usize::MAX))
                {
                    child.status = T1ScreenChildStatus::Pending;
                }
                child
            })
            .collect(),
        models: stored
            .models
            .iter()
            .enumerate()
            .map(|(index, model)| {
                if infrastructure_failed_models.contains(&index) {
                    return model.clone();
                }
                crate::model::T1ScreenModelState {
                    provider: model.provider.clone(),
                    model: model.model.clone(),
                    attempts: Vec::new(),
                    outcome: None,
                }
            })
            .collect(),
        candidate_usage: if stored.route_failures.is_empty() {
            t1_zero_usage()
        } else {
            stored.candidate_usage.clone()
        },
        judge_usage: if stored.route_failures.is_empty() {
            t1_zero_usage()
        } else {
            stored.judge_usage.clone()
        },
        spent_judge_millionths_of_dollar: if stored.route_failures.is_empty() {
            0
        } else {
            stored.spent_judge_millionths_of_dollar
        },
        pause: None,
    })
}

/// Builds one complete read-only T1 screening report from durable parent and child evidence.
///
/// The inputs are a safe screening identifier plus read-only parent and child stores. The output
/// retains inventory, attempts, five case slots, usage, and an optional terminal ranking. It
/// returns an error for missing, malformed, incomplete, or nonzero-cost selected evidence.
pub(crate) fn build_t1_screen_report(
    run_id: &T1ScreenRunId,
    parent_store: &dyn crate::ports::T1ScreenStore,
    child_store: &dyn RunStore,
) -> Result<T1ScreenReport, SkillEvalError> {
    let state = parent_store.load_t1_screen(run_id)?;
    validate_t1_screen_state(&state)?;
    let campaign = parent_store.load_t1_screen_campaign(&state.configuration.campaign_id)?;
    validate_t1_report_campaign(&state, &campaign)?;
    let campaign_remaining = campaign
        .approved_judge_total_millionths_of_dollar
        .checked_sub(campaign.aggregate_judge_spent_millionths_of_dollar)
        .ok_or_else(|| t1_invalid("campaign remaining spend underflow"))?;
    let models = build_t1_model_reports(&state, child_store)?;
    let ranking = build_t1_ranking(&state)?;
    let eligible_count = u64::try_from(state.configuration.eligible.len())
        .map_err(|_| t1_invalid("eligible count exceeds the supported range"))?;
    let excluded_count = u64::try_from(state.configuration.excluded.len())
        .map_err(|_| t1_invalid("excluded count exceeds the supported range"))?;
    let total_inventory_count = eligible_count
        .checked_add(excluded_count)
        .ok_or_else(|| t1_invalid("inventory count arithmetic overflow"))?;
    let active_child_run_id = state
        .child_runs
        .iter()
        .find(|child| {
            matches!(
                child.status,
                T1ScreenChildStatus::Running | T1ScreenChildStatus::Paused
            )
        })
        .map(|child| child.run_id.clone());
    let configuration = &state.configuration;
    let candidate_environment_manifest_entry_count =
        u64::try_from(configuration.candidate_environment.manifest.len()).map_err(|_| {
            t1_invalid("candidate environment manifest entry count exceeds the supported range")
        })?;
    let (effective_owner_cap, effective_provider_cap) = t1_screen_effective_caps(&state)?;
    Ok(T1ScreenReport {
        run_id: configuration.run_id.clone(),
        campaign_id: campaign.campaign_id.clone(),
        campaign_approved_judge_total_millionths_of_dollar: campaign
            .approved_judge_total_millionths_of_dollar,
        campaign_aggregate_judge_spent_millionths_of_dollar: campaign
            .aggregate_judge_spent_millionths_of_dollar,
        campaign_remaining_judge_millionths_of_dollar: campaign_remaining,
        campaign_runs: campaign.runs.clone(),
        campaign_active_run_id: campaign.active_run_id.clone(),
        campaign_status: campaign.status,
        created_at: configuration.created_at.clone(),
        status: state.status,
        snapshot: configuration.capability_snapshot.clone(),
        total_inventory_count,
        eligible_count,
        excluded_count,
        eligible: configuration.eligible.clone(),
        excluded: configuration.excluded.clone(),
        exam: configuration.exam.clone(),
        judge: configuration.judge.clone(),
        candidate_environment: configuration.candidate_environment.clone(),
        candidate_environment_manifest_digest: configuration.candidate_environment.digest.clone(),
        candidate_environment_manifest_entry_count,
        policy: configuration.policy.clone(),
        candidate_calls: configuration.candidate_calls.clone(),
        judge_calls: configuration.judge_calls.clone(),
        owner_approved_judge_cap_millionths_of_dollar: configuration
            .owner_approved_judge_cap_millionths_of_dollar,
        provider_enforced_judge_cap_millionths_of_dollar: configuration
            .provider_enforced_judge_cap_millionths_of_dollar,
        effective_owner_approved_judge_cap_millionths_of_dollar: effective_owner_cap,
        effective_provider_enforced_judge_cap_millionths_of_dollar: effective_provider_cap,
        cap_extensions: state.cap_extensions.clone(),
        route_failures: state.route_failures.clone(),
        spent_judge_millionths_of_dollar: state.spent_judge_millionths_of_dollar,
        candidate_usage: state.candidate_usage.clone(),
        judge_usage: state.judge_usage.clone(),
        active_child_run_id,
        pause: state.pause.clone(),
        child_runs: state.child_runs.clone(),
        models,
        ranking,
        is_owner_approval_required: true,
    })
}

/// Records one exact infrastructure-failed T1 route and returns the saved report.
///
/// The request, parent and child stores, and local clock identify and authorize the transition.
/// The output is the persisted report. Invalid pause evidence, identity, history, timestamps, or
/// persistence returns an error.
pub(crate) fn fail_t1_screen_route(
    request: &T1ScreenRouteFailureRequest,
    parent_store: &mut dyn crate::ports::T1ScreenStore,
    child_store: &dyn RunStore,
    clock: &dyn Clock,
) -> Result<T1ScreenReport, SkillEvalError> {
    if request.owner_reason.trim().is_empty() {
        return Err(t1_invalid("route failure owner reason is blank"));
    }
    let mut state = parent_store.load_t1_screen(&request.run_id)?;
    validate_t1_screen_state(&state)?;
    let pause_message = match &state.pause {
        Some(T1ScreenPauseReason::Infrastructure { message })
            if state.status == T1ScreenRunStatus::Paused =>
        {
            message.clone()
        }
        _ => {
            return Err(t1_invalid(
                "route failure requires a paused infrastructure run",
            ));
        }
    };
    let paused_children = state
        .child_runs
        .iter()
        .enumerate()
        .filter(|(_, child)| child.status == T1ScreenChildStatus::Paused)
        .collect::<Vec<_>>();
    if paused_children.len() != 1 || paused_children[0].1.run_id != request.child_run_id {
        return Err(t1_invalid(
            "route failure must name the one active paused child",
        ));
    }
    let child_index = paused_children[0].0;
    let child = state.child_runs[child_index].clone();
    let model_index = usize::try_from(child.model_index)
        .map_err(|_| t1_invalid("route failure model index arithmetic overflow"))?;
    let thinking_index = usize::try_from(child.thinking_index)
        .map_err(|_| t1_invalid("route failure thinking index arithmetic overflow"))?;
    let model = state
        .models
        .get(model_index)
        .ok_or_else(|| t1_invalid("route failure child has no model state"))?;
    if model.outcome.is_some()
        || model.attempts.len() != thinking_index
        || state.child_runs.iter().any(|sibling| {
            sibling.model_index == child.model_index
                && (sibling.thinking_index < child.thinking_index
                    && sibling.status != T1ScreenChildStatus::Completed
                    || sibling.thinking_index > child.thinking_index
                        && sibling.status != T1ScreenChildStatus::Pending)
        })
    {
        return Err(t1_invalid(
            "route failure child is not the adjacent active thinking route",
        ));
    }
    if state
        .route_failures
        .iter()
        .any(|failure| failure.child_run_id == child.run_id)
    {
        return Err(t1_invalid("route failure child was already recorded"));
    }
    if state
        .route_failures
        .iter()
        .any(|failure| failure.model == child.model)
    {
        return Err(t1_invalid("exact model route failure was already recorded"));
    }
    validate_paused_t1_child(&child, &pause_message, child_store)?;

    let campaign_before = parent_store.load_t1_screen_campaign(&state.configuration.campaign_id)?;
    if campaign_before.status != T1ScreenCampaignStatus::Paused
        || campaign_before.active_run_id.as_ref() != Some(&state.configuration.run_id)
    {
        return Err(t1_invalid(
            "route failure campaign is not paused on the exact parent run",
        ));
    }
    let timestamp = clock.now();
    if timestamp.0 <= state.configuration.created_at.0
        || state
            .cap_extensions
            .iter()
            .any(|extension| extension.timestamp.0 >= timestamp.0)
        || state
            .route_failures
            .iter()
            .any(|failure| failure.timestamp.0 >= timestamp.0)
    {
        return Err(t1_invalid(
            "route failure timestamp is not globally later than authority history",
        ));
    }
    state.route_failures.push(T1ScreenRouteFailure {
        timestamp,
        child_run_id: child.run_id.clone(),
        model: child.model.clone(),
        paused_message_sha256: Sha256::digest(pause_message.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        owner_reason: request.owner_reason.clone(),
    });
    state.child_runs[child_index].status = T1ScreenChildStatus::Failed;
    for sibling in state.child_runs.iter_mut().filter(|sibling| {
        sibling.model_index == child.model_index && sibling.thinking_index > child.thinking_index
    }) {
        sibling.status = T1ScreenChildStatus::Skipped;
    }
    state.models[model_index].outcome = Some(T1ScreenModelOutcome::InfrastructureFailed {
        model: child.model,
        child_run_id: child.run_id,
    });
    state.status = T1ScreenRunStatus::Running;
    state.pause = None;
    validate_t1_screen_state(&state)?;
    parent_store.save_t1_screen(&state)?;
    let campaign = parent_store.reconcile_t1_screen_campaign_run(&state)?;
    if campaign.status != T1ScreenCampaignStatus::Open
        || campaign.active_run_id.as_ref() != Some(&state.configuration.run_id)
        || campaign.aggregate_judge_spent_millionths_of_dollar
            != campaign_before.aggregate_judge_spent_millionths_of_dollar
        || campaign.approved_judge_total_millionths_of_dollar
            != campaign_before.approved_judge_total_millionths_of_dollar
    {
        return Err(t1_invalid(
            "route failure campaign reconciliation changed authority or spend",
        ));
    }
    build_t1_screen_report(&request.run_id, parent_store, child_store)
}

fn validate_paused_t1_child(
    child: &T1ScreenChildRun,
    pause_message: &str,
    child_store: &dyn RunStore,
) -> Result<(), SkillEvalError> {
    let mut is_started = false;
    let mut is_exact_route_seen = false;
    let mut active_pause = None;
    child_store.replay(&child.run_id, &mut |event| {
        match event {
            RunEvent::RunStarted { configuration, .. } => {
                if is_started || configuration.run_id != child.run_id {
                    return Err(t1_invalid("route failure child run identity differs"));
                }
                is_started = true;
            }
            RunEvent::TrialStarted { models, .. } => {
                if models.as_slice() != [child.model.clone()] {
                    return Err(t1_invalid("route failure child model identity differs"));
                }
                is_exact_route_seen = true;
            }
            RunEvent::CandidateExecuted { candidate, .. } => {
                if candidate.model != child.model {
                    return Err(t1_invalid("route failure candidate identity differs"));
                }
            }
            RunEvent::RunPaused { reason, .. } => active_pause = Some(reason),
            RunEvent::RunResumed { .. } => active_pause = None,
            _ => {}
        }
        Ok(())
    })?;
    if !is_started || !is_exact_route_seen {
        return Err(t1_invalid(
            "route failure child route evidence is incomplete",
        ));
    }
    match active_pause {
        Some(PauseReason::Infrastructure { message }) if message == pause_message => Ok(()),
        _ => Err(t1_invalid(
            "route failure child pause evidence differs from the parent pause",
        )),
    }
}

pub(crate) fn extend_t1_screen_cap(
    request: &T1ScreenCapExtensionRequest,
    parent_store: &mut dyn crate::ports::T1ScreenStore,
    child_store: &dyn RunStore,
    clock: &dyn Clock,
) -> Result<T1ScreenReport, SkillEvalError> {
    let mut state = parent_store.load_t1_screen(&request.run_id)?;
    validate_t1_screen_state(&state)?;
    if state.status != T1ScreenRunStatus::Paused
        || !matches!(state.pause, Some(T1ScreenPauseReason::JudgeCap { .. }))
    {
        return Err(t1_invalid("cap extension requires a paused judge-cap run"));
    }
    if request.owner_reason.trim().is_empty() {
        return Err(t1_invalid("cap extension owner reason is blank"));
    }
    let (previous_owner, previous_provider) = t1_screen_effective_caps(&state)?;
    if request.new_owner_cap_millionths_of_dollar <= previous_owner
        || request.new_provider_cap_millionths_of_dollar <= previous_provider
    {
        return Err(t1_invalid("cap extension must strictly increase both caps"));
    }
    if request.new_provider_cap_millionths_of_dollar > request.new_owner_cap_millionths_of_dollar {
        return Err(t1_invalid("cap extension provider cap exceeds owner cap"));
    }
    state.cap_extensions.push(T1ScreenCapExtension {
        timestamp: clock.now(),
        previous_owner_cap_millionths_of_dollar: previous_owner,
        new_owner_cap_millionths_of_dollar: request.new_owner_cap_millionths_of_dollar,
        previous_provider_cap_millionths_of_dollar: previous_provider,
        new_provider_cap_millionths_of_dollar: request.new_provider_cap_millionths_of_dollar,
        owner_reason: request.owner_reason.clone(),
    });
    validate_t1_screen_state(&state)?;
    parent_store.save_t1_screen(&state)?;
    build_t1_screen_report(&request.run_id, parent_store, child_store)
}

fn build_t1_model_reports(
    state: &T1ScreenRunState,
    child_store: &dyn RunStore,
) -> Result<Vec<T1ScreenModelReport>, SkillEvalError> {
    let mut reports = Vec::with_capacity(state.models.len());
    for (model_index, model) in state.models.iter().enumerate() {
        let model_index = u64::try_from(model_index)
            .map_err(|_| t1_invalid("model index arithmetic overflow"))?;
        let mut attempts = Vec::new();
        for child in state
            .child_runs
            .iter()
            .filter(|child| child.model_index == model_index)
            .filter(|child| {
                !matches!(
                    child.status,
                    T1ScreenChildStatus::Pending | T1ScreenChildStatus::Skipped
                )
            })
        {
            let evidence = model
                .attempts
                .iter()
                .find(|attempt| attempt.child_run_id == child.run_id)
                .map(|attempt| attempt.evidence.clone());
            let cases = build_t1_case_reports(state, child, child_store)?;
            if matches!(
                child.status,
                T1ScreenChildStatus::Completed | T1ScreenChildStatus::Exhausted
            ) && evidence.is_none()
            {
                return Err(t1_invalid(
                    "terminal T1 child has no aggregate attempt evidence",
                ));
            }
            attempts.push(T1ScreenAttemptReport {
                child_run_id: child.run_id.clone(),
                model: child.model.clone(),
                status: child.status,
                evidence,
                cases,
            });
        }
        reports.push(T1ScreenModelReport {
            provider: model.provider.clone(),
            model: model.model.clone(),
            attempts,
            outcome: model.outcome.clone(),
        });
    }
    Ok(reports)
}

fn build_t1_case_reports(
    state: &T1ScreenRunState,
    child: &T1ScreenChildRun,
    child_store: &dyn RunStore,
) -> Result<Vec<T1ScreenCaseReport>, SkillEvalError> {
    let mut candidates = BTreeMap::<CaseId, CandidateArtifact>::new();
    let mut trials = BTreeMap::<CaseId, TrialRecord>::new();
    match child_store.replay(&child.run_id, &mut |event| {
        match event {
            RunEvent::CandidateExecuted { candidate, .. } => {
                if candidates
                    .insert(candidate.key.case.clone(), candidate)
                    .is_some()
                {
                    return Err(t1_invalid("T1 case has duplicate candidate evidence"));
                }
            }
            RunEvent::TrialCompleted { record, .. } => {
                if trials.insert(record.key.case.clone(), record).is_some() {
                    return Err(t1_invalid("T1 case has duplicate completed evidence"));
                }
            }
            _ => {}
        }
        Ok(())
    }) {
        Ok(()) => {}
        Err(SkillEvalError::NotFound(_))
            if matches!(
                child.status,
                T1ScreenChildStatus::Running | T1ScreenChildStatus::Paused
            ) => {}
        Err(error) => return Err(error),
    }
    state
        .configuration
        .exam
        .cases
        .iter()
        .map(|case| {
            let candidate = candidates.remove(&case.id);
            let trial = trials.remove(&case.id);
            if trial.is_some() && candidate.is_none() {
                return Err(t1_invalid("T1 completed case has no candidate checkpoint"));
            }
            Ok(T1ScreenCaseReport {
                case: case.id.clone(),
                candidate,
                trial,
            })
        })
        .collect()
}

fn build_t1_ranking(
    state: &T1ScreenRunState,
) -> Result<Option<T1ScreenRankingReport>, SkillEvalError> {
    let is_terminal = state.status == T1ScreenRunStatus::AwaitingOwner
        && state.models.iter().all(|model| model.outcome.is_some());
    if !is_terminal {
        return Ok(None);
    }
    let mut ranked = state
        .models
        .iter()
        .filter_map(|model| match &model.outcome {
            Some(T1ScreenModelOutcome::Selected { model: selected }) => Some((model, selected)),
            _ => None,
        })
        .map(|(model, selected)| {
            let evidence = model
                .attempts
                .iter()
                .find(|attempt| attempt.evidence.requested_model == *selected)
                .map(|attempt| &attempt.evidence)
                .ok_or_else(|| t1_invalid("selected T1 route has no attempt evidence"))?;
            if !evidence.is_passing {
                return Err(t1_invalid(
                    "selected T1 route does not pass the frozen floors",
                ));
            }
            if evidence.candidate_usage.cost_millionths_of_dollar != 0 {
                return Err(t1_invalid(
                    "selected T1 candidate cost must be exactly zero before ranking",
                ));
            }
            if evidence.completed_trials == 0 {
                return Err(t1_invalid("selected T1 route has no completed trials"));
            }
            Ok(T1ScreenRankedRoute {
                rank: 0,
                model: selected.clone(),
                ranking_inputs: T1ScreenRankingInputs {
                    candidate_cost_millionths_of_dollar: evidence
                        .candidate_usage
                        .cost_millionths_of_dollar,
                    candidate_latency_milliseconds: evidence.candidate_usage.elapsed_milliseconds,
                    candidate_failed_trials: evidence.failed_trials,
                    candidate_completed_trials: evidence.completed_trials,
                    provider: selected.provider.clone(),
                    model: selected.model.clone(),
                    thinking: selected.thinking.clone(),
                },
            })
        })
        .collect::<Result<Vec<_>, SkillEvalError>>()?;
    ranked.sort_by(compare_t1_routes);
    for (index, route) in ranked.iter_mut().enumerate() {
        route.rank = u64::try_from(index + 1)
            .map_err(|_| t1_invalid("T1 rank exceeds the supported range"))?;
    }
    let passing_route_count = u64::try_from(ranked.len())
        .map_err(|_| t1_invalid("passing route count exceeds the supported range"))?;
    let recommendation_shortage_count = 3_usize.saturating_sub(ranked.len()) as u8;
    let (recommendations, alternates) = if recommendation_shortage_count == 0 {
        (ranked[..3].to_vec(), ranked[3..].to_vec())
    } else {
        (Vec::new(), ranked)
    };
    Ok(Some(T1ScreenRankingReport {
        passing_route_count,
        recommendation_shortage_count,
        recommendations,
        alternates,
    }))
}

fn compare_t1_routes(
    left: &T1ScreenRankedRoute,
    right: &T1ScreenRankedRoute,
) -> std::cmp::Ordering {
    let left_inputs = &left.ranking_inputs;
    let right_inputs = &right.ranking_inputs;
    left_inputs
        .candidate_cost_millionths_of_dollar
        .cmp(&right_inputs.candidate_cost_millionths_of_dollar)
        .then_with(|| {
            left_inputs
                .candidate_latency_milliseconds
                .cmp(&right_inputs.candidate_latency_milliseconds)
        })
        .then_with(|| {
            let left_failures = u64::from(left_inputs.candidate_failed_trials)
                * u64::from(right_inputs.candidate_completed_trials);
            let right_failures = u64::from(right_inputs.candidate_failed_trials)
                * u64::from(left_inputs.candidate_completed_trials);
            left_failures.cmp(&right_failures)
        })
        .then_with(|| left_inputs.provider.cmp(&right_inputs.provider))
        .then_with(|| left_inputs.model.cmp(&right_inputs.model))
        .then_with(|| left_inputs.thinking.cmp(&right_inputs.thinking))
}

/// Starts one preallocated T1 screening state and advances it until completion or pause.
///
/// The inputs are the complete pending state, its screening runtime, and a progress sink. The
/// output is the last durable parent state. It returns errors for invalid state, identity drift,
/// malformed evidence, arithmetic overflow, or boundary failures that cannot be paused.
pub(crate) fn start_t1_screening(
    state: T1ScreenRunState,
    runtime: &mut dyn T1ScreenRuntime,
    progress: &mut dyn T1ScreenProgressSink,
) -> Result<T1ScreenRunState, SkillEvalError> {
    validate_t1_screen_state(&state)?;
    validate_t1_screen_environment(&state, &state, runtime)?;
    let campaign = runtime.reconcile_t1_screen_campaign(&state.configuration.campaign_id)?;
    validate_t1_campaign_start(&state, &campaign, runtime)?;
    match runtime.load_t1_screen(&state.configuration.run_id) {
        Ok(stored) if stored == state => {}
        Ok(_) => return Err(t1_drift("pending start retry state")),
        Err(SkillEvalError::NotFound(_)) => runtime.create_t1_screen(&state)?,
        Err(error) => return Err(error),
    }
    runtime.register_t1_screen_campaign_run(&state)?;
    progress.emit_t1_screen(&state)?;
    continue_t1_screening(state, runtime, progress)
}

/// Resumes one T1 screening state against its original preallocated identity.
///
/// The inputs are the original pending state, its screening runtime, and a progress sink. The
/// output is the last durable parent state. It returns errors before candidate or judge calls when
/// any frozen identity drifted, or when stored evidence and arithmetic are invalid.
pub(crate) fn resume_t1_screening(
    expected: &T1ScreenRunState,
    runtime: &mut dyn T1ScreenRuntime,
    progress: &mut dyn T1ScreenProgressSink,
) -> Result<T1ScreenRunState, SkillEvalError> {
    validate_t1_screen_state(expected)?;
    validate_pending_t1_screen(expected)?;
    let mut state = runtime.load_t1_screen(&expected.configuration.run_id)?;
    validate_t1_screen_environment(expected, &state, runtime)?;
    runtime.reconcile_t1_screen_campaign(&state.configuration.campaign_id)?;
    validate_t1_campaign_resume(&state, runtime)?;
    sync_t1_usage(&mut state, runtime, progress)?;
    if state.status == T1ScreenRunStatus::AwaitingOwner
        || state.cap_extensions.is_empty()
            && matches!(state.pause, Some(T1ScreenPauseReason::JudgeCap { .. }))
    {
        return Ok(state);
    }
    continue_t1_screening(state, runtime, progress)
}

fn validate_t1_campaign_start(
    state: &T1ScreenRunState,
    campaign: &T1ScreenCampaignState,
    runtime: &mut dyn T1ScreenRuntime,
) -> Result<(), SkillEvalError> {
    if campaign.campaign_id != state.configuration.campaign_id {
        return Err(t1_invalid(
            "campaign identity differs from the parent state",
        ));
    }
    if campaign.status != T1ScreenCampaignStatus::Open
        || campaign
            .active_run_id
            .as_ref()
            .is_some_and(|run_id| run_id != &state.configuration.run_id)
    {
        return Err(t1_invalid("campaign is not open for this run"));
    }
    if state
        .configuration
        .candidate_environment
        .manifest
        .is_empty()
    {
        return Err(t1_invalid("candidate environment manifest is not named"));
    }
    let remaining = campaign
        .approved_judge_total_millionths_of_dollar
        .checked_sub(campaign.aggregate_judge_spent_millionths_of_dollar)
        .ok_or_else(|| t1_invalid("campaign remaining spend underflow"))?;
    let upper_bound =
        runtime.conservative_next_judge_cost_upper_bound(&state.configuration.judge)?;
    if remaining < upper_bound {
        runtime.pause_t1_screen_campaign_for_budget(&state.configuration.campaign_id)?;
        return Err(t1_invalid(format!(
            "campaign remaining {remaining} is below conservative next judge upper bound {upper_bound}"
        )));
    }
    Ok(())
}

fn validate_t1_campaign_resume(
    state: &T1ScreenRunState,
    runtime: &dyn T1ScreenRuntime,
) -> Result<(), SkillEvalError> {
    if state
        .configuration
        .candidate_environment
        .manifest
        .is_empty()
    {
        return Err(t1_invalid("candidate environment manifest is not named"));
    }
    let campaign = runtime.load_t1_screen_campaign(&state.configuration.campaign_id)?;
    if state.status == T1ScreenRunStatus::AwaitingOwner {
        if campaign.status != T1ScreenCampaignStatus::AwaitingOwner
            || campaign.active_run_id.is_some()
        {
            return Err(t1_invalid("awaiting-owner campaign state differs"));
        }
        return Ok(());
    }
    if campaign.active_run_id.as_ref() != Some(&state.configuration.run_id) {
        return Err(t1_invalid("campaign active run differs from resume run"));
    }
    let entry = campaign
        .runs
        .iter()
        .find(|entry| entry.run_id == state.configuration.run_id)
        .ok_or_else(|| t1_invalid("campaign active run is not registered"))?;
    if !entry.is_resumable {
        return Err(t1_invalid("campaign active run is not resumable"));
    }
    let remaining = campaign
        .approved_judge_total_millionths_of_dollar
        .checked_sub(campaign.aggregate_judge_spent_millionths_of_dollar)
        .ok_or_else(|| t1_invalid("campaign remaining spend underflow"))?;
    let upper_bound =
        runtime.conservative_next_judge_cost_upper_bound(&state.configuration.judge)?;
    if remaining < upper_bound {
        return Err(t1_invalid(format!(
            "campaign remaining {remaining} is below conservative next judge upper bound {upper_bound}"
        )));
    }
    Ok(())
}

fn validate_t1_report_campaign(
    state: &T1ScreenRunState,
    campaign: &T1ScreenCampaignState,
) -> Result<(), SkillEvalError> {
    if campaign.campaign_id != state.configuration.campaign_id {
        return Err(t1_invalid("report campaign identity differs"));
    }
    let entry = campaign
        .runs
        .iter()
        .find(|entry| entry.run_id == state.configuration.run_id)
        .ok_or_else(|| t1_invalid("report run is not registered in its campaign"))?;
    if entry.judge_spend_millionths_of_dollar != state.spent_judge_millionths_of_dollar
        || entry.candidate_cost_millionths_of_dollar
            != state.candidate_usage.cost_millionths_of_dollar
        || entry.observed_status != state.status
    {
        return Err(t1_invalid("report campaign audit entry is stale"));
    }
    Ok(())
}

fn validate_pending_t1_screen(state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
    if state.status != T1ScreenRunStatus::Pending || state.pause.is_some() {
        return Err(t1_invalid("resume identity is not pending"));
    }
    for (model_index, model) in state.models.iter().enumerate() {
        let is_infrastructure_failed = matches!(
            model.outcome,
            Some(T1ScreenModelOutcome::InfrastructureFailed { .. })
        );
        if !is_infrastructure_failed
            && (!model.attempts.is_empty()
                || model.outcome.is_some()
                || state.child_runs.iter().any(|child| {
                    child.model_index == u64::try_from(model_index).unwrap_or(u64::MAX)
                        && child.status != T1ScreenChildStatus::Pending
                }))
        {
            return Err(t1_invalid("resume identity contains mutable progress"));
        }
    }
    if state.route_failures.is_empty()
        && (state.candidate_usage != t1_zero_usage()
            || state.judge_usage != t1_zero_usage()
            || state.spent_judge_millionths_of_dollar != 0)
    {
        return Err(t1_invalid("resume identity is not unspent"));
    }
    Ok(())
}

fn continue_t1_screening(
    mut state: T1ScreenRunState,
    runtime: &mut dyn T1ScreenRuntime,
    progress: &mut dyn T1ScreenProgressSink,
) -> Result<T1ScreenRunState, SkillEvalError> {
    if state.status == T1ScreenRunStatus::Pending {
        state.status = T1ScreenRunStatus::Running;
        save_t1_screen_and_emit(runtime, progress, &state)?;
    }

    loop {
        let Some(child_index) = next_t1_child_index(&state)? else {
            if state.status != T1ScreenRunStatus::AwaitingOwner {
                state.status = T1ScreenRunStatus::AwaitingOwner;
                state.pause = None;
                save_t1_screen_and_emit(runtime, progress, &state)?;
            }
            return Ok(state);
        };
        let is_resuming = state.child_runs[child_index].status == T1ScreenChildStatus::Paused;
        if state.status == T1ScreenRunStatus::Paused {
            if !is_resuming {
                return Err(t1_drift("parent pause and active child"));
            }
            state.status = T1ScreenRunStatus::Running;
            state.pause = None;
        }
        if matches!(
            state.child_runs[child_index].status,
            T1ScreenChildStatus::Pending | T1ScreenChildStatus::Paused
        ) {
            state.child_runs[child_index].status = T1ScreenChildStatus::Running;
            save_t1_screen_and_emit(runtime, progress, &state)?;
        }

        match run_t1_child(&mut state, child_index, is_resuming, runtime, progress)? {
            T1ChildResult::Completed => {
                complete_t1_child(&mut state, child_index, runtime, progress)?;
            }
            T1ChildResult::Paused(reason) => {
                state.child_runs[child_index].status = T1ScreenChildStatus::Paused;
                state.status = T1ScreenRunStatus::Paused;
                state.pause = Some(reason);
                save_t1_screen_and_emit(runtime, progress, &state)?;
                return Ok(state);
            }
        }
    }
}

fn validate_t1_screen_environment<R: T1ScreenRuntime + ?Sized>(
    expected: &T1ScreenRunState,
    state: &T1ScreenRunState,
    runtime: &R,
) -> Result<(), SkillEvalError> {
    validate_t1_screen_state(state)?;
    if expected.configuration != state.configuration {
        return Err(t1_drift("frozen configuration"));
    }
    if expected.cap_extensions != state.cap_extensions {
        return Err(t1_drift("cap extension history"));
    }
    if expected.route_failures != state.route_failures {
        return Err(t1_drift("route failure history"));
    }
    if expected.child_runs.len() != state.child_runs.len()
        || expected
            .child_runs
            .iter()
            .zip(&state.child_runs)
            .any(|(expected, stored)| t1_child_identity(expected) != t1_child_identity(stored))
    {
        return Err(t1_drift("preallocated child identities"));
    }

    let current_manifest = runtime.candidate_environment_manifest()?;
    let current_digest = candidate_environment_manifest_digest(&current_manifest)?;
    if current_digest != state.configuration.candidate_environment.digest
        || current_manifest != state.configuration.candidate_environment.manifest
    {
        let difference = t1_environment_difference(
            &state.configuration.candidate_environment.manifest,
            &current_manifest,
        )
        .unwrap_or_else(|| "manifest digest changed without an entry difference".to_owned());
        return Err(t1_invalid(format!(
            "candidate environment drift: {difference}"
        )));
    }

    let snapshot =
        runtime.capability_snapshot_bytes(&state.configuration.capability_snapshot.path)?;
    let digest = Sha256::digest(snapshot)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if digest != state.configuration.capability_snapshot.sha256 {
        return Err(t1_drift("capability snapshot bytes or digest"));
    }
    if runtime.load(&state.configuration.exam.root)? != state.configuration.exam {
        return Err(t1_drift("fixed exam root, revision, or cases"));
    }
    if state.configuration.judge.tier <= Tier::T1 {
        return Err(t1_drift("judge identity"));
    }
    for (case, frozen_harness) in state
        .configuration
        .exam
        .cases
        .iter()
        .zip(&state.configuration.candidate_environment.harnesses)
    {
        if runtime.identity(&state.configuration.exam, &case.execution)? != *frozen_harness {
            return Err(t1_drift("candidate environment identity"));
        }
    }
    for child in &state.child_runs {
        if runtime.exact_candidate(&child.model)? != child.model {
            return Err(t1_drift("exact model capability"));
        }
        let judge = runtime.pool_judge(&child.model)?;
        if judge != state.configuration.judge
            || judge.provider == child.model.provider && judge.model == child.model.model
        {
            return Err(t1_drift("judge identity"));
        }
    }
    validate_t1_progression(state)
}

fn t1_child_identity(child: &T1ScreenChildRun) -> (&ModelIdentity, &RunId, u64, u64) {
    (
        &child.model,
        &child.run_id,
        child.model_index,
        child.thinking_index,
    )
}

fn validate_t1_progression(state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
    let mut is_unfinished_seen = false;
    for (model_index, model) in state.models.iter().enumerate() {
        let model_index = u64::try_from(model_index)
            .map_err(|_| t1_invalid("model index arithmetic overflow"))?;
        let children = state
            .child_runs
            .iter()
            .filter(|child| child.model_index == model_index)
            .collect::<Vec<_>>();
        if is_unfinished_seen
            && (model.outcome.is_some()
                || !model.attempts.is_empty()
                || children
                    .iter()
                    .any(|child| child.status != T1ScreenChildStatus::Pending))
        {
            return Err(t1_drift("frozen provider and model walk order"));
        }
        if model.outcome.is_none() {
            is_unfinished_seen = true;
            let attempt_count = model.attempts.len();
            if attempt_count == children.len() {
                return Err(t1_drift("complete thinking evidence without an outcome"));
            }
            for (index, child) in children.iter().enumerate() {
                if index < attempt_count && child.status != T1ScreenChildStatus::Completed
                    || index == attempt_count
                        && !matches!(
                            child.status,
                            T1ScreenChildStatus::Pending
                                | T1ScreenChildStatus::Running
                                | T1ScreenChildStatus::Paused
                        )
                    || index > attempt_count && child.status != T1ScreenChildStatus::Pending
                {
                    return Err(t1_drift("adjacent thinking-level progression"));
                }
            }
        } else {
            validate_terminal_t1_children(model, &children)?;
        }
    }
    if state.status == T1ScreenRunStatus::AwaitingOwner
        && state.models.iter().any(|model| model.outcome.is_none())
    {
        return Err(t1_drift("awaiting-owner terminal state"));
    }
    Ok(())
}

fn validate_terminal_t1_children(
    model: &crate::model::T1ScreenModelState,
    children: &[&T1ScreenChildRun],
) -> Result<(), SkillEvalError> {
    match &model.outcome {
        Some(T1ScreenModelOutcome::Selected { .. }) => {
            if model.attempts.len() != children.len()
                || children
                    .iter()
                    .any(|child| child.status != T1ScreenChildStatus::Completed)
            {
                return Err(t1_drift("selected model child statuses"));
            }
        }
        Some(T1ScreenModelOutcome::Exhausted) => {
            for (index, child) in children.iter().enumerate() {
                let expected = if index + 1 == children.len() {
                    T1ScreenChildStatus::Exhausted
                } else {
                    T1ScreenChildStatus::Completed
                };
                if child.status != expected {
                    return Err(t1_drift("exhausted model child statuses"));
                }
            }
        }
        Some(T1ScreenModelOutcome::InfrastructureFailed {
            model,
            child_run_id,
        }) => {
            let failed = children
                .iter()
                .position(|child| child.run_id == *child_run_id && child.model == *model)
                .ok_or_else(|| t1_drift("infrastructure-failed model child identity"))?;
            for (index, child) in children.iter().enumerate() {
                let expected = match index.cmp(&failed) {
                    std::cmp::Ordering::Less => T1ScreenChildStatus::Completed,
                    std::cmp::Ordering::Equal => T1ScreenChildStatus::Failed,
                    std::cmp::Ordering::Greater => T1ScreenChildStatus::Skipped,
                };
                if child.status != expected {
                    return Err(t1_drift("infrastructure-failed model child statuses"));
                }
            }
        }
        None => unreachable!(),
    }
    Ok(())
}

fn next_t1_child_index(state: &T1ScreenRunState) -> Result<Option<usize>, SkillEvalError> {
    validate_t1_progression(state)?;
    let Some((model_index, model)) = state
        .models
        .iter()
        .enumerate()
        .find(|(_, model)| model.outcome.is_none())
    else {
        return Ok(None);
    };
    let thinking_index = model.attempts.len();
    state
        .child_runs
        .iter()
        .position(|child| {
            child.model_index == u64::try_from(model_index).unwrap_or(u64::MAX)
                && child.thinking_index == u64::try_from(thinking_index).unwrap_or(u64::MAX)
        })
        .map(Some)
        .ok_or_else(|| t1_drift("next preallocated child"))
}

fn run_t1_child(
    state: &mut T1ScreenRunState,
    child_index: usize,
    is_resuming: bool,
    runtime: &mut dyn T1ScreenRuntime,
    progress: &mut dyn T1ScreenProgressSink,
) -> Result<T1ChildResult, SkillEvalError> {
    let child = state.child_runs[child_index].clone();
    let mut replay = match replay_pool_child(&child.run_id, runtime) {
        Ok(replay) => replay,
        Err(SkillEvalError::NotFound(_)) => PoolChildReplay::default(),
        Err(error) => return Err(error),
    };
    validate_t1_child_replay(state, &child, &replay)?;
    let mut child_progress = SilentProgress;
    if replay.configuration.is_none() {
        let configuration = t1_child_configuration(state, &child, runtime.now());
        append_t1_child_event(
            runtime,
            &mut child_progress,
            &child.run_id,
            RunEvent::RunStarted {
                at: configuration.created_at.clone(),
                configuration: configuration.clone(),
            },
        )?;
        replay.configuration = Some(configuration);
    } else if is_resuming {
        append_t1_child_event(
            runtime,
            &mut child_progress,
            &child.run_id,
            RunEvent::RunResumed { at: runtime.now() },
        )?;
    }
    if replay.discovery.is_none() {
        let discovery = t1_discovery(&state.configuration.exam);
        append_t1_child_event(
            runtime,
            &mut child_progress,
            &child.run_id,
            RunEvent::DiscoveryCompleted {
                at: runtime.now(),
                artifacts: vec![discovery.clone()],
            },
        )?;
        replay.discovery = Some(vec![discovery]);
    }

    let cases = state.configuration.exam.cases.clone();
    for (case_index, case) in cases.iter().enumerate() {
        let key = TrialKey {
            artifact: state.configuration.exam.name.clone(),
            tier: Tier::T1,
            route_index: 0,
            case: case.id.clone(),
            attempt: 1,
        };
        if replay.completed.contains_key(&key) {
            continue;
        }
        let harness = state
            .configuration
            .candidate_environment
            .harnesses
            .get(case_index)
            .ok_or_else(|| t1_drift("candidate environment identity"))?
            .clone();
        if !replay.started.contains_key(&key) {
            append_t1_child_event(
                runtime,
                &mut child_progress,
                &child.run_id,
                RunEvent::TrialStarted {
                    at: runtime.now(),
                    key: key.clone(),
                    models: vec![child.model.clone()],
                    harness: harness.clone(),
                },
            )?;
        }
        let candidate = if let Some(candidate) = replay.candidates.get(&key) {
            candidate.clone()
        } else {
            let candidate = match runtime.execute(
                &child.run_id,
                &key,
                &state.configuration.exam,
                case,
                &child.model,
                &harness,
                state.configuration.policy.candidate_timeout_seconds,
            ) {
                Ok(candidate) => candidate,
                Err(error) if is_t1_boundary_error(&error) => {
                    return pause_t1_child(runtime, &child.run_id, error);
                }
                Err(error) => return Err(error),
            };
            if candidate.key != key
                || candidate.model != child.model
                || candidate.harness != harness
            {
                return Err(t1_drift("requested and effective candidate identity"));
            }
            append_t1_child_event(
                runtime,
                &mut child_progress,
                &child.run_id,
                RunEvent::CandidateExecuted {
                    at: runtime.now(),
                    candidate: candidate.clone(),
                },
            )?;
            if candidate.usage.cost_millionths_of_dollar != 0 {
                return Err(t1_invalid("candidate cost must be exactly zero"));
            }
            sync_t1_usage(state, runtime, progress)?;
            candidate
        };
        if candidate.usage.cost_millionths_of_dollar != 0 {
            return Err(t1_invalid("candidate cost must be exactly zero"));
        }
        let checks = match runtime.verify(case, &candidate) {
            Ok(checks) => checks,
            Err(error) if is_t1_boundary_error(&error) => {
                return pause_t1_child(runtime, &child.run_id, error);
            }
            Err(error) => return Err(error),
        };
        let input = JudgeInput {
            candidate: candidate.clone(),
            expect: case.expect.clone(),
            rubric_path: state.configuration.exam.root.join("evals/rubric.md"),
            checks,
        };
        let upper_bound = runtime.judge_cost_upper_bound(&state.configuration.judge, &input)?;
        let projected_spend = state
            .spent_judge_millionths_of_dollar
            .checked_add(upper_bound)
            .ok_or_else(|| t1_invalid("judge spending arithmetic overflow"))?;
        let cap = t1_judge_cap(state);
        let campaign_remaining = t1_campaign_remaining(runtime, state)?;
        if projected_spend > cap || upper_bound > campaign_remaining {
            append_t1_child_event(
                runtime,
                &mut child_progress,
                &child.run_id,
                RunEvent::RunPaused {
                    at: runtime.now(),
                    reason: PauseReason::Infrastructure {
                        message: "T1 screening judge cap reached".to_owned(),
                    },
                },
            )?;
            return Ok(T1ChildResult::Paused(t1_judge_cap_pause(state)));
        }
        let judged = match runtime.grade(&state.configuration.judge, &input) {
            Ok(judged) => judged,
            Err(error) if is_t1_boundary_error(&error) => {
                return pause_t1_child(runtime, &child.run_id, error);
            }
            Err(error) => return Err(error),
        };
        if judged.model != state.configuration.judge {
            return Err(t1_drift("judge identity"));
        }
        if judged.usage.cost_millionths_of_dollar > upper_bound
            || state
                .spent_judge_millionths_of_dollar
                .checked_add(judged.usage.cost_millionths_of_dollar)
                .is_none_or(|spent| spent > cap)
        {
            return Err(t1_invalid(
                "judge usage exceeded its preflight upper bound or frozen cap",
            ));
        }
        let record = TrialRecord {
            key: key.clone(),
            model: candidate.model.clone(),
            harness: candidate.harness.clone(),
            artifact_path: candidate.artifact_path.clone(),
            transcript_path: candidate.transcript_path.clone(),
            candidate_usage: candidate.usage.clone(),
            judge_model: judged.model,
            judge_usage: judged.usage,
            verdict: judged.verdict,
        };
        append_t1_child_event(
            runtime,
            &mut child_progress,
            &child.run_id,
            RunEvent::TrialCompleted {
                at: runtime.now(),
                record,
            },
        )?;
        sync_t1_usage(state, runtime, progress)?;
    }

    if !replay
        .completed_artifacts
        .contains_key(&state.configuration.exam.name)
    {
        append_t1_child_event(
            runtime,
            &mut child_progress,
            &child.run_id,
            RunEvent::PoolChildCompleted {
                at: runtime.now(),
                artifact: state.configuration.exam.name.clone(),
                tier: Tier::T1,
            },
        )?;
    }
    Ok(T1ChildResult::Completed)
}

fn validate_t1_child_replay(
    state: &T1ScreenRunState,
    child: &T1ScreenChildRun,
    replay: &PoolChildReplay,
) -> Result<(), SkillEvalError> {
    if let Some(configuration) = &replay.configuration {
        let expected = t1_child_configuration(state, child, configuration.created_at.clone());
        if configuration != &expected {
            return Err(t1_drift("child frozen configuration"));
        }
    } else if replay.discovery.is_some()
        || !replay.started.is_empty()
        || !replay.candidates.is_empty()
        || !replay.completed.is_empty()
        || !replay.completed_artifacts.is_empty()
    {
        return Err(t1_drift("child run start checkpoint"));
    }
    let discovery = vec![t1_discovery(&state.configuration.exam)];
    if replay
        .discovery
        .as_ref()
        .is_some_and(|stored| stored != &discovery)
    {
        return Err(t1_drift("child discovery checkpoint"));
    }
    let planned = state
        .configuration
        .exam
        .cases
        .iter()
        .zip(&state.configuration.candidate_environment.harnesses)
        .map(|(case, harness)| {
            (
                TrialKey {
                    artifact: state.configuration.exam.name.clone(),
                    tier: Tier::T1,
                    route_index: 0,
                    case: case.id.clone(),
                    attempt: 1,
                },
                harness,
            )
        })
        .collect::<BTreeMap<_, _>>();
    if replay
        .started
        .keys()
        .chain(replay.candidates.keys())
        .chain(replay.completed.keys())
        .any(|key| !planned.contains_key(key))
    {
        return Err(t1_drift("five-case child plan"));
    }
    for (key, started) in &replay.started {
        if started.models != [child.model.clone()]
            || planned
                .get(key)
                .is_none_or(|harness| **harness != started.harness)
        {
            return Err(t1_drift("fallback-free child identity"));
        }
    }
    for (key, candidate) in &replay.candidates {
        if candidate.key != *key
            || candidate.model != child.model
            || planned
                .get(key)
                .is_none_or(|harness| **harness != candidate.harness)
            || candidate.usage.cost_millionths_of_dollar != 0
        {
            return Err(t1_drift("candidate checkpoint identity or cost"));
        }
    }
    for (key, record) in &replay.completed {
        let candidate = replay
            .candidates
            .get(key)
            .ok_or_else(|| t1_drift("candidate checkpoint"))?;
        if !candidate_matches_record(candidate, record)
            || record.judge_model != state.configuration.judge
        {
            return Err(t1_drift("judge completion checkpoint"));
        }
    }
    if replay.completed_artifacts.iter().any(|(artifact, tier)| {
        artifact != &state.configuration.exam.name
            || *tier != Tier::T1
            || replay.completed.len() != planned.len()
    }) {
        return Err(t1_drift("child completion checkpoint"));
    }
    Ok(())
}

fn t1_child_configuration(
    state: &T1ScreenRunState,
    child: &T1ScreenChildRun,
    created_at: crate::model::Timestamp,
) -> RunConfiguration {
    RunConfiguration {
        run_id: child.run_id.clone(),
        mode: RunMode::Execute,
        artifacts: vec![state.configuration.exam.clone()],
        change: None,
        policy: t1_qualification_policy(state),
        qualification_routes: BTreeMap::new(),
        created_at,
    }
}

fn t1_qualification_policy(state: &T1ScreenRunState) -> QualificationPolicy {
    QualificationPolicy {
        purpose: QualificationPurpose::ModelPool,
        candidate_tiers: vec![Tier::T1],
        reference_tier: Tier::T2,
        judge_tier: state.configuration.judge.tier,
        repeats_per_case: state.configuration.policy.repeats_per_case,
        minimum_score: state.configuration.policy.minimum_score,
        noninferiority_margin: 0.0,
        confidence_level: 0.95,
    }
}

fn t1_discovery(artifact: &ArtifactDefinition) -> ArtifactDiscovery {
    ArtifactDiscovery {
        artifact: artifact.name.clone(),
        kind: artifact.kind,
        revision: artifact.revision.clone(),
        cases: artifact
            .cases
            .iter()
            .map(|case| CaseDiscovery {
                id: case.id.clone(),
                drive: case.execution.drive.clone(),
                is_holdout: case.is_holdout,
            })
            .collect(),
    }
}

fn complete_t1_child(
    state: &mut T1ScreenRunState,
    child_index: usize,
    runtime: &mut dyn T1ScreenRuntime,
    progress: &mut dyn T1ScreenProgressSink,
) -> Result<(), SkillEvalError> {
    sync_t1_usage(state, runtime, progress)?;
    let child = state.child_runs[child_index].clone();
    let replay = replay_pool_child(&child.run_id, runtime)?;
    let expected_cases = state
        .configuration
        .exam
        .cases
        .iter()
        .map(|case| case.id.clone())
        .collect::<Vec<_>>();
    if replay.completed_artifacts.len() != 1
        || replay
            .completed_artifacts
            .get(&state.configuration.exam.name)
            != Some(&Tier::T1)
    {
        return Err(t1_drift("complete five-case child evidence"));
    }
    let evidence = evaluate_calibration(
        &child.model,
        &expected_cases,
        &replay.completed.into_values().collect::<Vec<_>>(),
        &t1_pool_policy(state),
    )?;
    if evidence.candidate_usage.cost_millionths_of_dollar != 0
        || evidence.effective_model != evidence.requested_model
        || evidence.judge_model != state.configuration.judge
    {
        return Err(t1_drift("completed child cost or identity"));
    }
    let model_index = usize::try_from(child.model_index)
        .map_err(|_| t1_invalid("model index arithmetic overflow"))?;
    if state.models[model_index].attempts.len()
        != usize::try_from(child.thinking_index)
            .map_err(|_| t1_invalid("thinking index arithmetic overflow"))?
    {
        return Err(t1_drift("append-only adjacent attempt"));
    }
    let is_strongest = state.child_runs.iter().all(|other| {
        other.model_index != child.model_index || other.thinking_index <= child.thinking_index
    });
    state.models[model_index]
        .attempts
        .push(T1ScreenAttemptEvidence {
            child_run_id: child.run_id.clone(),
            evidence,
        });
    if !is_strongest {
        state.child_runs[child_index].status = T1ScreenChildStatus::Completed;
    } else if let Some(selected) = state.models[model_index]
        .attempts
        .iter()
        .find(|attempt| attempt.evidence.is_passing)
        .map(|attempt| attempt.evidence.requested_model.clone())
    {
        state.child_runs[child_index].status = T1ScreenChildStatus::Completed;
        state.models[model_index].outcome =
            Some(T1ScreenModelOutcome::Selected { model: selected });
    } else {
        state.child_runs[child_index].status = T1ScreenChildStatus::Exhausted;
        state.models[model_index].outcome = Some(T1ScreenModelOutcome::Exhausted);
    }
    save_t1_screen_and_emit(runtime, progress, state)
}

fn t1_pool_policy(state: &T1ScreenRunState) -> crate::model::PoolPolicy {
    crate::model::PoolPolicy {
        calibration_repeats_per_case: state.configuration.policy.repeats_per_case,
        qualification_repeats_per_case: 1,
        promotion_count: 2,
        minimum_score: state.configuration.policy.minimum_score,
        calibration_minimum_reliability_basis_points: state
            .configuration
            .policy
            .calibration_minimum_reliability_basis_points,
        qualification_minimum_reliability_basis_points: 10_000,
        maximum_catalog_age_seconds: 1,
        spending_limit_millionths_of_dollar: t1_judge_cap(state),
        is_provider_limit_enforced: true,
    }
}

fn sync_t1_usage(
    state: &mut T1ScreenRunState,
    runtime: &mut dyn T1ScreenRuntime,
    progress: &mut dyn T1ScreenProgressSink,
) -> Result<(), SkillEvalError> {
    let mut candidate = t1_zero_usage();
    let mut judge = t1_zero_usage();
    for child in &state.child_runs {
        let replay = match replay_pool_child(&child.run_id, runtime) {
            Ok(replay) => replay,
            Err(SkillEvalError::NotFound(_)) => continue,
            Err(error) => return Err(error),
        };
        for item in replay.candidates.values() {
            if item.usage.cost_millionths_of_dollar != 0 {
                return Err(t1_invalid("candidate cost must be exactly zero"));
            }
            candidate = add_t1_usage(&candidate, &item.usage)?;
        }
        for item in replay.completed.values() {
            judge = add_t1_usage(&judge, &item.judge_usage)?;
        }
    }
    if !t1_usage_is_nondecreasing(&state.candidate_usage, &candidate)
        || !t1_usage_is_nondecreasing(&state.judge_usage, &judge)
    {
        return Err(t1_drift("aggregate child usage"));
    }
    if state.candidate_usage == candidate && state.judge_usage == judge {
        return Ok(());
    }
    state.candidate_usage = candidate;
    state.spent_judge_millionths_of_dollar = judge.cost_millionths_of_dollar;
    state.judge_usage = judge;
    if state.spent_judge_millionths_of_dollar > t1_judge_cap(state) {
        return Err(t1_invalid("judge spend exceeds a frozen cap"));
    }
    save_t1_screen_and_emit(runtime, progress, state)
}

fn add_t1_usage(left: &TrialUsage, right: &TrialUsage) -> Result<TrialUsage, SkillEvalError> {
    Ok(TrialUsage {
        input_tokens: t1_add(left.input_tokens, right.input_tokens, "input tokens")?,
        output_tokens: t1_add(left.output_tokens, right.output_tokens, "output tokens")?,
        cache_read_tokens: t1_add(
            left.cache_read_tokens,
            right.cache_read_tokens,
            "cache-read tokens",
        )?,
        cache_write_tokens: t1_add(
            left.cache_write_tokens,
            right.cache_write_tokens,
            "cache-write tokens",
        )?,
        turns: left
            .turns
            .checked_add(right.turns)
            .ok_or_else(|| t1_invalid("turns arithmetic overflow"))?,
        tool_calls: left
            .tool_calls
            .checked_add(right.tool_calls)
            .ok_or_else(|| t1_invalid("tool calls arithmetic overflow"))?,
        elapsed_milliseconds: t1_add(
            left.elapsed_milliseconds,
            right.elapsed_milliseconds,
            "elapsed milliseconds",
        )?,
        cost_millionths_of_dollar: t1_add(
            left.cost_millionths_of_dollar,
            right.cost_millionths_of_dollar,
            "cost",
        )?,
    })
}

fn t1_add(left: u64, right: u64, label: &str) -> Result<u64, SkillEvalError> {
    left.checked_add(right)
        .ok_or_else(|| t1_invalid(format!("{label} arithmetic overflow")))
}

fn t1_usage_is_nondecreasing(old: &TrialUsage, new: &TrialUsage) -> bool {
    new.input_tokens >= old.input_tokens
        && new.output_tokens >= old.output_tokens
        && new.cache_read_tokens >= old.cache_read_tokens
        && new.cache_write_tokens >= old.cache_write_tokens
        && new.turns >= old.turns
        && new.tool_calls >= old.tool_calls
        && new.elapsed_milliseconds >= old.elapsed_milliseconds
        && new.cost_millionths_of_dollar >= old.cost_millionths_of_dollar
}

fn t1_zero_usage() -> TrialUsage {
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

fn t1_campaign_remaining(
    runtime: &dyn T1ScreenRuntime,
    state: &T1ScreenRunState,
) -> Result<u64, SkillEvalError> {
    let campaign = runtime.load_t1_screen_campaign(&state.configuration.campaign_id)?;
    if campaign.active_run_id.as_ref() != Some(&state.configuration.run_id) {
        return Err(t1_invalid("campaign active run differs before judge call"));
    }
    campaign
        .approved_judge_total_millionths_of_dollar
        .checked_sub(campaign.aggregate_judge_spent_millionths_of_dollar)
        .ok_or_else(|| t1_invalid("campaign remaining spend underflow"))
}

fn t1_judge_cap(state: &T1ScreenRunState) -> u64 {
    let (owner, provider) = t1_screen_effective_caps(state)
        .expect("validated T1 screening state has an effective cap chain");
    owner.min(provider)
}

fn t1_judge_cap_pause(state: &T1ScreenRunState) -> T1ScreenPauseReason {
    let (owner, provider) = t1_screen_effective_caps(state)
        .expect("validated T1 screening state has an effective cap chain");
    T1ScreenPauseReason::JudgeCap {
        spent_millionths_of_dollar: state.spent_judge_millionths_of_dollar,
        owner_approved_millionths_of_dollar: owner,
        provider_enforced_millionths_of_dollar: provider,
    }
}

fn pause_t1_child<R: T1ScreenRuntime + ?Sized>(
    runtime: &mut R,
    run_id: &RunId,
    error: SkillEvalError,
) -> Result<T1ChildResult, SkillEvalError> {
    let reason = t1_pause_from_error(error);
    let child_reason = match &reason {
        T1ScreenPauseReason::Quota { model, reset_at } => PauseReason::Quota {
            model: model.clone(),
            reset_at: reset_at.clone(),
        },
        T1ScreenPauseReason::Infrastructure { message } => PauseReason::Infrastructure {
            message: message.clone(),
        },
        T1ScreenPauseReason::JudgeCap { .. } => unreachable!(),
    };
    let mut progress = SilentProgress;
    append_t1_child_event(
        runtime,
        &mut progress,
        run_id,
        RunEvent::RunPaused {
            at: runtime.now(),
            reason: child_reason,
        },
    )?;
    Ok(T1ChildResult::Paused(reason))
}

fn is_t1_boundary_error(error: &SkillEvalError) -> bool {
    !matches!(
        error,
        SkillEvalError::InvalidArguments(_) | SkillEvalError::InvalidConfiguration(_)
    )
}

fn t1_pause_from_error(error: SkillEvalError) -> T1ScreenPauseReason {
    match error {
        SkillEvalError::Quota { model, reset_at } => T1ScreenPauseReason::Quota { model, reset_at },
        error => T1ScreenPauseReason::Infrastructure {
            message: format!("{error:?}"),
        },
    }
}

fn append_t1_child_event<R: T1ScreenRuntime + ?Sized>(
    runtime: &mut R,
    progress: &mut dyn ProgressSink,
    run_id: &RunId,
    event: RunEvent,
) -> Result<(), SkillEvalError> {
    runtime.append(run_id, &event)?;
    progress.emit(&event)
}

fn save_t1_screen_and_emit<R: T1ScreenRuntime + ?Sized>(
    runtime: &mut R,
    progress: &mut dyn T1ScreenProgressSink,
    state: &T1ScreenRunState,
) -> Result<(), SkillEvalError> {
    runtime.save_t1_screen(state)?;
    runtime.reconcile_t1_screen_campaign_run(state)?;
    progress.emit_t1_screen(state)
}

fn t1_invalid(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(format!("T1 screening {}", message.into()))
}

fn t1_drift(label: &str) -> SkillEvalError {
    t1_invalid(format!("resume drift in {label}"))
}

pub(crate) fn t1_environment_difference(
    expected: &[CandidateEnvironmentEntry],
    current: &[CandidateEnvironmentEntry],
) -> Option<String> {
    let mut expected_index = 0;
    let mut current_index = 0;
    while let (Some(frozen), Some(observed)) =
        (expected.get(expected_index), current.get(current_index))
    {
        match frozen.key.cmp(&observed.key) {
            std::cmp::Ordering::Less => {
                return Some(format!("removed {}", frozen.key));
            }
            std::cmp::Ordering::Greater => {
                return Some(format!("added {}", observed.key));
            }
            std::cmp::Ordering::Equal if frozen.sha256 != observed.sha256 => {
                return Some(format!("changed {}", frozen.key));
            }
            std::cmp::Ordering::Equal => {
                expected_index += 1;
                current_index += 1;
            }
        }
    }
    if let Some(frozen) = expected.get(expected_index) {
        return Some(format!("removed {}", frozen.key));
    }
    if let Some(observed) = current.get(current_index) {
        return Some(format!("added {}", observed.key));
    }
    None
}

enum T1ChildResult {
    Completed,
    Paused(T1ScreenPauseReason),
}

pub(crate) fn start_pool_qualification(
    request: PoolQualifyRequest,
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn PoolProgressSink,
) -> Result<PoolRunState, SkillEvalError> {
    let plan = runtime.load_pool_plan(&request.plan_path)?;
    let created_at = runtime.now();
    runtime.validate_pool_plan_freshness(&plan, &created_at)?;
    validate_pool_request(&request, &plan)?;
    let artifacts = load_and_validate_artifacts(&request.artifact_roots, runtime)?;
    validate_single_pool_artifact(&artifacts)?;

    let pool_run_id = runtime.next_pool()?;
    let configuration = PoolRunConfiguration {
        run_id: pool_run_id,
        created_at,
        artifacts,
        entrants: plan.entrants,
        control: plan.control,
        policy: plan.policy,
    };
    let child_runs = preallocate_pool_children(&request.selected_tiers, &configuration, runtime)?;
    let mut state = PoolRunState {
        configuration,
        selected_tiers: request.selected_tiers,
        status: PoolRunStatus::Pending,
        child_runs,
        pools: Vec::new(),
        pause: None,
        spent_millionths_of_dollar: 0,
    };
    runtime.create_pool(&state)?;
    progress.emit_pool(&state)?;

    if request.is_dry_run {
        return Ok(state);
    }

    state.status = PoolRunStatus::Running;
    save_pool_and_emit(runtime, progress, &state)?;
    let child_index = next_pool_child_index(&state)?.ok_or_else(|| {
        SkillEvalError::InvalidConfiguration(
            "pool has no first selected calibration child".to_owned(),
        )
    })?;
    state.child_runs[child_index].status = PoolChildStatus::Running;
    save_pool_and_emit(runtime, progress, &state)?;

    let child = state.child_runs[child_index].clone();
    let requested = requested_pool_child_model(&state, &child)?;
    let candidate_timeout_seconds = pool_child_candidate_timeout(&state, &child)?;
    let child_request = pool_child_request(
        &state
            .configuration
            .artifacts
            .iter()
            .map(|artifact| artifact.root.clone())
            .collect::<Vec<_>>(),
        child.tier,
        state.configuration.policy.calibration_repeats_per_case,
        state.configuration.policy.minimum_score,
        runtime.configured_judge_tier()?,
    );
    let mut child_progress = SilentProgress;
    match start_pool_qualification_with_run_id(
        child.run_id,
        requested,
        candidate_timeout_seconds,
        child_request,
        runtime,
        &mut child_progress,
    ) {
        Ok(report) => {
            let candidate_cost = candidate_cost(&report.run_id, runtime)?;
            if candidate_cost > 0 {
                state.spent_millionths_of_dollar = state
                    .spent_millionths_of_dollar
                    .checked_add(candidate_cost)
                    .ok_or_else(|| {
                        SkillEvalError::InvalidConfiguration(
                            "pool spending arithmetic overflow".to_owned(),
                        )
                    })?;
                save_pool_and_emit(runtime, progress, &state)?;
            }
            match report.status {
                RunStatus::Completed => {
                    state.child_runs[child_index].status = PoolChildStatus::Completed;
                    save_pool_and_emit(runtime, progress, &state)?;
                    persist_completed_pool_child_evidence(
                        &mut state,
                        child_index,
                        runtime,
                        progress,
                    )?;
                }
                RunStatus::Paused => {
                    let reason = report.pause.ok_or_else(|| {
                        SkillEvalError::InvalidConfiguration(
                            "paused pool child has no pause reason".to_owned(),
                        )
                    })?;
                    pause_pool_child(&mut state, child_index, reason, runtime, progress)?;
                }
                status => {
                    return Err(SkillEvalError::InvalidConfiguration(format!(
                        "pool child returned nonterminal status {status:?}"
                    )));
                }
            }
            Ok(state)
        }
        Err(
            error @ (SkillEvalError::InvalidArguments(_)
            | SkillEvalError::InvalidConfiguration(_)
            | SkillEvalError::JudgeUnavailable { .. }),
        ) => Err(error),
        Err(error) => {
            let reason = pause_reason_from_error(error);
            pause_pool_child(&mut state, child_index, reason, runtime, progress)?;
            Ok(state)
        }
    }
}

fn validate_pool_request(
    request: &PoolQualifyRequest,
    plan: &crate::model::PoolPlan,
) -> Result<(), SkillEvalError> {
    if request.artifact_roots.is_empty() || request.selected_tiers.is_empty() {
        return Err(SkillEvalError::InvalidArguments(
            "pool qualification requires an exam and at least one selected tier".to_owned(),
        ));
    }
    let selected = request
        .selected_tiers
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if selected.len() != request.selected_tiers.len() {
        return Err(SkillEvalError::InvalidConfiguration(
            "pool qualification contains duplicate selected tiers".to_owned(),
        ));
    }
    if selected.iter().any(|tier| {
        plan.entrants
            .get(tier)
            .is_none_or(|entrants| entrants.len() < 3)
    }) {
        return Err(SkillEvalError::InvalidConfiguration(
            "each selected pool tier must contain at least three entrants".to_owned(),
        ));
    }
    if plan.policy.spending_limit_millionths_of_dollar == 0
        || !plan.policy.is_provider_limit_enforced
    {
        return Err(SkillEvalError::InvalidConfiguration(
            "pool launch requires a positive provider-enforced spending limit".to_owned(),
        ));
    }
    Ok(())
}

fn load_and_validate_artifacts(
    roots: &[std::path::PathBuf],
    runtime: &dyn QualificationRuntime,
) -> Result<Vec<ArtifactDefinition>, SkillEvalError> {
    let artifacts = roots
        .iter()
        .map(|root| runtime.load(root))
        .collect::<Result<Vec<_>, _>>()?;
    let mut names = BTreeSet::new();
    for artifact in &artifacts {
        validate_artifact(artifact)?;
        if !artifact.cases.iter().any(|case| !case.is_holdout) {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "pool exam artifact {:?} has no non-holdout case",
                artifact.name.0
            )));
        }
        if !names.insert(artifact.name.clone()) {
            return Err(SkillEvalError::InvalidConfiguration(
                "pool exam contains a duplicate artifact".to_owned(),
            ));
        }
    }
    Ok(artifacts)
}

fn validate_single_pool_artifact(artifacts: &[ArtifactDefinition]) -> Result<(), SkillEvalError> {
    if artifacts.len() != 1 {
        return Err(SkillEvalError::InvalidConfiguration(
            "pool qualification requires exactly one frozen calibration artifact".to_owned(),
        ));
    }
    Ok(())
}

fn preallocate_pool_children(
    selected_tiers: &[Tier],
    configuration: &PoolRunConfiguration,
    runtime: &mut dyn PoolRuntime,
) -> Result<Vec<PoolChildRun>, SkillEvalError> {
    let slot_count = selected_tiers.iter().try_fold(0_usize, |count, tier| {
        let entrants = configuration.entrants.get(tier).ok_or_else(|| {
            SkillEvalError::InvalidConfiguration(
                "pool selected tier is absent from its configuration".to_owned(),
            )
        })?;
        entrants.iter().try_fold(count, |count, entrant| {
            count
                .checked_add(entrant.thinking_levels.len().saturating_mul(2))
                .ok_or_else(|| {
                    SkillEvalError::InvalidConfiguration(
                        "pool child preallocation count overflow".to_owned(),
                    )
                })
        })
    })?;
    let mut child_runs = Vec::with_capacity(slot_count);
    let mut run_ids = BTreeSet::new();
    for tier in selected_tiers {
        let entrants = &configuration.entrants[tier];
        for (entrant_index, entrant) in entrants.iter().enumerate() {
            let entrant_index = u8::try_from(entrant_index).map_err(|_| {
                SkillEvalError::InvalidConfiguration(
                    "pool child entrant index is out of range".to_owned(),
                )
            })?;
            for thinking_index in 0..entrant.thinking_levels.len() {
                let thinking_index = u8::try_from(thinking_index).map_err(|_| {
                    SkillEvalError::InvalidConfiguration(
                        "pool child thinking index is out of range".to_owned(),
                    )
                })?;
                for stage in [PoolStage::Calibration, PoolStage::Qualification] {
                    let run_id = runtime.next()?;
                    validate_run_id(&run_id)?;
                    if !run_ids.insert(run_id.clone()) {
                        return Err(SkillEvalError::InvalidConfiguration(
                            "pool child run identifiers must be unique".to_owned(),
                        ));
                    }
                    child_runs.push(PoolChildRun {
                        tier: *tier,
                        entrant_index,
                        thinking_index,
                        stage,
                        run_id,
                        status: PoolChildStatus::Pending,
                    });
                }
            }
        }
    }
    Ok(child_runs)
}

fn requested_pool_child_model(
    state: &PoolRunState,
    child: &PoolChildRun,
) -> Result<ModelIdentity, SkillEvalError> {
    let entrant = state
        .configuration
        .entrants
        .get(&child.tier)
        .and_then(|entrants| entrants.get(usize::from(child.entrant_index)))
        .ok_or_else(|| resume_drift("pool child entrant index"))?;
    let thinking = entrant
        .thinking_levels
        .get(usize::from(child.thinking_index))
        .ok_or_else(|| resume_drift("pool child thinking index"))?;
    let mut requested = entrant.model.clone();
    requested.thinking.clone_from(thinking);
    Ok(requested)
}

fn pool_child_candidate_timeout(
    state: &PoolRunState,
    child: &PoolChildRun,
) -> Result<Option<u32>, SkillEvalError> {
    state
        .configuration
        .entrants
        .get(&child.tier)
        .and_then(|entrants| entrants.get(usize::from(child.entrant_index)))
        .map(|entrant| entrant.candidate_timeout_seconds)
        .ok_or_else(|| resume_drift("pool child entrant index"))
}

fn pool_child_request(
    artifact_roots: &[std::path::PathBuf],
    candidate_tier: Tier,
    repeats_per_case: u16,
    minimum_score: u8,
    judge_tier: Tier,
) -> QualifyRequest {
    QualifyRequest {
        artifact_roots: artifact_roots.to_vec(),
        change: None,
        policy: QualificationPolicy {
            purpose: QualificationPurpose::ModelPool,
            candidate_tiers: vec![candidate_tier],
            reference_tier: pool_reference_tier(candidate_tier),
            judge_tier,
            repeats_per_case,
            minimum_score,
            noninferiority_margin: 0.0,
            confidence_level: 0.95,
        },
        is_dry_run: false,
    }
}

fn pool_reference_tier(candidate_tier: Tier) -> Tier {
    match candidate_tier {
        Tier::T1 => Tier::T2,
        Tier::T2 | Tier::T3 | Tier::T4 | Tier::T5 => Tier::T1,
    }
}

fn save_pool_and_emit(
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn PoolProgressSink,
    state: &PoolRunState,
) -> Result<(), SkillEvalError> {
    runtime.save_pool(state)?;
    progress.emit_pool(state)
}

fn pause_pool_child(
    state: &mut PoolRunState,
    child_index: usize,
    reason: PauseReason,
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn PoolProgressSink,
) -> Result<(), SkillEvalError> {
    state.child_runs[child_index].status = PoolChildStatus::Paused;
    save_pool_and_emit(runtime, progress, state)?;
    state.status = PoolRunStatus::Paused;
    state.pause = Some(match reason {
        PauseReason::Quota { model, reset_at } => PoolPauseReason::Quota { model, reset_at },
        PauseReason::Infrastructure { message } => PoolPauseReason::Infrastructure { message },
    });
    save_pool_and_emit(runtime, progress, state)
}

fn pause_reason_from_error(error: SkillEvalError) -> PauseReason {
    match error {
        SkillEvalError::Quota { model, reset_at } => PauseReason::Quota { model, reset_at },
        error => PauseReason::Infrastructure {
            message: format!("{error:?}"),
        },
    }
}

struct SilentProgress;

impl ProgressSink for SilentProgress {
    fn emit(&mut self, _event: &RunEvent) -> Result<(), SkillEvalError> {
        Ok(())
    }
}

pub(crate) fn start_pool_replacement_qualification(
    parent_run_id: &PoolRunId,
    entrant_index: u8,
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    let state = runtime.load_pool(parent_run_id)?;
    validate_frozen_pool_artifacts(&state, runtime)?;
    validate_single_pool_artifact(&state.configuration.artifacts)?;
    validate_frozen_pool_plan(&state)?;
    if state.status != PoolRunStatus::AwaitingDecision {
        return Err(SkillEvalError::InvalidConfiguration(
            "pool replacement requires an awaiting-decision parent".to_owned(),
        ));
    }
    let tier = state
        .selected_tiers
        .first()
        .copied()
        .ok_or_else(|| resume_drift("replacement selected tier"))?;
    if state.selected_tiers.len() != 1 {
        return Err(SkillEvalError::InvalidConfiguration(
            "pool replacement requires one selected tier".to_owned(),
        ));
    }
    let entrant = state.configuration.entrants[&tier]
        .get(usize::from(entrant_index))
        .ok_or_else(|| {
            SkillEvalError::InvalidArguments("replacement entrant index is out of range".to_owned())
        })?;
    let pool = state
        .pools
        .iter()
        .find(|pool| pool.tier == tier)
        .ok_or_else(|| resume_drift("replacement ranked pool"))?;
    let replacement = pool
        .thinking_selections
        .iter()
        .find(|model| is_same_base_model(model, &entrant.model))
        .cloned()
        .ok_or_else(|| {
            SkillEvalError::InvalidConfiguration(
                "replacement entrant has no completed thinking selection".to_owned(),
            )
        })?;
    if pool.promoted.contains(&replacement)
        || !pool
            .calibration
            .iter()
            .any(|evidence| evidence.requested_model == replacement && evidence.is_passing)
        || pool
            .qualification
            .iter()
            .any(|evidence| evidence.requested_model == replacement)
        || !pool.qualification.iter().any(|evidence| {
            pool.promoted.contains(&evidence.requested_model) && !evidence.is_passing
        })
    {
        return Err(SkillEvalError::InvalidConfiguration(
            "replacement requires one passing unpromoted calibration entrant and one failed finalist"
                .to_owned(),
        ));
    }
    let thinking_index = entrant
        .thinking_levels
        .iter()
        .position(|thinking| thinking == &replacement.thinking)
        .and_then(|index| u8::try_from(index).ok())
        .ok_or_else(|| resume_drift("replacement thinking index"))?;
    let is_skipped = state.child_runs.iter().any(|child| {
        child.tier == tier
            && child.entrant_index == entrant_index
            && child.thinking_index == thinking_index
            && child.stage == PoolStage::Qualification
            && child.status == PoolChildStatus::Skipped
    });
    if !is_skipped {
        return Err(SkillEvalError::InvalidConfiguration(
            "replacement qualification child is not a terminal skip".to_owned(),
        ));
    }

    let roots = state
        .configuration
        .artifacts
        .iter()
        .map(|artifact| artifact.root.clone())
        .collect::<Vec<_>>();
    let request = pool_child_request(
        &roots,
        tier,
        state.configuration.policy.qualification_repeats_per_case,
        state.configuration.policy.minimum_score,
        runtime.configured_judge_tier()?,
    );
    let candidate_timeout_seconds = entrant.candidate_timeout_seconds;
    let run_id = runtime.next()?;
    start_pool_qualification_with_run_id(
        run_id,
        replacement,
        candidate_timeout_seconds,
        request,
        runtime,
        progress,
    )
}

pub(crate) fn resume_pool_qualification(
    run_id: &PoolRunId,
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn PoolProgressSink,
) -> Result<PoolRunState, SkillEvalError> {
    let mut state = runtime.load_pool(run_id)?;
    validate_frozen_pool_artifacts(&state, runtime)?;
    validate_single_pool_artifact(&state.configuration.artifacts)?;
    let prior_status = state.status;
    recover_completed_pool_child_evidence(&mut state, runtime, progress)?;
    if matches!(prior_status, PoolRunStatus::Running | PoolRunStatus::Paused)
        && !matches!(state.status, PoolRunStatus::Running | PoolRunStatus::Paused)
    {
        return Ok(state);
    }
    let Some(child_index) = validate_pool_resume_state(run_id, &state)? else {
        recover_completed_thinking_decisions(&mut state, runtime, progress)?;
        return Ok(state);
    };
    if state.spent_millionths_of_dollar
        >= state
            .configuration
            .policy
            .spending_limit_millionths_of_dollar
    {
        pause_pool_for_spending_limit(&mut state, runtime, progress)?;
        return Ok(state);
    }
    let child = state.child_runs[child_index].clone();
    let requested = requested_pool_child_model(&state, &child)?;
    let repeats_per_case = match child.stage {
        PoolStage::Calibration => state.configuration.policy.calibration_repeats_per_case,
        PoolStage::Qualification => state.configuration.policy.qualification_repeats_per_case,
    };
    let roots = state
        .configuration
        .artifacts
        .iter()
        .map(|artifact| artifact.root.clone())
        .collect::<Vec<_>>();

    if child.status == PoolChildStatus::Pending
        && child.stage == PoolStage::Qualification
        && !is_promoted_pool_child(&state, &child, &requested)
    {
        return Ok(state);
    }

    let prior_report = match child.status {
        PoolChildStatus::Pending => None,
        PoolChildStatus::Running | PoolChildStatus::Paused => {
            Some(build_report(&child.run_id, runtime)?)
        }
        PoolChildStatus::Completed | PoolChildStatus::Skipped => unreachable!(),
        PoolChildStatus::Failed => {
            return Err(resume_drift("failed pool child"));
        }
    };
    let prior_cost = prior_report
        .as_ref()
        .map(|report| candidate_cost(&report.run_id, runtime))
        .transpose()?
        .unwrap_or(0);

    let mut replay = None;
    let mut exact_candidate = None;
    let mut pool_judge = None;
    let pending_judge_tier = if let Some(report) = prior_report.as_ref() {
        let current_replay = replay_pool_child(&child.run_id, runtime)?;
        let (candidate, judge) = validate_pool_child_resume(
            &state,
            &child,
            &requested,
            report,
            &current_replay,
            runtime,
        )?;
        replay = Some(current_replay);
        exact_candidate = Some(candidate);
        pool_judge = Some(judge);
        None
    } else {
        Some(validate_pending_pool_child(
            &state, &child, &requested, runtime,
        )?)
    };

    if state.status == PoolRunStatus::Paused {
        state.status = PoolRunStatus::Running;
        state.pause = None;
        save_pool_and_emit(runtime, progress, &state)?;
    }
    if matches!(
        child.status,
        PoolChildStatus::Pending | PoolChildStatus::Paused
    ) {
        state.child_runs[child_index].status = PoolChildStatus::Running;
        save_pool_and_emit(runtime, progress, &state)?;
    }

    let candidate_timeout_seconds = pool_child_candidate_timeout(&state, &child)?;
    let mut child_progress = SilentProgress;
    let result = match prior_report {
        None => {
            let child_request = pool_child_request(
                &roots,
                child.tier,
                repeats_per_case,
                state.configuration.policy.minimum_score,
                pending_judge_tier.expect("pending child judge tier is present"),
            );
            start_pool_qualification_with_run_id(
                child.run_id.clone(),
                requested,
                candidate_timeout_seconds,
                child_request,
                runtime,
                &mut child_progress,
            )
        }
        Some(report) if report.status == RunStatus::Completed => Ok(report),
        Some(report) if matches!(report.status, RunStatus::Running | RunStatus::Paused) => {
            resume_exact_pool_child(
                &child.run_id,
                report.status == RunStatus::Paused,
                replay.as_ref().expect("existing child replay is present"),
                exact_candidate
                    .as_ref()
                    .expect("existing child exact candidate is present"),
                pool_judge
                    .as_ref()
                    .expect("existing child pool judge is present"),
                candidate_timeout_seconds,
                runtime,
                &mut child_progress,
            )
        }
        Some(report) if report.status == RunStatus::Failed => Ok(report),
        Some(report) => Err(SkillEvalError::InvalidConfiguration(format!(
            "pool child has invalid resumable status {:?}",
            report.status
        ))),
    };

    let report = match result {
        Ok(report) => report,
        Err(
            error @ (SkillEvalError::InvalidArguments(_)
            | SkillEvalError::InvalidConfiguration(_)
            | SkillEvalError::JudgeUnavailable { .. }),
        ) => return Err(error),
        Err(error) => {
            let reason = pause_reason_from_error(error);
            pause_pool_child(&mut state, child_index, reason, runtime, progress)?;
            return Ok(state);
        }
    };

    add_pool_resume_spending(&mut state, prior_cost, &report, runtime, progress)?;
    match report.status {
        RunStatus::Completed => {
            state.child_runs[child_index].status = PoolChildStatus::Completed;
            save_pool_and_emit(runtime, progress, &state)?;
            persist_completed_pool_child_evidence(&mut state, child_index, runtime, progress)?;
        }
        RunStatus::Paused => {
            let reason = report.pause.ok_or_else(|| {
                SkillEvalError::InvalidConfiguration(
                    "paused pool child has no pause reason".to_owned(),
                )
            })?;
            pause_pool_child(&mut state, child_index, reason, runtime, progress)?;
        }
        RunStatus::Failed => {
            review_failed_pool_child(&mut state, child_index, runtime, progress)?;
        }
        status => {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "pool child returned malformed status {status:?}"
            )));
        }
    }
    Ok(state)
}

fn recover_completed_pool_child_evidence(
    state: &mut PoolRunState,
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn PoolProgressSink,
) -> Result<(), SkillEvalError> {
    for child_index in 0..state.child_runs.len() {
        let child = state.child_runs[child_index].clone();
        if child.status != PoolChildStatus::Completed {
            continue;
        }
        let requested = requested_pool_child_model(state, &child)?;
        let is_persisted = state
            .pools
            .iter()
            .find(|pool| pool.tier == child.tier)
            .is_some_and(|pool| {
                let evidence = match child.stage {
                    PoolStage::Calibration => &pool.calibration,
                    PoolStage::Qualification => &pool.qualification,
                };
                evidence
                    .iter()
                    .any(|item| item.requested_model == requested)
            });
        if !is_persisted {
            let replay = replay_pool_child(&child.run_id, runtime)?;
            if replay.completed_artifacts.is_empty() {
                continue;
            }
            persist_completed_pool_child_evidence(state, child_index, runtime, progress)?;
        }
    }
    Ok(())
}

fn recover_completed_thinking_decisions(
    state: &mut PoolRunState,
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn PoolProgressSink,
) -> Result<(), SkillEvalError> {
    for tier in state.selected_tiers.clone() {
        let Some(pool_index) = state.pools.iter().position(|pool| pool.tier == tier) else {
            continue;
        };
        let entrant_count = state.configuration.entrants[&tier].len();
        for entrant_index in 0..entrant_count {
            let entrant_index = u8::try_from(entrant_index).map_err(|_| {
                SkillEvalError::InvalidConfiguration(
                    "pool entrant index exceeds its persisted range".to_owned(),
                )
            })?;
            let entrant = &state.configuration.entrants[&tier][usize::from(entrant_index)];
            let evidence =
                base_model_calibration_evidence(&state.pools[pool_index], &entrant.model);
            if select_thinking_level(entrant, &evidence)?.is_complete {
                advance_thinking_selection(
                    state,
                    tier,
                    entrant_index,
                    pool_index,
                    runtime,
                    progress,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_pool_resume_state(
    run_id: &PoolRunId,
    state: &PoolRunState,
) -> Result<Option<usize>, SkillEvalError> {
    if state.configuration.run_id != *run_id {
        return Err(resume_drift("pool run identity"));
    }
    if !matches!(state.status, PoolRunStatus::Running | PoolRunStatus::Paused) {
        return Err(SkillEvalError::InvalidConfiguration(
            "pool resume requires a running or paused nonterminal run".to_owned(),
        ));
    }
    if (state.status == PoolRunStatus::Paused) != state.pause.is_some() {
        return Err(resume_drift("pool pause state"));
    }
    if state.selected_tiers.is_empty()
        || state
            .selected_tiers
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != state.selected_tiers.len()
    {
        return Err(resume_drift("selected tier plan"));
    }
    validate_frozen_pool_plan(state)?;
    validate_pool_child_slots(state)?;

    let child_index = next_pool_child_index(state)?;
    let Some(child_index) = child_index else {
        if state.child_runs.iter().any(|child| {
            matches!(
                child.status,
                PoolChildStatus::Running | PoolChildStatus::Paused | PoolChildStatus::Failed
            )
        }) {
            return Err(resume_drift("stable pool child order"));
        }
        return Ok(None);
    };
    let child_status = state.child_runs[child_index].status;
    if state.child_runs.iter().enumerate().any(|(index, child)| {
        index != child_index
            && matches!(
                child.status,
                PoolChildStatus::Running | PoolChildStatus::Paused | PoolChildStatus::Failed
            )
    }) {
        return Err(resume_drift("stable pool child order"));
    }
    match (state.status, child_status) {
        (
            PoolRunStatus::Running,
            PoolChildStatus::Pending | PoolChildStatus::Running | PoolChildStatus::Paused,
        )
        | (PoolRunStatus::Paused, PoolChildStatus::Paused) => Ok(Some(child_index)),
        (PoolRunStatus::Paused, PoolChildStatus::Pending)
            if matches!(state.pause, Some(PoolPauseReason::SpendingLimit { .. })) =>
        {
            Ok(Some(child_index))
        }
        (_, PoolChildStatus::Failed) => Err(resume_drift("failed pool child")),
        _ => Err(resume_drift("parent and child status")),
    }
}

fn validate_pool_child_slots(state: &PoolRunState) -> Result<(), SkillEvalError> {
    let mut slots = BTreeSet::new();
    let mut run_ids = BTreeSet::new();
    for child in &state.child_runs {
        validate_run_id(&child.run_id)?;
        let entrant = state
            .configuration
            .entrants
            .get(&child.tier)
            .and_then(|entrants| entrants.get(usize::from(child.entrant_index)))
            .ok_or_else(|| resume_drift("pool child entrant index"))?;
        if usize::from(child.thinking_index) >= entrant.thinking_levels.len()
            || !state.selected_tiers.contains(&child.tier)
            || !slots.insert((
                child.tier,
                child.entrant_index,
                child.thinking_index,
                child.stage,
            ))
            || !run_ids.insert(child.run_id.clone())
        {
            return Err(resume_drift("pool child identity"));
        }
    }
    for tier in &state.selected_tiers {
        for (entrant_index, entrant) in state.configuration.entrants[tier].iter().enumerate() {
            let entrant_index = u8::try_from(entrant_index)
                .map_err(|_| resume_drift("pool child entrant index"))?;
            for thinking_index in 0..entrant.thinking_levels.len() {
                let thinking_index = u8::try_from(thinking_index)
                    .map_err(|_| resume_drift("pool child thinking index"))?;
                for stage in [PoolStage::Calibration, PoolStage::Qualification] {
                    if !slots.contains(&(*tier, entrant_index, thinking_index, stage)) {
                        return Err(resume_drift("pool child identity"));
                    }
                }
            }
        }
    }
    if slots.len() != state.child_runs.len() {
        return Err(resume_drift("pool child identity"));
    }
    Ok(())
}

fn next_pool_child_index(state: &PoolRunState) -> Result<Option<usize>, SkillEvalError> {
    for tier in &state.selected_tiers {
        let entrants = &state.configuration.entrants[tier];
        let pool = state.pools.iter().find(|pool| pool.tier == *tier);
        for (entrant_index, entrant) in entrants.iter().enumerate() {
            let entrant_index = u8::try_from(entrant_index)
                .map_err(|_| resume_drift("pool child entrant index"))?;
            let evidence = pool.map_or(&[][..], |pool| pool.calibration.as_slice());
            let evidence = evidence
                .iter()
                .filter(|item| {
                    item.requested_model.tier == entrant.model.tier
                        && item.requested_model.provider == entrant.model.provider
                        && item.requested_model.model == entrant.model.model
                })
                .cloned()
                .collect::<Vec<_>>();
            let decision = select_thinking_level(entrant, &evidence)?;
            if let Some(thinking_index) = decision.next_thinking_index {
                let child_index = pool_child_slot(
                    state,
                    *tier,
                    entrant_index,
                    thinking_index,
                    PoolStage::Calibration,
                )?;
                match state.child_runs[child_index].status {
                    PoolChildStatus::Completed => continue,
                    PoolChildStatus::Skipped => {
                        return Err(resume_drift("thinking decision child status"));
                    }
                    PoolChildStatus::Pending
                    | PoolChildStatus::Running
                    | PoolChildStatus::Paused
                    | PoolChildStatus::Failed => return Ok(Some(child_index)),
                }
            }

            let mut is_terminal = true;
            for thinking_index in 0..entrant.thinking_levels.len() {
                let thinking_index = u8::try_from(thinking_index)
                    .map_err(|_| resume_drift("pool child thinking index"))?;
                let index = pool_child_slot(
                    state,
                    *tier,
                    entrant_index,
                    thinking_index,
                    PoolStage::Calibration,
                )?;
                is_terminal &= matches!(
                    state.child_runs[index].status,
                    PoolChildStatus::Completed | PoolChildStatus::Skipped
                );
            }
            let is_selection_persisted = match &decision.selected {
                Some(selected) => {
                    pool.is_some_and(|pool| pool.thinking_selections.contains(selected))
                }
                None => true,
            };
            if !is_terminal || !is_selection_persisted {
                return Ok(None);
            }
        }

        let Some(pool) = pool else {
            return Ok(None);
        };
        for (entrant_index, entrant) in entrants.iter().enumerate() {
            let entrant_index = u8::try_from(entrant_index)
                .map_err(|_| resume_drift("pool child entrant index"))?;
            let calibration = base_model_calibration_evidence(pool, &entrant.model);
            let screening = select_thinking_level(entrant, &calibration)?;
            if !screening.is_complete {
                return Ok(None);
            }
            if qualification_eligible_indices(entrant, &calibration)?.is_empty() {
                continue;
            }
            let qualification = base_model_qualification_evidence(pool, &entrant.model);
            let decision =
                select_qualification_thinking_level(entrant, &calibration, &qualification)?;
            if let Some(thinking_index) = decision.next_thinking_index {
                let child_index = pool_child_slot(
                    state,
                    *tier,
                    entrant_index,
                    thinking_index,
                    PoolStage::Qualification,
                )?;
                match state.child_runs[child_index].status {
                    PoolChildStatus::Pending
                    | PoolChildStatus::Running
                    | PoolChildStatus::Paused
                    | PoolChildStatus::Failed => return Ok(Some(child_index)),
                    PoolChildStatus::Completed => continue,
                    PoolChildStatus::Skipped => {
                        return Err(resume_drift("qualification decision child status"));
                    }
                }
            }
        }
    }
    Err(resume_drift("terminal pool status"))
}

fn pool_child_slot(
    state: &PoolRunState,
    tier: Tier,
    entrant_index: u8,
    thinking_index: u8,
    stage: PoolStage,
) -> Result<usize, SkillEvalError> {
    let mut matches = state.child_runs.iter().enumerate().filter(|(_, child)| {
        child.tier == tier
            && child.entrant_index == entrant_index
            && child.thinking_index == thinking_index
            && child.stage == stage
    });
    let (index, _) = matches
        .next()
        .ok_or_else(|| resume_drift("pool child identity"))?;
    if matches.next().is_some() {
        return Err(resume_drift("pool child identity"));
    }
    Ok(index)
}

fn is_promoted_pool_child(
    state: &PoolRunState,
    child: &PoolChildRun,
    _requested: &ModelIdentity,
) -> bool {
    let Some(pool) = state.pools.iter().find(|pool| pool.tier == child.tier) else {
        return false;
    };
    let entrant = &state.configuration.entrants[&child.tier][usize::from(child.entrant_index)];
    let calibration = base_model_calibration_evidence(pool, &entrant.model);
    let qualification = base_model_qualification_evidence(pool, &entrant.model);
    select_qualification_thinking_level(entrant, &calibration, &qualification)
        .is_ok_and(|decision| decision.next_thinking_index == Some(child.thinking_index))
}

fn validate_frozen_pool_plan(state: &PoolRunState) -> Result<(), SkillEvalError> {
    let policy = &state.configuration.policy;
    if policy.calibration_repeats_per_case == 0
        || policy.qualification_repeats_per_case == 0
        || policy.promotion_count != 2
        || policy.minimum_score > 10
        || policy.calibration_minimum_reliability_basis_points != 8_000
        || policy.qualification_minimum_reliability_basis_points != 10_000
        || policy.maximum_catalog_age_seconds == 0
        || policy.spending_limit_millionths_of_dollar == 0
        || !policy.is_provider_limit_enforced
        || state.configuration.control.tier != Tier::T1
    {
        return Err(resume_drift("pool plan"));
    }

    let mut models = Vec::new();
    for tier in [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5] {
        let entrants = state
            .configuration
            .entrants
            .get(&tier)
            .ok_or_else(|| resume_drift("pool model plan"))?;
        if entrants.len() < 3 {
            return Err(resume_drift("pool model plan"));
        }
        for entrant in entrants {
            if entrant.model.tier != tier
                || entrant.model.provider.trim().is_empty()
                || entrant.model.model.trim().is_empty()
                || entrant.model.thinking.trim().is_empty()
                || entrant.catalog_observed_at.0.trim().is_empty()
                || models.contains(&entrant.model)
                || select_thinking_level(entrant, &[]).is_err()
            {
                return Err(resume_drift("pool model plan"));
            }
            models.push(entrant.model.clone());
        }
    }
    if state.configuration.entrants.len() != 5
        || state.configuration.control.provider.trim().is_empty()
        || state.configuration.control.model.trim().is_empty()
        || state.configuration.control.thinking.trim().is_empty()
        || models.contains(&state.configuration.control)
    {
        return Err(resume_drift("pool model plan"));
    }
    Ok(())
}

fn validate_frozen_pool_artifacts(
    state: &PoolRunState,
    runtime: &dyn QualificationRuntime,
) -> Result<(), SkillEvalError> {
    for frozen in &state.configuration.artifacts {
        let current = runtime.load(&frozen.root)?;
        if current != *frozen {
            return Err(resume_drift("complete artifact definition"));
        }
    }
    Ok(())
}

#[derive(Default)]
struct PoolChildReplay {
    configuration: Option<RunConfiguration>,
    discovery: Option<Vec<ArtifactDiscovery>>,
    started: BTreeMap<TrialKey, StartedTrial>,
    candidates: BTreeMap<TrialKey, crate::model::CandidateArtifact>,
    completed: BTreeMap<TrialKey, TrialRecord>,
    completed_artifacts: BTreeMap<ArtifactName, Tier>,
}

fn replay_pool_child<S: RunStore + ?Sized>(
    run_id: &RunId,
    store: &S,
) -> Result<PoolChildReplay, SkillEvalError> {
    let mut replay = PoolChildReplay::default();
    store.replay(run_id, &mut |event| {
        match event {
            RunEvent::RunStarted { configuration, .. } => {
                if replay.configuration.replace(configuration).is_some() {
                    return Err(resume_drift("child frozen configuration"));
                }
            }
            RunEvent::DiscoveryCompleted { artifacts, .. } => {
                if replay.discovery.replace(artifacts).is_some() {
                    return Err(resume_drift("child discovery checkpoint"));
                }
            }
            RunEvent::TrialStarted {
                key,
                models,
                harness,
                ..
            } => {
                if replay
                    .started
                    .insert(key, StartedTrial { models, harness })
                    .is_some()
                {
                    return Err(resume_drift("child trial checkpoint"));
                }
            }
            RunEvent::CandidateExecuted { candidate, .. } => {
                if replay
                    .candidates
                    .insert(candidate.key.clone(), candidate)
                    .is_some()
                {
                    return Err(resume_drift("child candidate checkpoint"));
                }
            }
            RunEvent::TrialCompleted { record, .. } => {
                if replay
                    .completed
                    .insert(record.key.clone(), record)
                    .is_some()
                {
                    return Err(resume_drift("child trial checkpoint"));
                }
            }
            RunEvent::PoolChildCompleted { artifact, tier, .. } => {
                if replay.completed_artifacts.insert(artifact, tier).is_some() {
                    return Err(resume_drift("child completion checkpoint"));
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(replay)
}

fn validate_pending_pool_child(
    state: &PoolRunState,
    child: &PoolChildRun,
    requested: &ModelIdentity,
    runtime: &dyn QualificationRuntime,
) -> Result<Tier, SkillEvalError> {
    let exact_candidate = runtime.exact_candidate(requested)?;
    if exact_candidate != *requested || exact_candidate.tier != child.tier {
        return Err(resume_drift("exact child model"));
    }
    let judge_tier = runtime.configured_judge_tier()?;
    let judge = runtime.pool_judge(&exact_candidate)?;
    validate_external_judge(&exact_candidate, &judge, judge.tier)?;
    for artifact in &state.configuration.artifacts {
        for case in artifact.cases.iter().filter(|case| !case.is_holdout) {
            runtime.identity(artifact, &case.execution)?;
        }
    }
    Ok(judge_tier)
}

fn validate_pool_child_resume(
    state: &PoolRunState,
    child: &PoolChildRun,
    requested: &ModelIdentity,
    report: &QualificationReport,
    replay: &PoolChildReplay,
    runtime: &dyn QualificationRuntime,
) -> Result<(ModelIdentity, ModelIdentity), SkillEvalError> {
    let configuration = replay
        .configuration
        .as_ref()
        .ok_or_else(|| resume_drift("child frozen configuration"))?;
    let repeats_per_case = match child.stage {
        PoolStage::Calibration => state.configuration.policy.calibration_repeats_per_case,
        PoolStage::Qualification => state.configuration.policy.qualification_repeats_per_case,
    };
    let expected_policy = pool_child_request(
        &state
            .configuration
            .artifacts
            .iter()
            .map(|artifact| artifact.root.clone())
            .collect::<Vec<_>>(),
        child.tier,
        repeats_per_case,
        state.configuration.policy.minimum_score,
        configuration.policy.judge_tier,
    )
    .policy;
    if configuration.run_id != child.run_id
        || configuration.mode != RunMode::Execute
        || configuration.artifacts != state.configuration.artifacts
        || configuration.change.is_some()
        || configuration.policy != expected_policy
        || report.run_id != child.run_id
        || report.mode != RunMode::Execute
        || report.change.is_some()
    {
        return Err(resume_drift("child execution plan"));
    }
    let expected_discovery = configuration
        .artifacts
        .iter()
        .map(|artifact| ArtifactDiscovery {
            artifact: artifact.name.clone(),
            kind: artifact.kind,
            revision: artifact.revision.clone(),
            cases: artifact
                .cases
                .iter()
                .map(|case| CaseDiscovery {
                    id: case.id.clone(),
                    drive: case.execution.drive.clone(),
                    is_holdout: case.is_holdout,
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    if replay
        .discovery
        .as_ref()
        .is_some_and(|discovery| *discovery != expected_discovery)
        || report.discoveries != replay.discovery.clone().unwrap_or_default()
    {
        return Err(resume_drift("child discovery checkpoint"));
    }
    if runtime.configured_judge_tier()? != configuration.policy.judge_tier {
        return Err(resume_drift("child judge plan"));
    }
    let exact_candidate = runtime.exact_candidate(requested)?;
    if exact_candidate != *requested || exact_candidate.tier != child.tier {
        return Err(resume_drift("exact child model"));
    }
    let judge = runtime.pool_judge(&exact_candidate)?;
    validate_external_judge(&exact_candidate, &judge, judge.tier)?;

    let mut planned_keys = BTreeSet::new();
    for artifact in &state.configuration.artifacts {
        for case in artifact.cases.iter().filter(|case| !case.is_holdout) {
            let harness = runtime.identity(artifact, &case.execution)?;
            for attempt in 1..=repeats_per_case {
                let key = TrialKey {
                    artifact: artifact.name.clone(),
                    tier: child.tier,
                    route_index: 0,
                    case: case.id.clone(),
                    attempt,
                };
                planned_keys.insert(key.clone());
                if let Some(started) = replay.started.get(&key)
                    && (started.models != [exact_candidate.clone()] || started.harness != harness)
                {
                    return Err(resume_drift("child model or harness identity"));
                }
            }
        }
    }
    if replay
        .started
        .keys()
        .chain(replay.candidates.keys())
        .chain(replay.completed.keys())
        .any(|key| !planned_keys.contains(key))
    {
        return Err(resume_drift("child trial plan"));
    }
    for (key, candidate) in &replay.candidates {
        let started = replay
            .started
            .get(key)
            .ok_or_else(|| resume_drift("child candidate checkpoint"))?;
        if candidate.key != *key
            || candidate.model != exact_candidate
            || candidate.harness != started.harness
        {
            return Err(resume_drift("child candidate checkpoint"));
        }
    }
    for (key, record) in &replay.completed {
        let candidate = replay
            .candidates
            .get(key)
            .ok_or_else(|| resume_drift("child candidate checkpoint"))?;
        if !candidate_matches_record(candidate, record) || record.judge_model != judge {
            return Err(resume_drift("child judge or trial checkpoint"));
        }
    }
    for (artifact, tier) in &replay.completed_artifacts {
        if *tier != child.tier
            || !state
                .configuration
                .artifacts
                .iter()
                .any(|definition| definition.name == *artifact)
            || planned_keys
                .iter()
                .any(|key| key.artifact == *artifact && !replay.completed.contains_key(key))
        {
            return Err(resume_drift("child completion checkpoint"));
        }
    }
    Ok((exact_candidate, judge))
}

#[expect(
    clippy::too_many_arguments,
    reason = "pool resume needs the frozen candidate, judge, and timeout context"
)]
fn resume_exact_pool_child(
    run_id: &RunId,
    is_paused: bool,
    replay: &PoolChildReplay,
    exact_candidate: &ModelIdentity,
    judge: &ModelIdentity,
    candidate_timeout_seconds: Option<u32>,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    let configuration = replay
        .configuration
        .as_ref()
        .ok_or_else(|| resume_drift("child frozen configuration"))?;
    if is_paused {
        let at = runtime.now();
        append_and_emit(runtime, progress, run_id, RunEvent::RunResumed { at })?;
    }
    if replay.discovery.is_none() {
        append_discovery(runtime, progress, run_id, &configuration.artifacts)?;
    }

    for artifact in &configuration.artifacts {
        if replay.completed_artifacts.contains_key(&artifact.name) {
            continue;
        }
        for case in artifact.cases.iter().filter(|case| !case.is_holdout) {
            let current_harness = runtime.identity(artifact, &case.execution)?;
            for attempt in 1..=configuration.policy.repeats_per_case {
                let key = TrialKey {
                    artifact: artifact.name.clone(),
                    tier: exact_candidate.tier,
                    route_index: 0,
                    case: case.id.clone(),
                    attempt,
                };
                if replay.completed.contains_key(&key) {
                    continue;
                }
                let harness = if let Some(started) = replay.started.get(&key) {
                    started.harness.clone()
                } else {
                    let at = runtime.now();
                    append_and_emit(
                        runtime,
                        progress,
                        run_id,
                        RunEvent::TrialStarted {
                            at,
                            key: key.clone(),
                            models: vec![exact_candidate.clone()],
                            harness: current_harness.clone(),
                        },
                    )?;
                    current_harness.clone()
                };
                let candidate = if let Some(candidate) = replay.candidates.get(&key) {
                    candidate.clone()
                } else {
                    match runtime.execute(
                        run_id,
                        &key,
                        artifact,
                        case,
                        exact_candidate,
                        &harness,
                        candidate_timeout_seconds,
                    ) {
                        Ok(candidate) => {
                            if candidate.key != key
                                || candidate.model != *exact_candidate
                                || candidate.harness != harness
                            {
                                return Err(resume_drift("exact child execution"));
                            }
                            let at = runtime.now();
                            append_and_emit(
                                runtime,
                                progress,
                                run_id,
                                RunEvent::CandidateExecuted {
                                    at,
                                    candidate: candidate.clone(),
                                },
                            )?;
                            candidate
                        }
                        Err(
                            error @ (SkillEvalError::InvalidArguments(_)
                            | SkillEvalError::InvalidConfiguration(_)),
                        ) => return Err(error),
                        Err(error) => {
                            append_pause(runtime, progress, run_id, error)?;
                            return build_report(run_id, runtime);
                        }
                    }
                };
                let checks = match runtime.verify(case, &candidate) {
                    Ok(checks) => checks,
                    Err(
                        error @ (SkillEvalError::InvalidArguments(_)
                        | SkillEvalError::InvalidConfiguration(_)),
                    ) => return Err(error),
                    Err(error) => {
                        append_pause(runtime, progress, run_id, error)?;
                        return build_report(run_id, runtime);
                    }
                };
                let judged = match runtime.grade(
                    judge,
                    &JudgeInput {
                        candidate: candidate.clone(),
                        expect: case.expect.clone(),
                        rubric_path: artifact.root.join("evals/rubric.md"),
                        checks,
                    },
                ) {
                    Ok(judged) => judged,
                    Err(
                        error @ (SkillEvalError::InvalidArguments(_)
                        | SkillEvalError::InvalidConfiguration(_)
                        | SkillEvalError::JudgeUnavailable { .. }),
                    ) => return Err(error),
                    Err(error) => {
                        append_pause(runtime, progress, run_id, error)?;
                        return build_report(run_id, runtime);
                    }
                };
                if judged.model != *judge {
                    return Err(resume_drift("child judge identity"));
                }
                let record = TrialRecord {
                    key,
                    model: candidate.model.clone(),
                    harness: candidate.harness.clone(),
                    artifact_path: candidate.artifact_path.clone(),
                    transcript_path: candidate.transcript_path.clone(),
                    candidate_usage: candidate.usage.clone(),
                    judge_model: judged.model,
                    judge_usage: judged.usage,
                    verdict: judged.verdict,
                };
                let at = runtime.now();
                append_and_emit(
                    runtime,
                    progress,
                    run_id,
                    RunEvent::TrialCompleted { at, record },
                )?;
            }
        }
        let at = runtime.now();
        append_and_emit(
            runtime,
            progress,
            run_id,
            RunEvent::PoolChildCompleted {
                at,
                artifact: artifact.name.clone(),
                tier: exact_candidate.tier,
            },
        )?;
    }
    build_report(run_id, runtime)
}

fn add_pool_resume_spending(
    state: &mut PoolRunState,
    prior_cost: u64,
    report: &QualificationReport,
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn PoolProgressSink,
) -> Result<(), SkillEvalError> {
    let current_cost = candidate_cost(&report.run_id, runtime)?;
    let delta = current_cost.checked_sub(prior_cost).ok_or_else(|| {
        SkillEvalError::InvalidConfiguration("pool child usage decreased across resume".to_owned())
    })?;
    if delta == 0 {
        return Ok(());
    }
    state.spent_millionths_of_dollar = state
        .spent_millionths_of_dollar
        .checked_add(delta)
        .ok_or_else(|| {
            SkillEvalError::InvalidConfiguration("pool spending arithmetic overflow".to_owned())
        })?;
    save_pool_and_emit(runtime, progress, state)
}

fn candidate_cost(run_id: &RunId, store: &dyn RunStore) -> Result<u64, SkillEvalError> {
    replay_pool_child(run_id, store)?
        .candidates
        .values()
        .try_fold(0_u64, |cost, candidate| {
            cost.checked_add(candidate.usage.cost_millionths_of_dollar)
                .ok_or_else(|| {
                    SkillEvalError::InvalidConfiguration(
                        "pool candidate spending arithmetic overflow".to_owned(),
                    )
                })
        })
}

fn pause_pool_for_spending_limit(
    state: &mut PoolRunState,
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn PoolProgressSink,
) -> Result<(), SkillEvalError> {
    let limit = state
        .configuration
        .policy
        .spending_limit_millionths_of_dollar;
    if matches!(state.pause, Some(PoolPauseReason::SpendingLimit { .. })) {
        return Ok(());
    }
    if state.status == PoolRunStatus::Paused {
        state.status = PoolRunStatus::Running;
        state.pause = None;
        save_pool_and_emit(runtime, progress, state)?;
    }
    state.status = PoolRunStatus::Paused;
    state.pause = Some(PoolPauseReason::SpendingLimit {
        spent_millionths_of_dollar: state.spent_millionths_of_dollar,
        limit_millionths_of_dollar: limit,
    });
    save_pool_and_emit(runtime, progress, state)
}

fn persist_completed_pool_child_evidence(
    state: &mut PoolRunState,
    child_index: usize,
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn PoolProgressSink,
) -> Result<(), SkillEvalError> {
    let child = state.child_runs[child_index].clone();
    let requested = requested_pool_child_model(state, &child)?;
    let replay = replay_pool_child(&child.run_id, runtime)?;
    let artifact = state
        .configuration
        .artifacts
        .first()
        .ok_or_else(|| resume_drift("single frozen calibration artifact"))?;
    if replay.completed_artifacts.len() != 1
        || replay.completed_artifacts.get(&artifact.name) != Some(&child.tier)
    {
        return Err(resume_drift("complete child artifact evidence"));
    }
    let expected_cases = artifact
        .cases
        .iter()
        .filter(|case| !case.is_holdout)
        .map(|case| case.id.clone())
        .collect::<Vec<_>>();
    let trials = replay.completed.into_values().collect::<Vec<_>>();
    let evidence = match child.stage {
        PoolStage::Calibration => evaluate_calibration(
            &requested,
            &expected_cases,
            &trials,
            &state.configuration.policy,
        )?,
        PoolStage::Qualification => evaluate_qualification(
            &requested,
            &expected_cases,
            &trials,
            &state.configuration.policy,
        )?,
    };

    let pool_index = match state.pools.iter().position(|pool| pool.tier == child.tier) {
        Some(index) => index,
        None => {
            state.pools.push(crate::model::RankedPool {
                tier: child.tier,
                calibration: Vec::new(),
                thinking_selections: Vec::new(),
                retained_lower_routes: Vec::new(),
                promoted: Vec::new(),
                qualification: Vec::new(),
                ranked: Vec::new(),
                is_complete: false,
            });
            state.pools.len() - 1
        }
    };
    let stage_evidence = match child.stage {
        PoolStage::Calibration => &mut state.pools[pool_index].calibration,
        PoolStage::Qualification => &mut state.pools[pool_index].qualification,
    };
    if stage_evidence
        .iter()
        .any(|item| item.requested_model == requested)
    {
        return Ok(());
    }
    stage_evidence.push(evidence.clone());

    match child.stage {
        PoolStage::Calibration => {
            save_pool_and_emit(runtime, progress, state)?;
            advance_thinking_selection(
                state,
                child.tier,
                child.entrant_index,
                pool_index,
                runtime,
                progress,
            )
        }
        PoolStage::Qualification => {
            let mut ranked = rank_pool(
                child.tier,
                &state.configuration.entrants[&child.tier],
                &state.pools[pool_index].calibration,
                &state.pools[pool_index].qualification,
                &state.configuration.policy,
            )?;
            restore_thinking_evidence(&mut ranked, &state.pools[pool_index]);
            state.pools[pool_index] = ranked;
            save_pool_and_emit(runtime, progress, state)?;
            skip_unpromoted_qualification_children(
                state, child.tier, pool_index, runtime, progress,
            )?;
            if is_selected_pool_walks_complete(state)? {
                state.status = PoolRunStatus::AwaitingDecision;
                save_pool_and_emit(runtime, progress, state)?;
            }
            Ok(())
        }
    }
}

fn advance_thinking_selection(
    state: &mut PoolRunState,
    tier: Tier,
    entrant_index: u8,
    pool_index: usize,
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn PoolProgressSink,
) -> Result<(), SkillEvalError> {
    let entrant = &state.configuration.entrants[&tier][usize::from(entrant_index)];
    let evidence = base_model_calibration_evidence(&state.pools[pool_index], &entrant.model);
    let decision = select_thinking_level(entrant, &evidence)?;
    if !decision.is_complete {
        return Ok(());
    }

    if let Some(selected) = decision.selected
        && !state.pools[pool_index]
            .thinking_selections
            .contains(&selected)
    {
        state.pools[pool_index].thinking_selections.push(selected);
        save_pool_and_emit(runtime, progress, state)?;
    }
    if !is_thinking_decisions_complete(state, tier, pool_index)? {
        return Ok(());
    }
    let mut ranked = rank_pool(
        tier,
        &state.configuration.entrants[&tier],
        &state.pools[pool_index].calibration,
        &[],
        &state.configuration.policy,
    )?;
    restore_thinking_evidence(&mut ranked, &state.pools[pool_index]);
    state.pools[pool_index] = ranked;
    save_pool_and_emit(runtime, progress, state)?;
    skip_unpromoted_qualification_children(state, tier, pool_index, runtime, progress)?;
    if is_selected_pool_walks_complete(state)? {
        state.status = PoolRunStatus::AwaitingDecision;
        save_pool_and_emit(runtime, progress, state)?;
    }
    Ok(())
}

fn base_model_calibration_evidence(
    pool: &crate::model::RankedPool,
    model: &ModelIdentity,
) -> Vec<crate::model::PoolEntrantEvidence> {
    pool.calibration
        .iter()
        .filter(|evidence| is_same_base_model(&evidence.requested_model, model))
        .cloned()
        .collect()
}

fn base_model_qualification_evidence(
    pool: &crate::model::RankedPool,
    model: &ModelIdentity,
) -> Vec<crate::model::PoolEntrantEvidence> {
    pool.qualification
        .iter()
        .filter(|evidence| is_same_base_model(&evidence.requested_model, model))
        .cloned()
        .collect()
}

fn is_same_base_model(left: &ModelIdentity, right: &ModelIdentity) -> bool {
    left.tier == right.tier && left.provider == right.provider && left.model == right.model
}

fn is_thinking_decisions_complete(
    state: &PoolRunState,
    tier: Tier,
    pool_index: usize,
) -> Result<bool, SkillEvalError> {
    state.configuration.entrants[&tier]
        .iter()
        .map(|entrant| {
            let evidence =
                base_model_calibration_evidence(&state.pools[pool_index], &entrant.model);
            select_thinking_level(entrant, &evidence).map(|decision| decision.is_complete)
        })
        .try_fold(true, |is_all_complete, is_complete| {
            is_complete.map(|is_complete| is_all_complete && is_complete)
        })
}

fn selected_calibration_evidence(
    state: &PoolRunState,
    pool_index: usize,
) -> Result<Vec<crate::model::PoolEntrantEvidence>, SkillEvalError> {
    state.pools[pool_index]
        .thinking_selections
        .iter()
        .map(|selected| {
            state.pools[pool_index]
                .calibration
                .iter()
                .find(|evidence| evidence.requested_model == *selected && evidence.is_passing)
                .cloned()
                .ok_or_else(|| resume_drift("selected thinking calibration evidence"))
        })
        .collect()
}

fn restore_thinking_evidence(
    ranked: &mut crate::model::RankedPool,
    attempted: &crate::model::RankedPool,
) {
    ranked.calibration.clone_from(&attempted.calibration);
    ranked
        .thinking_selections
        .clone_from(&attempted.thinking_selections);
}

fn skip_unpromoted_qualification_children(
    state: &mut PoolRunState,
    tier: Tier,
    pool_index: usize,
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn PoolProgressSink,
) -> Result<(), SkillEvalError> {
    let entrants = state.configuration.entrants[&tier].clone();
    for (entrant_index, entrant) in entrants.iter().enumerate() {
        let calibration = base_model_calibration_evidence(&state.pools[pool_index], &entrant.model);
        let eligible = qualification_eligible_indices(entrant, &calibration)?;
        let qualification =
            base_model_qualification_evidence(&state.pools[pool_index], &entrant.model);
        let final_decision = if eligible.is_empty() {
            None
        } else {
            Some(select_qualification_thinking_level(
                entrant,
                &calibration,
                &qualification,
            )?)
        };
        let selected_index = final_decision
            .as_ref()
            .and_then(|decision| decision.selected.as_ref())
            .and_then(|selected| {
                entrant
                    .thinking_levels
                    .iter()
                    .position(|level| level == &selected.thinking)
            });
        for thinking_index in 0..entrant.thinking_levels.len() {
            let child_index = pool_child_slot(
                state,
                tier,
                u8::try_from(entrant_index)
                    .map_err(|_| resume_drift("pool child entrant index"))?,
                u8::try_from(thinking_index)
                    .map_err(|_| resume_drift("pool child thinking index"))?,
                PoolStage::Qualification,
            )?;
            if state.child_runs[child_index].status != PoolChildStatus::Pending {
                continue;
            }
            let is_skipped = !eligible.contains(&thinking_index)
                || selected_index.is_some_and(|selected| thinking_index > selected);
            if is_skipped {
                state.child_runs[child_index].status = PoolChildStatus::Skipped;
                save_pool_and_emit(runtime, progress, state)?;
            }
        }
    }
    Ok(())
}

fn qualification_eligible_indices(
    entrant: &PoolEntrant,
    calibration: &[crate::model::PoolEntrantEvidence],
) -> Result<BTreeSet<usize>, SkillEvalError> {
    let mut eligible = BTreeSet::new();
    if let Some(retained) = &entrant.retained_lower_thinking_level {
        let retained_index = entrant
            .thinking_levels
            .iter()
            .position(|level| level == retained)
            .ok_or_else(|| resume_drift("retained lower thinking level"))?;
        if calibration
            .iter()
            .any(|evidence| evidence.requested_model.thinking == *retained && evidence.is_passing)
        {
            eligible.insert(retained_index);
        }
    }
    if let Some(start) = qualification_start_index(entrant, calibration)? {
        eligible.extend(start..entrant.thinking_levels.len());
    }
    Ok(eligible)
}

fn is_selected_pool_walks_complete(state: &PoolRunState) -> Result<bool, SkillEvalError> {
    state
        .selected_tiers
        .iter()
        .try_fold(true, |is_all_complete, tier| {
            let Some(pool) = state.pools.iter().find(|pool| pool.tier == *tier) else {
                return Ok(false);
            };
            let mut is_tier_complete = true;
            for entrant in &state.configuration.entrants[tier] {
                let calibration = base_model_calibration_evidence(pool, &entrant.model);
                let screening = select_thinking_level(entrant, &calibration)?;
                if !screening.is_complete {
                    is_tier_complete = false;
                    continue;
                }
                if !qualification_eligible_indices(entrant, &calibration)?.is_empty() {
                    let qualification = base_model_qualification_evidence(pool, &entrant.model);
                    is_tier_complete &=
                        select_qualification_thinking_level(entrant, &calibration, &qualification)?
                            .is_complete;
                }
            }
            Ok(is_all_complete && is_tier_complete)
        })
}

fn review_failed_pool_child(
    state: &mut PoolRunState,
    child_index: usize,
    runtime: &mut dyn PoolRuntime,
    progress: &mut dyn PoolProgressSink,
) -> Result<(), SkillEvalError> {
    state.child_runs[child_index].status = PoolChildStatus::Failed;
    save_pool_and_emit(runtime, progress, state)?;
    state.status = PoolRunStatus::AwaitingDecision;
    state.pause = None;
    save_pool_and_emit(runtime, progress, state)
}

pub(crate) fn build_pool_report(
    run_id: &PoolRunId,
    store: &dyn crate::ports::PoolStore,
) -> Result<PoolRunState, SkillEvalError> {
    store.load_pool(run_id)
}

pub(crate) fn start_qualification_with_run_id(
    run_id: RunId,
    exact_candidate: Option<ModelIdentity>,
    request: QualifyRequest,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    start_qualification_for_run(
        run_id,
        exact_candidate,
        None,
        request,
        true,
        runtime,
        progress,
    )
}

fn start_pool_qualification_with_run_id(
    run_id: RunId,
    exact_candidate: ModelIdentity,
    candidate_timeout_seconds: Option<u32>,
    request: QualifyRequest,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    start_qualification_for_run(
        run_id,
        Some(exact_candidate),
        candidate_timeout_seconds,
        request,
        true,
        runtime,
        progress,
    )
}

pub(crate) fn start_qualification(
    request: QualifyRequest,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    validate_start_request(&request)?;
    let run_id = runtime.next()?;
    start_qualification_for_run(run_id, None, None, request, false, runtime, progress)
}

fn start_qualification_for_run(
    run_id: RunId,
    exact_candidate: Option<ModelIdentity>,
    candidate_timeout_seconds: Option<u32>,
    request: QualifyRequest,
    is_preallocated: bool,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    validate_start_request(&request)?;
    validate_run_id(&run_id)?;
    if candidate_timeout_seconds == Some(0) {
        return Err(SkillEvalError::InvalidConfiguration(
            "candidate timeout must be greater than zero".to_owned(),
        ));
    }
    if exact_candidate.is_none() && candidate_timeout_seconds.is_some() {
        return Err(SkillEvalError::InvalidConfiguration(
            "candidate timeout requires an exact pool candidate".to_owned(),
        ));
    }
    if is_preallocated {
        validate_exact_candidate_shape(&request, exact_candidate.as_ref())?;
    }

    let artifacts = request
        .artifact_roots
        .iter()
        .map(|root| runtime.load(root))
        .collect::<Result<Vec<_>, _>>()?;
    let mut artifact_names = BTreeSet::new();
    for artifact in &artifacts {
        validate_artifact(artifact)?;
        if !artifact_names.insert(artifact.name.clone()) {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "duplicate artifact {:?}",
                artifact.name.0
            )));
        }
    }
    validate_change(request.change.as_ref(), &artifacts)?;
    let exact_candidate = match exact_candidate {
        Some(requested) => {
            let effective = runtime.exact_candidate(&requested)?;
            if effective != requested {
                return Err(SkillEvalError::InvalidConfiguration(
                    "exact candidate resolution changed the requested identity".to_owned(),
                ));
            }
            Some(effective)
        }
        None => None,
    };

    let created_at = runtime.now();
    let mode = if request.is_dry_run {
        RunMode::DryRun
    } else {
        RunMode::Execute
    };
    let qualification_routes = if mode == RunMode::Execute && exact_candidate.is_none() {
        let configured_judge_tier = runtime.configured_judge_tier()?;
        if configured_judge_tier != request.policy.judge_tier {
            return Err(SkillEvalError::InvalidConfiguration(
                "configured judge tier differs from qualification policy".to_string(),
            ));
        }
        let tiers = std::iter::once(request.policy.reference_tier)
            .chain(request.policy.candidate_tiers.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut routes = BTreeMap::new();
        for tier in tiers {
            routes.insert(tier, exact_qualification_routes(runtime, tier)?);
        }
        routes
    } else {
        BTreeMap::new()
    };

    let configuration = RunConfiguration {
        run_id: run_id.clone(),
        mode,
        artifacts: artifacts.clone(),
        change: request.change.clone(),
        policy: request.policy.clone(),
        qualification_routes: qualification_routes.clone(),
        created_at: created_at.clone(),
    };
    append_and_emit(
        runtime,
        progress,
        &run_id,
        RunEvent::RunStarted {
            at: created_at,
            configuration,
        },
    )?;

    if request.is_dry_run {
        append_discovery(runtime, progress, &run_id, &artifacts)?;
        return build_report(&run_id, runtime);
    }

    if let Some(exact_candidate) = exact_candidate.as_ref() {
        append_discovery(runtime, progress, &run_id, &artifacts)?;
        return run_exact_pool_child(
            &run_id,
            &artifacts,
            exact_candidate,
            candidate_timeout_seconds,
            &request.policy,
            runtime,
            progress,
        );
    }

    for artifact in &artifacts {
        let reference = match run_tier_qualification(
            &run_id,
            artifact,
            request.policy.reference_tier,
            EvidenceRole::Reference,
            None,
            &request.policy,
            routes_for_tier(&qualification_routes, request.policy.reference_tier)?,
            runtime,
            progress,
        ) {
            Ok(evidence) => evidence,
            Err(StartTrialError::BeforeCheckpoint(error)) => return Err(error),
            Err(StartTrialError::AfterCheckpoint(error)) => {
                append_pause(runtime, progress, &run_id, error)?;
                return build_report(&run_id, runtime);
            }
        };
        let at = runtime.now();
        append_and_emit(
            runtime,
            progress,
            &run_id,
            RunEvent::TierEvaluated {
                at,
                artifact: artifact.name.clone(),
                evidence: reference.clone(),
            },
        )?;

        let mut candidate_evidence = Vec::new();
        for tier in &request.policy.candidate_tiers {
            let evidence = match run_tier_qualification(
                &run_id,
                artifact,
                *tier,
                EvidenceRole::Candidate,
                Some(&reference),
                &request.policy,
                routes_for_tier(&qualification_routes, *tier)?,
                runtime,
                progress,
            ) {
                Ok(evidence) => evidence,
                Err(StartTrialError::BeforeCheckpoint(error)) => return Err(error),
                Err(StartTrialError::AfterCheckpoint(error)) => {
                    append_pause(runtime, progress, &run_id, error)?;
                    return build_report(&run_id, runtime);
                }
            };
            let at = runtime.now();
            append_and_emit(
                runtime,
                progress,
                &run_id,
                RunEvent::TierEvaluated {
                    at,
                    artifact: artifact.name.clone(),
                    evidence: evidence.clone(),
                },
            )?;
            candidate_evidence.push(evidence);

            match find_boundary(&candidate_evidence, &request.policy) {
                Ok(Some(boundary)) => {
                    let at = runtime.now();
                    append_and_emit(
                        runtime,
                        progress,
                        &run_id,
                        RunEvent::BoundaryFound {
                            at,
                            artifact: artifact.name.clone(),
                            boundary,
                        },
                    )?;
                    break;
                }
                Ok(None) => {}
                Err(_) => {
                    append_review(
                        runtime,
                        progress,
                        &run_id,
                        &artifact.name,
                        "candidate evidence is non-monotonic",
                    )?;
                    return build_report(&run_id, runtime);
                }
            }
        }

        if find_boundary(&candidate_evidence, &request.policy)
            .is_ok_and(|boundary| boundary.is_none())
        {
            append_review(
                runtime,
                progress,
                &run_id,
                &artifact.name,
                "candidate evidence does not establish a supported boundary",
            )?;
            return build_report(&run_id, runtime);
        }
    }

    build_report(&run_id, runtime)
}

fn validate_exact_candidate_shape(
    request: &QualifyRequest,
    exact_candidate: Option<&ModelIdentity>,
) -> Result<(), SkillEvalError> {
    match (request.policy.purpose, exact_candidate) {
        (QualificationPurpose::Artifact, None) => Ok(()),
        (QualificationPurpose::Artifact, Some(_)) => Err(SkillEvalError::InvalidConfiguration(
            "artifact qualification cannot use an exact candidate".to_owned(),
        )),
        (QualificationPurpose::ModelPool, None) => Err(SkillEvalError::InvalidConfiguration(
            "model-pool qualification requires an exact candidate".to_owned(),
        )),
        (QualificationPurpose::ModelPool, Some(candidate))
            if request.policy.candidate_tiers.len() == 1
                && request.policy.candidate_tiers[0] == candidate.tier =>
        {
            Ok(())
        }
        (QualificationPurpose::ModelPool, Some(_)) => Err(SkillEvalError::InvalidConfiguration(
            "model-pool qualification requires one candidate tier matching the exact candidate"
                .to_owned(),
        )),
    }
}

fn append_discovery(
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
    run_id: &RunId,
    artifacts: &[ArtifactDefinition],
) -> Result<(), SkillEvalError> {
    let discoveries = artifacts
        .iter()
        .map(|artifact| ArtifactDiscovery {
            artifact: artifact.name.clone(),
            kind: artifact.kind,
            revision: artifact.revision.clone(),
            cases: artifact
                .cases
                .iter()
                .map(|case| CaseDiscovery {
                    id: case.id.clone(),
                    drive: case.execution.drive.clone(),
                    is_holdout: case.is_holdout,
                })
                .collect(),
        })
        .collect();
    let event = RunEvent::DiscoveryCompleted {
        at: runtime.now(),
        artifacts: discoveries,
    };
    append_and_emit(runtime, progress, run_id, event)
}

fn run_exact_pool_child(
    run_id: &RunId,
    artifacts: &[ArtifactDefinition],
    exact_candidate: &ModelIdentity,
    candidate_timeout_seconds: Option<u32>,
    policy: &QualificationPolicy,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    for artifact in artifacts {
        for case in artifact.cases.iter().filter(|case| !case.is_holdout) {
            let harness = runtime.identity(artifact, &case.execution)?;
            for attempt in 1..=policy.repeats_per_case {
                let key = TrialKey {
                    artifact: artifact.name.clone(),
                    tier: exact_candidate.tier,
                    route_index: 0,
                    case: case.id.clone(),
                    attempt,
                };
                let at = runtime.now();
                append_and_emit(
                    runtime,
                    progress,
                    run_id,
                    RunEvent::TrialStarted {
                        at,
                        key: key.clone(),
                        models: vec![exact_candidate.clone()],
                        harness: harness.clone(),
                    },
                )?;

                let candidate = match runtime.execute(
                    run_id,
                    &key,
                    artifact,
                    case,
                    exact_candidate,
                    &harness,
                    candidate_timeout_seconds,
                ) {
                    Ok(candidate) => candidate,
                    Err(
                        error @ (SkillEvalError::InvalidArguments(_)
                        | SkillEvalError::InvalidConfiguration(_)),
                    ) => return Err(error),
                    Err(error) => {
                        append_pause(runtime, progress, run_id, error)?;
                        return build_report(run_id, runtime);
                    }
                };
                if candidate.model != *exact_candidate {
                    return Err(SkillEvalError::InvalidConfiguration(
                        "exact candidate execution returned a different model".to_owned(),
                    ));
                }
                let at = runtime.now();
                append_and_emit(
                    runtime,
                    progress,
                    run_id,
                    RunEvent::CandidateExecuted {
                        at,
                        candidate: candidate.clone(),
                    },
                )?;

                let checks = match runtime.verify(case, &candidate) {
                    Ok(checks) => checks,
                    Err(
                        error @ (SkillEvalError::InvalidArguments(_)
                        | SkillEvalError::InvalidConfiguration(_)),
                    ) => return Err(error),
                    Err(error) => {
                        append_pause(runtime, progress, run_id, error)?;
                        return build_report(run_id, runtime);
                    }
                };
                let judge = match runtime.pool_judge(&candidate.model) {
                    Ok(judge) => judge,
                    Err(
                        error @ (SkillEvalError::InvalidArguments(_)
                        | SkillEvalError::InvalidConfiguration(_)
                        | SkillEvalError::JudgeUnavailable { .. }),
                    ) => return Err(error),
                    Err(error) => {
                        append_pause(runtime, progress, run_id, error)?;
                        return build_report(run_id, runtime);
                    }
                };
                validate_external_judge(&candidate.model, &judge, judge.tier)?;
                let judged = match runtime.grade(
                    &judge,
                    &JudgeInput {
                        candidate: candidate.clone(),
                        expect: case.expect.clone(),
                        rubric_path: artifact.root.join("evals/rubric.md"),
                        checks,
                    },
                ) {
                    Ok(judged) => judged,
                    Err(
                        error @ (SkillEvalError::InvalidArguments(_)
                        | SkillEvalError::InvalidConfiguration(_)
                        | SkillEvalError::JudgeUnavailable { .. }),
                    ) => return Err(error),
                    Err(error) => {
                        append_pause(runtime, progress, run_id, error)?;
                        return build_report(run_id, runtime);
                    }
                };
                validate_external_judge(&candidate.model, &judged.model, judged.model.tier)?;
                let record = TrialRecord {
                    key,
                    model: candidate.model.clone(),
                    harness: candidate.harness.clone(),
                    artifact_path: candidate.artifact_path.clone(),
                    transcript_path: candidate.transcript_path.clone(),
                    candidate_usage: candidate.usage.clone(),
                    judge_model: judged.model,
                    judge_usage: judged.usage,
                    verdict: judged.verdict,
                };
                let at = runtime.now();
                append_and_emit(
                    runtime,
                    progress,
                    run_id,
                    RunEvent::TrialCompleted { at, record },
                )?;
            }
        }
        let at = runtime.now();
        append_and_emit(
            runtime,
            progress,
            run_id,
            RunEvent::PoolChildCompleted {
                at,
                artifact: artifact.name.clone(),
                tier: exact_candidate.tier,
            },
        )?;
    }
    build_report(run_id, runtime)
}

enum StartTrialError {
    BeforeCheckpoint(SkillEvalError),
    AfterCheckpoint(SkillEvalError),
}

#[expect(
    clippy::too_many_arguments,
    reason = "tier qualification needs the route and evidence context"
)]
fn run_tier_qualification(
    run_id: &RunId,
    artifact: &ArtifactDefinition,
    tier: Tier,
    role: EvidenceRole,
    reference: Option<&TierEvidence>,
    policy: &QualificationPolicy,
    routes: &[ModelIdentity],
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<TierEvidence, StartTrialError> {
    let mut last_evidence = None;
    for (route_index, route) in routes.iter().enumerate() {
        let route_index = u16::try_from(route_index).map_err(|_| {
            StartTrialError::BeforeCheckpoint(SkillEvalError::InvalidConfiguration(format!(
                "tier {tier:?} qualification route index is out of range"
            )))
        })?;
        let trials = run_exact_route_trials(
            run_id,
            artifact,
            tier,
            route_index,
            route,
            policy,
            runtime,
            progress,
        )?;
        let evidence = evaluate_tier(role, &trials, reference, policy)
            .map_err(StartTrialError::BeforeCheckpoint)?;
        if evidence.status == TierStatus::Accepted {
            return Ok(evidence);
        }
        last_evidence = Some(evidence);
    }
    last_evidence.ok_or_else(|| {
        StartTrialError::BeforeCheckpoint(SkillEvalError::InvalidConfiguration(format!(
            "artifact qualification route order for tier {tier:?} is absent"
        )))
    })
}

fn exact_qualification_routes(
    runtime: &dyn QualificationRuntime,
    tier: Tier,
) -> Result<Vec<ModelIdentity>, SkillEvalError> {
    let routes = runtime.qualification_routes(tier)?;
    validate_exact_qualification_routes(runtime, tier, &routes)?;
    Ok(routes)
}

fn validate_exact_qualification_routes(
    runtime: &dyn QualificationRuntime,
    tier: Tier,
    routes: &[ModelIdentity],
) -> Result<(), SkillEvalError> {
    if routes.is_empty() {
        return Err(SkillEvalError::InvalidConfiguration(format!(
            "artifact qualification route order for tier {tier:?} is absent"
        )));
    }
    u16::try_from(routes.len() - 1).map_err(|_| {
        SkillEvalError::InvalidConfiguration(format!(
            "artifact qualification route order for tier {tier:?} is too long"
        ))
    })?;
    let mut exact_routes = BTreeSet::new();
    for route in routes {
        if route.tier != tier || runtime.exact_candidate(route)? != *route {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "artifact qualification route for tier {tier:?} is not exact"
            )));
        }
        if !exact_routes.insert((
            route.provider.clone(),
            route.model.clone(),
            route.thinking.clone(),
        )) {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "artifact qualification route order for tier {tier:?} contains a duplicate exact route"
            )));
        }
    }
    Ok(())
}

fn routes_for_tier(
    routes: &BTreeMap<Tier, Vec<ModelIdentity>>,
    tier: Tier,
) -> Result<&[ModelIdentity], SkillEvalError> {
    routes.get(&tier).map(Vec::as_slice).ok_or_else(|| {
        SkillEvalError::InvalidConfiguration(format!(
            "frozen artifact qualification routes for tier {tier:?} are absent"
        ))
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "trial execution needs the frozen route context"
)]
fn run_exact_route_trials(
    run_id: &RunId,
    artifact: &ArtifactDefinition,
    tier: Tier,
    route_index: u16,
    route: &ModelIdentity,
    policy: &QualificationPolicy,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<Vec<TrialRecord>, StartTrialError> {
    let mut trials = Vec::with_capacity(
        artifact
            .cases
            .len()
            .saturating_mul(usize::from(policy.repeats_per_case)),
    );

    for case in &artifact.cases {
        let harness = runtime
            .identity(artifact, &case.execution)
            .map_err(StartTrialError::BeforeCheckpoint)?;
        for attempt in 1..=policy.repeats_per_case {
            let key = TrialKey {
                artifact: artifact.name.clone(),
                tier,
                route_index,
                case: case.id.clone(),
                attempt,
            };
            let at = runtime.now();
            append_and_emit(
                runtime,
                progress,
                run_id,
                RunEvent::TrialStarted {
                    at,
                    key: key.clone(),
                    models: vec![route.clone()],
                    harness: harness.clone(),
                },
            )
            .map_err(StartTrialError::BeforeCheckpoint)?;

            let candidate = runtime
                .execute(
                    run_id,
                    &key,
                    artifact,
                    case,
                    route,
                    &harness,
                    Some(case.execution.timeout_seconds),
                )
                .map_err(StartTrialError::BeforeCheckpoint)?;
            if candidate.key != key || candidate.model != *route || candidate.harness != harness {
                return Err(StartTrialError::BeforeCheckpoint(resume_drift(
                    "exact artifact qualification route",
                )));
            }
            let at = runtime.now();
            append_and_emit(
                runtime,
                progress,
                run_id,
                RunEvent::CandidateExecuted {
                    at,
                    candidate: candidate.clone(),
                },
            )
            .map_err(StartTrialError::BeforeCheckpoint)?;

            let checks = runtime.verify(case, &candidate).map_err(checkpoint_error)?;
            let judge = runtime
                .judge(policy.judge_tier, Some(&candidate.model))
                .map_err(checkpoint_error)?;
            validate_external_judge(&candidate.model, &judge, policy.judge_tier)
                .map_err(checkpoint_error)?;
            let judged = runtime
                .grade(
                    &judge,
                    &JudgeInput {
                        candidate: candidate.clone(),
                        expect: case.expect.clone(),
                        rubric_path: artifact.root.join("evals/rubric.md"),
                        checks,
                    },
                )
                .map_err(checkpoint_error)?;
            validate_external_judge(&candidate.model, &judged.model, policy.judge_tier)
                .map_err(checkpoint_error)?;
            let record = TrialRecord {
                key,
                model: candidate.model.clone(),
                harness: candidate.harness.clone(),
                artifact_path: candidate.artifact_path.clone(),
                transcript_path: candidate.transcript_path.clone(),
                candidate_usage: candidate.usage.clone(),
                judge_model: judged.model,
                judge_usage: judged.usage,
                verdict: judged.verdict,
            };
            let at = runtime.now();
            append_and_emit(
                runtime,
                progress,
                run_id,
                RunEvent::TrialCompleted {
                    at,
                    record: record.clone(),
                },
            )
            .map_err(checkpoint_error)?;
            trials.push(record);
        }
    }

    Ok(trials)
}

fn validate_external_judge(
    candidate: &crate::model::ModelIdentity,
    judge: &crate::model::ModelIdentity,
    judge_tier: Tier,
) -> Result<(), SkillEvalError> {
    if candidate.provider == judge.provider && candidate.model == judge.model {
        Err(SkillEvalError::InvalidConfiguration(format!(
            "judge tier {judge_tier:?} resolved to the candidate model"
        )))
    } else {
        Ok(())
    }
}

fn checkpoint_error(error: SkillEvalError) -> StartTrialError {
    match error {
        SkillEvalError::InvalidArguments(_) | SkillEvalError::InvalidConfiguration(_) => {
            StartTrialError::BeforeCheckpoint(error)
        }
        _ => StartTrialError::AfterCheckpoint(error),
    }
}

fn append_and_emit(
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
    run_id: &RunId,
    event: RunEvent,
) -> Result<(), SkillEvalError> {
    runtime.append(run_id, &event)?;
    progress.emit(&event)
}

fn append_pause(
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
    run_id: &RunId,
    error: SkillEvalError,
) -> Result<(), SkillEvalError> {
    let reason = match error {
        SkillEvalError::Quota { model, reset_at } => PauseReason::Quota { model, reset_at },
        error => PauseReason::Infrastructure {
            message: format!("{error:?}"),
        },
    };
    let at = runtime.now();
    append_and_emit(
        runtime,
        progress,
        run_id,
        RunEvent::RunPaused { at, reason },
    )
}

fn append_review(
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
    run_id: &RunId,
    artifact: &ArtifactName,
    reason: &str,
) -> Result<(), SkillEvalError> {
    let at = runtime.now();
    append_and_emit(
        runtime,
        progress,
        run_id,
        RunEvent::ReviewRequired {
            at,
            artifact: artifact.clone(),
            reason: reason.to_string(),
        },
    )
}

fn validate_start_request(request: &QualifyRequest) -> Result<(), SkillEvalError> {
    if request.artifact_roots.is_empty() {
        return Err(SkillEvalError::InvalidArguments(
            "qualification requires at least one artifact".to_string(),
        ));
    }
    if request.policy.candidate_tiers.is_empty()
        || request.policy.repeats_per_case == 0
        || request.policy.minimum_score > 10
        || !request.policy.noninferiority_margin.is_finite()
        || request.policy.noninferiority_margin < 0.0
        || !request.policy.confidence_level.is_finite()
        || request.policy.confidence_level <= 0.0
        || request.policy.confidence_level >= 1.0
    {
        return Err(SkillEvalError::InvalidConfiguration(
            "qualification policy has invalid values".to_string(),
        ));
    }
    let tiers = request
        .policy
        .candidate_tiers
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let is_candidate_order_valid = match request.policy.purpose {
        QualificationPurpose::Artifact => {
            tiers.iter().all(|tier| request.policy.judge_tier > *tier)
        }
        QualificationPurpose::ModelPool => tiers.iter().all(|tier| {
            request.policy.judge_tier > *tier
                || (*tier == Tier::T5 && request.policy.judge_tier == Tier::T5)
        }),
    };
    if tiers.len() != request.policy.candidate_tiers.len()
        || tiers.contains(&request.policy.reference_tier)
        || request.policy.judge_tier <= request.policy.reference_tier
        || !is_candidate_order_valid
    {
        return Err(SkillEvalError::InvalidConfiguration(
            "qualification policy has invalid tier ordering".to_string(),
        ));
    }
    Ok(())
}

fn validate_artifact(artifact: &ArtifactDefinition) -> Result<(), SkillEvalError> {
    report_destinations(artifact)?;
    if artifact.revision.trim().is_empty() || artifact.cases.is_empty() {
        return Err(SkillEvalError::InvalidConfiguration(format!(
            "artifact {:?} has no revision or cases",
            artifact.name.0
        )));
    }
    let mut cases = BTreeSet::new();
    if artifact
        .cases
        .iter()
        .any(|case| !cases.insert(case.id.clone()))
    {
        return Err(SkillEvalError::InvalidConfiguration(format!(
            "artifact {:?} has duplicate case identifiers",
            artifact.name.0
        )));
    }
    Ok(())
}

fn validate_change(
    change: Option<&ArtifactChange>,
    artifacts: &[ArtifactDefinition],
) -> Result<(), SkillEvalError> {
    let Some(change) = change else {
        return Ok(());
    };
    let artifact = artifacts
        .iter()
        .find(|artifact| artifact.name == change.artifact)
        .ok_or_else(|| {
            SkillEvalError::InvalidConfiguration(
                "artifact change names an artifact outside the run".to_string(),
            )
        })?;
    if artifact.kind != change.kind
        || artifact.revision != change.candidate_revision
        || change.incumbent_revision.trim().is_empty()
        || change.candidate_revision.trim().is_empty()
        || change.incumbent_revision == change.candidate_revision
        || change.own_eval.artifact_revision != change.candidate_revision
        || change.own_eval.path.as_os_str().is_empty()
    {
        return Err(SkillEvalError::InvalidConfiguration(
            "artifact change identity or own-eval revision is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_run_id(run_id: &RunId) -> Result<(), SkillEvalError> {
    let mut components = Path::new(&run_id.0).components();
    let is_path_safe = matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
        && !run_id.0.is_empty();
    if !is_path_safe {
        return Err(SkillEvalError::InvalidArguments(format!(
            "run identifier {:?} must be one path component",
            run_id.0
        )));
    }
    Ok(())
}

pub(crate) fn resume_qualification(
    run_id: &RunId,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    validate_run_id(run_id)?;
    let report = build_report(run_id, runtime)?;
    if matches!(
        report.status,
        RunStatus::AwaitingDecision | RunStatus::Completed | RunStatus::Failed
    ) {
        return Ok(report);
    }
    if report.status != RunStatus::Paused || report.pause.is_none() {
        return Err(SkillEvalError::InvalidConfiguration(
            "resume requires a paused qualification run".to_string(),
        ));
    }

    let replay = replay_resume_data(run_id, runtime)?;
    let configuration = replay.configuration.as_ref().ok_or_else(|| {
        SkillEvalError::InvalidConfiguration("run has no frozen configuration".to_string())
    })?;
    if configuration.policy.purpose == QualificationPurpose::ModelPool {
        return Err(SkillEvalError::InvalidConfiguration(
            "resume a model-pool child through its parent pool run".to_owned(),
        ));
    }
    validate_resume_environment(configuration, &replay, runtime)?;

    let at = runtime.now();
    append_and_emit(runtime, progress, run_id, RunEvent::RunResumed { at })?;
    continue_qualification(configuration, &report, &replay, runtime, progress)
}

#[derive(Default)]
struct ResumeData {
    configuration: Option<RunConfiguration>,
    started: BTreeMap<TrialKey, StartedTrial>,
    candidates: BTreeMap<TrialKey, crate::model::CandidateArtifact>,
    completed: BTreeMap<TrialKey, TrialRecord>,
}

struct StartedTrial {
    models: Vec<crate::model::ModelIdentity>,
    harness: crate::model::HarnessIdentity,
}

fn replay_resume_data(run_id: &RunId, store: &dyn RunStore) -> Result<ResumeData, SkillEvalError> {
    let mut replay = ResumeData::default();
    store.replay(run_id, &mut |event| {
        match event {
            RunEvent::RunStarted { configuration, .. } => {
                if replay.configuration.replace(configuration).is_some() {
                    return Err(SkillEvalError::InvalidConfiguration(
                        "run has more than one frozen configuration".to_string(),
                    ));
                }
            }
            RunEvent::TrialStarted {
                key,
                models,
                harness,
                ..
            } => {
                if models.is_empty()
                    || replay
                        .started
                        .insert(key, StartedTrial { models, harness })
                        .is_some()
                {
                    return Err(SkillEvalError::InvalidConfiguration(
                        "run has an invalid or duplicate trial checkpoint".to_string(),
                    ));
                }
            }
            RunEvent::CandidateExecuted { candidate, .. } => {
                if replay
                    .candidates
                    .insert(candidate.key.clone(), candidate)
                    .is_some()
                {
                    return Err(SkillEvalError::InvalidConfiguration(
                        "run has a duplicate candidate checkpoint".to_string(),
                    ));
                }
            }
            RunEvent::TrialCompleted { record, .. } => {
                if replay
                    .completed
                    .insert(record.key.clone(), record)
                    .is_some()
                {
                    return Err(SkillEvalError::InvalidConfiguration(
                        "run has a duplicate completed trial".to_string(),
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(replay)
}

fn validate_resume_environment(
    configuration: &RunConfiguration,
    replay: &ResumeData,
    runtime: &dyn QualificationRuntime,
) -> Result<(), SkillEvalError> {
    if configuration.mode != RunMode::Execute {
        return Err(resume_drift("run mode"));
    }
    validate_start_request(&QualifyRequest {
        artifact_roots: configuration
            .artifacts
            .iter()
            .map(|artifact| artifact.root.clone())
            .collect(),
        change: configuration.change.clone(),
        policy: configuration.policy.clone(),
        is_dry_run: false,
    })?;
    validate_frozen_resume_routes(configuration, runtime)?;
    if runtime.configured_judge_tier()? != configuration.policy.judge_tier {
        return Err(resume_drift("qualification policy"));
    }

    let mut planned_keys = BTreeSet::new();
    for artifact in &configuration.artifacts {
        let current = runtime.load(&artifact.root)?;
        if current.name != artifact.name
            || current.kind != artifact.kind
            || current.revision != artifact.revision
        {
            return Err(resume_drift("artifact revision"));
        }
        for case in &artifact.cases {
            let current_harness = runtime.identity(artifact, &case.execution)?;
            for tier in std::iter::once(configuration.policy.reference_tier)
                .chain(configuration.policy.candidate_tiers.iter().copied())
            {
                for (route_index, route) in
                    routes_for_tier(&configuration.qualification_routes, tier)?
                        .iter()
                        .enumerate()
                {
                    let route_index = u16::try_from(route_index)
                        .map_err(|_| resume_drift("qualification route index"))?;
                    for attempt in 1..=configuration.policy.repeats_per_case {
                        let key = TrialKey {
                            artifact: artifact.name.clone(),
                            tier,
                            route_index,
                            case: case.id.clone(),
                            attempt,
                        };
                        planned_keys.insert(key.clone());
                        if let Some(started) = replay.started.get(&key) {
                            if started.models.len() != 1 {
                                return Err(SkillEvalError::InvalidConfiguration(
                                    "resume rejected legacy artifact qualification route schema; start a new run"
                                        .to_owned(),
                                ));
                            }
                            if started.models[0] != *route {
                                return Err(resume_drift("model route"));
                            }
                            if started.harness != current_harness {
                                return Err(resume_drift("harness identity"));
                            }
                        }
                    }
                }
            }
        }
    }

    if replay
        .started
        .keys()
        .chain(replay.candidates.keys())
        .chain(replay.completed.keys())
        .any(|key| !planned_keys.contains(key))
    {
        return Err(resume_drift("qualification policy"));
    }
    for (key, candidate) in &replay.candidates {
        let started = replay
            .started
            .get(key)
            .ok_or_else(|| resume_drift("candidate checkpoint"))?;
        if candidate.key != *key
            || !started.models.contains(&candidate.model)
            || candidate.harness != started.harness
        {
            return Err(resume_drift("candidate checkpoint"));
        }
    }
    for (key, record) in &replay.completed {
        let candidate = replay
            .candidates
            .get(key)
            .ok_or_else(|| resume_drift("candidate checkpoint"))?;
        if !candidate_matches_record(candidate, record) {
            return Err(resume_drift("candidate checkpoint"));
        }
    }
    Ok(())
}

fn validate_frozen_resume_routes(
    configuration: &RunConfiguration,
    runtime: &dyn QualificationRuntime,
) -> Result<(), SkillEvalError> {
    if configuration.policy.purpose != QualificationPurpose::Artifact {
        return Ok(());
    }
    if configuration.qualification_routes.is_empty() {
        return Err(SkillEvalError::InvalidConfiguration(
            "resume rejected legacy artifact qualification run without frozen routes; start a new run"
                .to_owned(),
        ));
    }
    let tiers = std::iter::once(configuration.policy.reference_tier)
        .chain(configuration.policy.candidate_tiers.iter().copied())
        .collect::<BTreeSet<_>>();
    if configuration
        .qualification_routes
        .keys()
        .copied()
        .collect::<BTreeSet<_>>()
        != tiers
    {
        return Err(resume_drift("frozen qualification routes"));
    }
    for (tier, routes) in &configuration.qualification_routes {
        validate_exact_qualification_routes(runtime, *tier, routes)?;
    }
    Ok(())
}

fn continue_qualification(
    configuration: &RunConfiguration,
    report: &QualificationReport,
    replay: &ResumeData,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    for artifact in &configuration.artifacts {
        let prior = report
            .artifacts
            .iter()
            .find(|current| current.artifact == artifact.name)
            .ok_or_else(|| resume_drift("report artifact state"))?;
        if prior.boundary.is_some() || prior.review_reason.is_some() {
            continue;
        }

        let reference = if let Some(reference) = &prior.reference {
            reference.clone()
        } else {
            let evidence = match resume_tier_qualification(
                &configuration.run_id,
                artifact,
                configuration.policy.reference_tier,
                EvidenceRole::Reference,
                None,
                &configuration.policy,
                routes_for_tier(
                    &configuration.qualification_routes,
                    configuration.policy.reference_tier,
                )?,
                replay,
                runtime,
                progress,
            ) {
                Ok(evidence) => evidence,
                Err(StartTrialError::BeforeCheckpoint(error)) => return Err(error),
                Err(StartTrialError::AfterCheckpoint(error)) => {
                    append_pause(runtime, progress, &configuration.run_id, error)?;
                    return build_report(&configuration.run_id, runtime);
                }
            };
            let at = runtime.now();
            append_and_emit(
                runtime,
                progress,
                &configuration.run_id,
                RunEvent::TierEvaluated {
                    at,
                    artifact: artifact.name.clone(),
                    evidence: evidence.clone(),
                },
            )?;
            evidence
        };

        let mut candidate_evidence = prior.tiers.clone();
        for tier in &configuration.policy.candidate_tiers {
            if candidate_evidence
                .iter()
                .any(|evidence| evidence.tier == *tier)
            {
                if find_boundary(&candidate_evidence, &configuration.policy)?.is_some() {
                    append_resume_boundary(
                        runtime,
                        progress,
                        &configuration.run_id,
                        artifact,
                        &candidate_evidence,
                        &configuration.policy,
                    )?;
                    break;
                }
                continue;
            }
            let evidence = match resume_tier_qualification(
                &configuration.run_id,
                artifact,
                *tier,
                EvidenceRole::Candidate,
                Some(&reference),
                &configuration.policy,
                routes_for_tier(&configuration.qualification_routes, *tier)?,
                replay,
                runtime,
                progress,
            ) {
                Ok(evidence) => evidence,
                Err(StartTrialError::BeforeCheckpoint(error)) => return Err(error),
                Err(StartTrialError::AfterCheckpoint(error)) => {
                    append_pause(runtime, progress, &configuration.run_id, error)?;
                    return build_report(&configuration.run_id, runtime);
                }
            };
            let at = runtime.now();
            append_and_emit(
                runtime,
                progress,
                &configuration.run_id,
                RunEvent::TierEvaluated {
                    at,
                    artifact: artifact.name.clone(),
                    evidence: evidence.clone(),
                },
            )?;
            candidate_evidence.push(evidence);

            match find_boundary(&candidate_evidence, &configuration.policy) {
                Ok(Some(_)) => {
                    append_resume_boundary(
                        runtime,
                        progress,
                        &configuration.run_id,
                        artifact,
                        &candidate_evidence,
                        &configuration.policy,
                    )?;
                    break;
                }
                Ok(None) => {}
                Err(_) => {
                    append_review(
                        runtime,
                        progress,
                        &configuration.run_id,
                        &artifact.name,
                        "candidate evidence is non-monotonic",
                    )?;
                    return build_report(&configuration.run_id, runtime);
                }
            }
        }

        if find_boundary(&candidate_evidence, &configuration.policy)
            .is_ok_and(|boundary| boundary.is_none())
        {
            append_review(
                runtime,
                progress,
                &configuration.run_id,
                &artifact.name,
                "candidate evidence does not establish a supported boundary",
            )?;
            return build_report(&configuration.run_id, runtime);
        }
    }
    build_report(&configuration.run_id, runtime)
}

#[expect(
    clippy::too_many_arguments,
    reason = "resume needs the frozen route and evidence context"
)]
fn resume_tier_qualification(
    run_id: &RunId,
    artifact: &ArtifactDefinition,
    tier: Tier,
    role: EvidenceRole,
    reference: Option<&TierEvidence>,
    policy: &QualificationPolicy,
    routes: &[ModelIdentity],
    replay: &ResumeData,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<TierEvidence, StartTrialError> {
    let mut last_evidence = None;
    for (route_index, route) in routes.iter().enumerate() {
        let route_index = u16::try_from(route_index).map_err(|_| {
            StartTrialError::BeforeCheckpoint(resume_drift("qualification route index"))
        })?;
        let trials = resume_exact_route_trials(
            run_id,
            artifact,
            tier,
            route_index,
            route,
            policy,
            replay,
            runtime,
            progress,
        )?;
        let evidence = evaluate_tier(role, &trials, reference, policy)
            .map_err(StartTrialError::BeforeCheckpoint)?;
        if evidence.status == TierStatus::Accepted {
            return Ok(evidence);
        }
        last_evidence = Some(evidence);
    }
    last_evidence.ok_or_else(|| {
        StartTrialError::BeforeCheckpoint(SkillEvalError::InvalidConfiguration(format!(
            "artifact qualification route order for tier {tier:?} is absent"
        )))
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "resume needs the frozen exact trial context"
)]
fn resume_exact_route_trials(
    run_id: &RunId,
    artifact: &ArtifactDefinition,
    tier: Tier,
    route_index: u16,
    route: &ModelIdentity,
    policy: &QualificationPolicy,
    replay: &ResumeData,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<Vec<TrialRecord>, StartTrialError> {
    let mut trials = Vec::with_capacity(
        artifact
            .cases
            .len()
            .saturating_mul(usize::from(policy.repeats_per_case)),
    );
    for case in &artifact.cases {
        for attempt in 1..=policy.repeats_per_case {
            let key = TrialKey {
                artifact: artifact.name.clone(),
                tier,
                route_index,
                case: case.id.clone(),
                attempt,
            };
            if let Some(record) = replay.completed.get(&key) {
                trials.push(record.clone());
                continue;
            }

            let harness = if let Some(started) = replay.started.get(&key) {
                if started.models != [route.clone()] {
                    return Err(StartTrialError::BeforeCheckpoint(resume_drift(
                        "exact model route",
                    )));
                }
                started.harness.clone()
            } else {
                let harness = runtime
                    .identity(artifact, &case.execution)
                    .map_err(StartTrialError::BeforeCheckpoint)?;
                let at = runtime.now();
                append_and_emit(
                    runtime,
                    progress,
                    run_id,
                    RunEvent::TrialStarted {
                        at,
                        key: key.clone(),
                        models: vec![route.clone()],
                        harness: harness.clone(),
                    },
                )
                .map_err(StartTrialError::BeforeCheckpoint)?;
                harness
            };

            let candidate = if let Some(candidate) = replay.candidates.get(&key) {
                candidate.clone()
            } else {
                let candidate = runtime
                    .execute(
                        run_id,
                        &key,
                        artifact,
                        case,
                        route,
                        &harness,
                        Some(case.execution.timeout_seconds),
                    )
                    .map_err(StartTrialError::BeforeCheckpoint)?;
                if candidate.key != key || candidate.model != *route || candidate.harness != harness
                {
                    return Err(StartTrialError::BeforeCheckpoint(resume_drift(
                        "exact artifact qualification route",
                    )));
                }
                let at = runtime.now();
                append_and_emit(
                    runtime,
                    progress,
                    run_id,
                    RunEvent::CandidateExecuted {
                        at,
                        candidate: candidate.clone(),
                    },
                )
                .map_err(StartTrialError::BeforeCheckpoint)?;
                candidate
            };

            let checks = runtime.verify(case, &candidate).map_err(checkpoint_error)?;
            let judge = runtime
                .judge(policy.judge_tier, Some(&candidate.model))
                .map_err(checkpoint_error)?;
            validate_external_judge(&candidate.model, &judge, policy.judge_tier)
                .map_err(checkpoint_error)?;
            let judged = runtime
                .grade(
                    &judge,
                    &JudgeInput {
                        candidate: candidate.clone(),
                        expect: case.expect.clone(),
                        rubric_path: artifact.root.join("evals/rubric.md"),
                        checks,
                    },
                )
                .map_err(checkpoint_error)?;
            validate_external_judge(&candidate.model, &judged.model, policy.judge_tier)
                .map_err(checkpoint_error)?;
            let record = TrialRecord {
                key,
                model: candidate.model.clone(),
                harness: candidate.harness.clone(),
                artifact_path: candidate.artifact_path.clone(),
                transcript_path: candidate.transcript_path.clone(),
                candidate_usage: candidate.usage.clone(),
                judge_model: judged.model,
                judge_usage: judged.usage,
                verdict: judged.verdict,
            };
            let at = runtime.now();
            append_and_emit(
                runtime,
                progress,
                run_id,
                RunEvent::TrialCompleted {
                    at,
                    record: record.clone(),
                },
            )
            .map_err(checkpoint_error)?;
            trials.push(record);
        }
    }
    Ok(trials)
}

fn append_resume_boundary(
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
    run_id: &RunId,
    artifact: &ArtifactDefinition,
    evidence: &[TierEvidence],
    policy: &QualificationPolicy,
) -> Result<(), SkillEvalError> {
    let boundary = find_boundary(evidence, policy)?.ok_or_else(|| {
        SkillEvalError::InvalidConfiguration("candidate boundary is incomplete".to_string())
    })?;
    let at = runtime.now();
    append_and_emit(
        runtime,
        progress,
        run_id,
        RunEvent::BoundaryFound {
            at,
            artifact: artifact.name.clone(),
            boundary,
        },
    )
}

fn resume_drift(identity: &str) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(format!(
        "resume rejected {identity} drift from the frozen run"
    ))
}

pub(crate) fn build_report(
    run_id: &RunId,
    store: &dyn RunStore,
) -> Result<QualificationReport, SkillEvalError> {
    let mut state = empty_run_state();
    let mut artifact_kinds = BTreeMap::new();
    let mut required_destinations = BTreeMap::new();
    let mut change = None;
    let mut total_usage = empty_usage();
    let mut line = 0_u64;

    store.replay(run_id, &mut |event| {
        line = line
            .checked_add(1)
            .ok_or_else(|| SkillEvalError::InvalidEvent {
                line: u64::MAX,
                message: "event log has too many lines".to_string(),
            })?;

        validate_report_event(run_id, &event, line, &artifact_kinds, change.as_ref())?;
        apply_event(&mut state, &event).map_err(|error| event_at_line(error, line))?;

        match &event {
            RunEvent::RunStarted { configuration, .. } => {
                for artifact in &configuration.artifacts {
                    artifact_kinds.insert(artifact.name.clone(), artifact.kind);
                    required_destinations
                        .insert(artifact.name.clone(), report_destinations(artifact)?);
                }
                change = configuration.change.clone();
            }
            RunEvent::CandidateExecuted { candidate, .. } => {
                add_usage(&mut total_usage, &candidate.usage, line)?;
            }
            RunEvent::TrialCompleted { record, .. } => {
                add_usage(&mut total_usage, &record.judge_usage, line)?;
            }
            _ => {}
        }
        Ok(())
    })?;

    if state.run_id.0.is_empty() {
        return Err(invalid_event(1, "event log has no run_started event"));
    }
    if state.run_id != *run_id {
        return Err(invalid_event(
            1,
            "run_started identity differs from the requested run",
        ));
    }

    let artifacts = state
        .artifacts
        .iter()
        .map(|(artifact, qualification)| {
            let kind = artifact_kinds.get(artifact).copied().ok_or_else(|| {
                SkillEvalError::InvalidConfiguration(format!(
                    "artifact {:?} has no kind in run configuration",
                    artifact.0
                ))
            })?;
            let reference = qualification
                .tiers
                .iter()
                .find(|evidence| evidence.role == EvidenceRole::Reference)
                .cloned();
            let tiers = qualification
                .tiers
                .iter()
                .filter(|evidence| evidence.role == EvidenceRole::Candidate)
                .cloned()
                .collect();
            let required_destinations =
                required_destinations
                    .get(artifact)
                    .cloned()
                    .ok_or_else(|| {
                        SkillEvalError::InvalidConfiguration(format!(
                            "artifact {:?} has no destination definition in run configuration",
                            artifact.0
                        ))
                    })?;
            Ok(ArtifactReport {
                artifact: artifact.clone(),
                kind,
                required_destinations,
                status: qualification.status,
                review_reason: qualification.review_reason.clone(),
                pending_candidates: qualification.pending_candidates.clone(),
                reference,
                tiers,
                boundary: qualification.boundary.clone(),
                decision: state.decisions.get(artifact).cloned(),
                publication_gate: state.publication_gates.get(artifact).cloned(),
            })
        })
        .collect::<Result<Vec<_>, SkillEvalError>>()?;

    Ok(QualificationReport {
        run_id: state.run_id,
        mode: state.mode,
        change,
        status: state.status,
        discoveries: state.discoveries,
        artifacts,
        pause: state.pause,
        total_usage,
    })
}

fn report_destinations(
    artifact: &ArtifactDefinition,
) -> Result<Vec<TierDestination>, SkillEvalError> {
    let mut seen = BTreeSet::new();
    let mut is_skill_minimum_declared = false;
    let mut is_agent_declared = false;
    let mut is_orchestrator_declared = false;

    for destination in &artifact.required_destinations {
        if !seen.insert(destination.clone()) {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "artifact {:?} repeats a required tier destination",
                artifact.name.0
            )));
        }

        match (artifact.kind, destination) {
            (ArtifactKind::Skill, TierDestination::SkillMinimum) => {
                is_skill_minimum_declared = true;
            }
            (ArtifactKind::Skill, TierDestination::SkillTarget) => {}
            (ArtifactKind::Agent, TierDestination::Agent) => {
                is_agent_declared = true;
            }
            (ArtifactKind::Workflow, TierDestination::WorkflowOrchestrator) => {
                is_orchestrator_declared = true;
            }
            (ArtifactKind::Workflow, TierDestination::WorkflowNode { node })
                if !node.trim().is_empty() => {}
            _ => {
                return Err(SkillEvalError::InvalidConfiguration(format!(
                    "artifact {:?} has a required destination for another artifact kind",
                    artifact.name.0
                )));
            }
        }
    }

    let is_complete = match artifact.kind {
        ArtifactKind::Skill => is_skill_minimum_declared,
        ArtifactKind::Agent => is_agent_declared,
        ArtifactKind::Workflow => is_orchestrator_declared,
    };
    if !is_complete {
        return Err(SkillEvalError::InvalidConfiguration(format!(
            "artifact {:?} is missing its required base destination",
            artifact.name.0
        )));
    }

    Ok(artifact.required_destinations.clone())
}

pub(crate) fn inspect_trial(
    selector: &TrialSelector,
    store: &dyn RunStore,
) -> Result<TrialRecord, SkillEvalError> {
    store.find_trial(selector)
}

pub(crate) fn record_decision(
    run_id: &RunId,
    artifact: &ArtifactName,
    decision: Decision,
    assignments: Vec<TierAssignment>,
    reason: Option<String>,
    store: &mut dyn RunStore,
    clock: &dyn Clock,
) -> Result<DecisionRecord, SkillEvalError> {
    let report = build_report(run_id, store)?;
    let artifact_report = report
        .artifacts
        .iter()
        .find(|candidate| candidate.artifact == *artifact)
        .ok_or_else(|| {
            SkillEvalError::NotFound(format!("artifact {:?} is not in the run", artifact.0))
        })?;

    if artifact_report.decision.is_some() {
        return Err(SkillEvalError::InvalidArguments(
            "artifact already has a recorded decision".to_owned(),
        ));
    }
    if artifact_report.status != ArtifactStatus::AwaitingDecision {
        return Err(SkillEvalError::InvalidArguments(
            "artifact must await a decision".to_owned(),
        ));
    }

    match decision {
        Decision::Accepted => {
            let boundary = artifact_report.boundary.as_ref().ok_or_else(|| {
                SkillEvalError::InvalidConfiguration(
                    "accepted decision requires a supported boundary".to_owned(),
                )
            })?;
            let required = artifact_report
                .required_destinations
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            let supplied = assignments
                .iter()
                .map(|assignment| assignment.destination.clone())
                .collect::<BTreeSet<_>>();
            if supplied.len() != assignments.len() || supplied != required {
                return Err(SkillEvalError::InvalidArguments(
                    "accepted decision requires exactly the artifact tier destinations".to_owned(),
                ));
            }
            if assignments
                .iter()
                .any(|assignment| assignment.tier != boundary.accepted.tier)
            {
                return Err(SkillEvalError::InvalidArguments(
                    "accepted tier assignments must match the supported boundary".to_owned(),
                ));
            }
        }
        Decision::Rejected => {
            if !assignments.is_empty() {
                return Err(SkillEvalError::InvalidArguments(
                    "rejected decision cannot include tier assignments".to_owned(),
                ));
            }
            if reason.as_ref().is_none_or(|value| value.trim().is_empty()) {
                return Err(SkillEvalError::InvalidArguments(
                    "rejected decision requires a non-empty owner reason".to_owned(),
                ));
            }
        }
    }

    let decided_at = clock.now();
    let record = DecisionRecord {
        artifact: artifact.clone(),
        decision,
        assignments,
        reason,
        decided_at: decided_at.clone(),
    };
    store.append(
        run_id,
        &RunEvent::DecisionRecorded {
            at: decided_at,
            decision: record.clone(),
        },
    )?;
    Ok(record)
}

pub(crate) fn routing_decision(
    report: &QualificationReport,
    artifact: &ArtifactName,
) -> Result<Option<SkillRoutingDecision>, SkillEvalError> {
    let artifact_report = report
        .artifacts
        .iter()
        .find(|candidate| candidate.artifact == *artifact)
        .ok_or_else(|| {
            SkillEvalError::NotFound(format!("artifact {:?} is not in the report", artifact.0))
        })?;
    if artifact_report.kind != ArtifactKind::Skill {
        return Ok(None);
    }
    let Some(decision) = artifact_report
        .decision
        .as_ref()
        .filter(|decision| decision.decision == Decision::Accepted)
    else {
        return Ok(None);
    };
    let targets = decision
        .assignments
        .iter()
        .filter(|assignment| assignment.destination == TierDestination::SkillTarget)
        .collect::<Vec<_>>();
    let Some(target) = targets.first() else {
        return Ok(None);
    };
    if targets.len() != 1 {
        return Err(SkillEvalError::InvalidConfiguration(
            "accepted skill decision has duplicate target assignments".to_owned(),
        ));
    }
    let boundary = artifact_report.boundary.as_ref().ok_or_else(|| {
        SkillEvalError::InvalidConfiguration(
            "accepted skill decision has no supported boundary".to_owned(),
        )
    })?;
    if target.tier != boundary.accepted.tier {
        return Err(SkillEvalError::InvalidConfiguration(
            "accepted skill target differs from the supported boundary".to_owned(),
        ));
    }

    Ok(Some(SkillRoutingDecision {
        artifact: artifact.clone(),
        target_tier: target.tier,
        parent_responsibilities: vec![
            ParentResponsibility::HumanDecision,
            ParentResponsibility::IrreversibleAction,
            ParentResponsibility::FinalVerification,
        ],
    }))
}

pub(crate) fn evaluate_publication_gate(
    change: &ArtifactChange,
    report: &QualificationReport,
) -> Result<PublicationGate, SkillEvalError> {
    crate::publication::evaluate_publication_gate(change, report)
}

pub(crate) fn apply_tier_assignments(
    gate: &PublicationGate,
    artifact: &ArtifactDefinition,
    writer: &mut dyn TierWriter,
) -> Result<(), SkillEvalError> {
    if gate.status != PublicationStatus::Ready {
        return Err(SkillEvalError::InvalidArguments(
            "tier assignments require a ready publication gate".to_owned(),
        ));
    }
    if gate.change.artifact != artifact.name
        || gate.change.kind != artifact.kind
        || gate.change.candidate_revision != artifact.revision
        || gate.change.own_eval.artifact_revision != artifact.revision
    {
        return Err(SkillEvalError::InvalidConfiguration(
            "publication gate identity differs from the loaded artifact".to_owned(),
        ));
    }

    let required = report_destinations(artifact)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let supplied = gate
        .assignments
        .iter()
        .map(|assignment| assignment.destination.clone())
        .collect::<BTreeSet<_>>();
    if supplied.len() != gate.assignments.len() || supplied != required {
        return Err(SkillEvalError::InvalidArguments(
            "ready gate requires exactly the artifact tier destinations".to_owned(),
        ));
    }
    if gate
        .assignments
        .iter()
        .map(|assignment| assignment.tier)
        .collect::<BTreeSet<_>>()
        .len()
        != 1
    {
        return Err(SkillEvalError::InvalidArguments(
            "ready gate tier assignments must use one accepted tier".to_owned(),
        ));
    }

    writer.write(artifact, &gate.assignments)
}

pub(crate) fn prepare_audit_briefs(
    request: &AuditBriefRequest,
    runtime: &mut dyn QualificationRuntime,
) -> Result<Vec<AuditBrief>, SkillEvalError> {
    if request.artifact_roots.is_empty() {
        return Err(SkillEvalError::InvalidArguments(
            "audit requires at least one artifact".to_owned(),
        ));
    }
    crate::audit::reject_candidate_mutations_at_roots(&request.artifact_roots)?;

    let mut artifacts = request
        .artifact_roots
        .iter()
        .map(|root| runtime.load(root))
        .collect::<Result<Vec<_>, _>>()?;
    artifacts.sort_by(|left, right| left.name.cmp(&right.name));
    if artifacts
        .windows(2)
        .any(|pair| pair[0].name == pair[1].name)
    {
        return Err(SkillEvalError::InvalidConfiguration(
            "audit contains a duplicate artifact".to_owned(),
        ));
    }
    for artifact in &artifacts {
        validate_artifact(artifact)?;
        if !artifact
            .cases
            .iter()
            .any(|case| !case.is_holdout && case.execution.timeout_seconds > 0)
        {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "artifact {:?} has no executable non-holdout case",
                artifact.name.0
            )));
        }
    }
    crate::audit::reject_candidate_mutations(&artifacts)?;
    crate::audit::validate_output_root(&request.output_root)?;

    let audit_run_id = runtime.next()?;
    let judge_tier = runtime.configured_judge_tier()?;
    let mut drafts = Vec::with_capacity(artifacts.len());
    for mut artifact in artifacts {
        let tier = crate::audit::incumbent_tier(&artifact)?;
        artifact.cases.sort_by(|left, right| left.id.cmp(&right.id));
        let mut failures = Vec::new();

        for case in artifact
            .cases
            .iter()
            .filter(|case| !case.is_holdout && case.execution.timeout_seconds > 0)
        {
            let models = runtime.candidates(tier)?;
            let incumbent = models.first().ok_or_else(|| {
                SkillEvalError::InvalidConfiguration(format!(
                    "incumbent tier {tier:?} has an empty model route"
                ))
            })?;
            let harness = runtime.identity(&artifact, &case.execution)?;
            let key = TrialKey {
                artifact: artifact.name.clone(),
                tier,
                route_index: 0,
                case: case.id.clone(),
                attempt: 1,
            };
            let result = runtime.execute(
                &audit_run_id,
                &key,
                &artifact,
                case,
                incumbent,
                &harness,
                Some(case.execution.timeout_seconds),
            )?;
            if result.key != key || result.harness != harness || result.model != *incumbent {
                return Err(SkillEvalError::InvalidConfiguration(
                    "incumbent execution returned drifted trial identity".to_owned(),
                ));
            }
            let checks = runtime.verify(case, &result)?;
            let judge = runtime.judge(judge_tier, Some(&result.model))?;
            let judged = runtime.grade(
                &judge,
                &JudgeInput {
                    candidate: result,
                    expect: case.expect.clone(),
                    rubric_path: artifact.root.join("evals/rubric.md"),
                    checks,
                },
            )?;
            let modes = crate::audit::failure_modes(&judged.verdict);
            if !modes.is_empty() {
                failures.push((case.id.clone(), modes));
            }
        }

        drafts.push(crate::audit::AuditDraft {
            artifact: artifact.name,
            failures,
        });
    }

    crate::audit::write_audit_briefs(&request.output_root, drafts)
}

pub(crate) fn judge_prompt(
    request: &PromptJudgeRequest,
    runtime: &mut dyn QualificationRuntime,
) -> Result<PromptJudgeResult, SkillEvalError> {
    if request.prompt.trim().is_empty() {
        return Err(SkillEvalError::InvalidArguments(
            "judge prompt must not be empty".to_owned(),
        ));
    }
    if request.timeout_seconds == 0 {
        return Err(SkillEvalError::InvalidArguments(
            "judge timeout must be greater than zero".to_owned(),
        ));
    }

    let judge_tier = runtime.configured_judge_tier()?;
    let judge = runtime.judge(judge_tier, request.candidate_model.as_ref())?;
    if let Some(candidate) = &request.candidate_model
        && candidate.provider == judge.provider
        && candidate.model == judge.model
    {
        return Err(SkillEvalError::JudgeUnavailable {
            candidate: candidate.clone(),
            judge_tier,
        });
    }

    let result = runtime.grade_prompt(&judge, request)?;
    if let Some(candidate) = &request.candidate_model
        && candidate.provider == result.model.provider
        && candidate.model == result.model.model
    {
        return Err(SkillEvalError::JudgeUnavailable {
            candidate: candidate.clone(),
            judge_tier,
        });
    }
    Ok(result)
}

pub(crate) fn apply_event(state: &mut RunState, event: &RunEvent) -> Result<(), SkillEvalError> {
    if let RunEvent::RunStarted { configuration, .. } = event {
        if !state.run_id.0.is_empty() {
            return Err(transition_error("duplicate run_started event"));
        }
        if configuration.run_id.0.is_empty() {
            return Err(transition_error("run_started has an empty run identifier"));
        }

        let mut artifacts = BTreeMap::new();
        for artifact in &configuration.artifacts {
            if artifacts
                .insert(
                    artifact.name.clone(),
                    ArtifactQualificationState {
                        status: ArtifactStatus::Pending,
                        pending_candidates: Vec::new(),
                        tiers: Vec::new(),
                        boundary: None,
                        review_reason: None,
                    },
                )
                .is_some()
            {
                return Err(transition_error(
                    "run_started contains a duplicate artifact",
                ));
            }
        }

        state.run_id = configuration.run_id.clone();
        state.mode = configuration.mode;
        state.purpose = configuration.policy.purpose;
        state.status = RunStatus::Running;
        state.discoveries.clear();
        state.artifacts = artifacts;
        state.pause = None;
        state.decisions.clear();
        state.publication_gates.clear();
        return Ok(());
    }

    if state.run_id.0.is_empty() {
        return Err(transition_error(
            "run_started must occur before later events",
        ));
    }

    match event {
        RunEvent::RunStarted { .. } => unreachable!(),
        RunEvent::TrialStarted { .. } => require_running(state)?,
        RunEvent::CandidateExecuted { candidate, .. } => {
            require_running(state)?;
            let qualification = artifact_mut(state, &candidate.key.artifact)?;
            if !matches!(
                qualification.status,
                ArtifactStatus::Pending | ArtifactStatus::Running
            ) {
                return Err(transition_error(
                    "candidate checkpoint requires a pending or running artifact",
                ));
            }
            if qualification
                .pending_candidates
                .iter()
                .any(|pending| pending.key == candidate.key)
            {
                return Err(transition_error("duplicate candidate checkpoint"));
            }
            qualification.status = ArtifactStatus::Running;
            qualification.pending_candidates.push(candidate.clone());
        }
        RunEvent::TrialCompleted { record, .. } => {
            require_running(state)?;
            let qualification = artifact_mut(state, &record.key.artifact)?;
            let position = qualification
                .pending_candidates
                .iter()
                .position(|candidate| candidate_matches_record(candidate, record))
                .ok_or_else(|| {
                    transition_error("trial completion has no matching candidate checkpoint")
                })?;
            qualification.pending_candidates.remove(position);
        }
        RunEvent::PoolChildCompleted { artifact, .. } => {
            require_running(state)?;
            if state.purpose != QualificationPurpose::ModelPool {
                return Err(transition_error(
                    "pool child completion requires model-pool purpose",
                ));
            }
            if !state.decisions.is_empty() || !state.publication_gates.is_empty() {
                return Err(transition_error(
                    "pool child completion cannot follow a decision or publication gate",
                ));
            }
            let qualification = artifact_mut(state, artifact)?;
            if !matches!(
                qualification.status,
                ArtifactStatus::Pending | ArtifactStatus::Running
            ) || !qualification.pending_candidates.is_empty()
                || !qualification.tiers.is_empty()
                || qualification.boundary.is_some()
                || qualification.review_reason.is_some()
            {
                return Err(transition_error(
                    "pool child completion requires exact trials without artifact evidence",
                ));
            }
            qualification.status = ArtifactStatus::PoolCompleted;
            refresh_run_status(state);
        }
        RunEvent::TierEvaluated {
            artifact, evidence, ..
        } => {
            require_running(state)?;
            if evidence.expected_trials == 0
                || evidence.completed_trials != evidence.expected_trials
            {
                return Err(transition_error("tier evidence is incomplete"));
            }
            if !matches!(evidence.status, TierStatus::Accepted | TierStatus::Failed) {
                return Err(transition_error("tier evidence must be accepted or failed"));
            }
            let qualification = artifact_mut(state, artifact)?;
            if !matches!(
                qualification.status,
                ArtifactStatus::Pending | ArtifactStatus::Running
            ) {
                return Err(transition_error(
                    "tier evidence requires a pending or running artifact",
                ));
            }
            if qualification
                .tiers
                .iter()
                .any(|current| current.role == evidence.role && current.tier == evidence.tier)
            {
                return Err(transition_error("duplicate tier_evaluated event"));
            }
            if evidence.role == EvidenceRole::Reference
                && qualification
                    .tiers
                    .iter()
                    .any(|current| current.role == EvidenceRole::Reference)
            {
                return Err(transition_error("duplicate reference tier evidence"));
            }
            qualification.status = ArtifactStatus::Running;
            qualification.tiers.push(evidence.clone());
        }
        RunEvent::BoundaryFound {
            artifact, boundary, ..
        } => {
            require_running(state)?;
            let qualification = artifact_mut(state, artifact)?;
            if qualification.boundary.is_some() {
                return Err(transition_error("duplicate boundary_found event"));
            }
            if qualification.status != ArtifactStatus::Running {
                return Err(transition_error("boundary requires a running artifact"));
            }
            if boundary.accepted.role != EvidenceRole::Candidate
                || boundary.accepted.status != TierStatus::Accepted
                || !qualification.tiers.contains(&boundary.accepted)
            {
                return Err(transition_error(
                    "boundary accepted rung has no matching evidence",
                ));
            }
            if let Some(failing) = &boundary.failing
                && (failing.role != EvidenceRole::Candidate
                    || failing.status != TierStatus::Failed
                    || !qualification.tiers.contains(failing))
            {
                return Err(transition_error(
                    "boundary failing rung has no matching evidence",
                ));
            }
            qualification.boundary = Some(boundary.clone());
            qualification.status = ArtifactStatus::AwaitingDecision;
            refresh_run_status(state);
        }
        RunEvent::ReviewRequired {
            artifact, reason, ..
        } => {
            require_running(state)?;
            let qualification = artifact_mut(state, artifact)?;
            if qualification.status != ArtifactStatus::Running || qualification.tiers.is_empty() {
                return Err(transition_error(
                    "review requires evidence for a running artifact",
                ));
            }
            qualification.status = ArtifactStatus::NeedsReview;
            qualification.review_reason = Some(reason.clone());
            state.status = RunStatus::Failed;
        }
        RunEvent::RunPaused { reason, .. } => {
            require_running(state)?;
            if state.pause.is_some() {
                return Err(transition_error("duplicate run_paused event"));
            }
            for artifact in state.artifacts.values_mut() {
                if artifact.status == ArtifactStatus::Running {
                    artifact.status = ArtifactStatus::Paused;
                }
            }
            state.status = RunStatus::Paused;
            state.pause = Some(reason.clone());
        }
        RunEvent::RunResumed { .. } => {
            if state.status != RunStatus::Paused || state.pause.is_none() {
                return Err(transition_error("run_resumed requires a paused run"));
            }
            for artifact in state.artifacts.values_mut() {
                if artifact.status == ArtifactStatus::Paused {
                    artifact.status = ArtifactStatus::Running;
                }
            }
            state.status = RunStatus::Running;
            state.pause = None;
        }
        RunEvent::DecisionRecorded { decision, .. } => {
            if state.decisions.contains_key(&decision.artifact) {
                return Err(transition_error("duplicate decision_recorded event"));
            }
            let qualification = artifact_mut(state, &decision.artifact)?;
            if qualification.status != ArtifactStatus::AwaitingDecision
                || qualification.boundary.is_none()
            {
                return Err(transition_error(
                    "decision requires an artifact awaiting decision",
                ));
            }
            qualification.status = match decision.decision {
                Decision::Accepted => ArtifactStatus::Accepted,
                Decision::Rejected => ArtifactStatus::Rejected,
            };
            state
                .decisions
                .insert(decision.artifact.clone(), decision.clone());
            refresh_run_status(state);
        }
        RunEvent::PublicationGateEvaluated { gate, .. } => {
            let artifact = &gate.change.artifact;
            if let Some(previous) = state.publication_gates.get(artifact) {
                if previous.change != gate.change {
                    return Err(transition_error(
                        "publication gate change differs from its prior evaluation",
                    ));
                }
                if publication_status_rank(gate.status) <= publication_status_rank(previous.status)
                {
                    return Err(transition_error("publication gate moved backward"));
                }
            }
            let qualification = state
                .artifacts
                .get(artifact)
                .ok_or_else(|| transition_error("event names an unknown artifact"))?;
            let decision = state.decisions.get(artifact);
            match gate.status {
                PublicationStatus::AwaitingQualification
                    if qualification.boundary.is_some() || decision.is_some() =>
                {
                    return Err(transition_error(
                        "awaiting qualification gate conflicts with evidence",
                    ));
                }
                PublicationStatus::AwaitingDecision
                    if qualification.boundary.is_none() || decision.is_some() =>
                {
                    return Err(transition_error(
                        "awaiting decision gate requires undecided boundary evidence",
                    ));
                }
                PublicationStatus::Ready => match decision {
                    Some(record)
                        if qualification.boundary.is_some()
                            && record.decision == Decision::Accepted
                            && record.assignments == gate.assignments => {}
                    _ => {
                        return Err(transition_error(
                            "ready publication gate requires accepted boundary evidence",
                        ));
                    }
                },
                _ => {}
            }
            state
                .publication_gates
                .insert(artifact.clone(), gate.clone());
        }
        RunEvent::DiscoveryCompleted { artifacts, .. } => {
            require_running(state)?;
            if state.mode != crate::model::RunMode::DryRun
                && state.purpose != QualificationPurpose::ModelPool
            {
                return Err(transition_error(
                    "discovery completion requires dry-run or model-pool purpose",
                ));
            }
            if !state.discoveries.is_empty() {
                return Err(transition_error("duplicate discovery completion"));
            }
            if state.artifacts.values().any(|artifact| {
                artifact.status != ArtifactStatus::Pending
                    || !artifact.pending_candidates.is_empty()
                    || !artifact.tiers.is_empty()
                    || artifact.boundary.is_some()
            }) {
                return Err(transition_error(
                    "discovery completion cannot follow model evidence",
                ));
            }
            state.discoveries = artifacts.clone();
            if state.mode == crate::model::RunMode::DryRun {
                state.status = RunStatus::Discovered;
            }
        }
    }
    Ok(())
}

fn empty_run_state() -> RunState {
    RunState {
        run_id: RunId(String::new()),
        mode: crate::model::RunMode::Execute,
        purpose: QualificationPurpose::Artifact,
        status: RunStatus::Running,
        discoveries: Vec::new(),
        artifacts: BTreeMap::new(),
        pause: None,
        decisions: BTreeMap::new(),
        publication_gates: BTreeMap::new(),
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

fn add_usage(total: &mut TrialUsage, usage: &TrialUsage, line: u64) -> Result<(), SkillEvalError> {
    let sum = TrialUsage {
        input_tokens: checked_usage_u64(total.input_tokens, usage.input_tokens, line)?,
        output_tokens: checked_usage_u64(total.output_tokens, usage.output_tokens, line)?,
        cache_read_tokens: checked_usage_u64(
            total.cache_read_tokens,
            usage.cache_read_tokens,
            line,
        )?,
        cache_write_tokens: checked_usage_u64(
            total.cache_write_tokens,
            usage.cache_write_tokens,
            line,
        )?,
        turns: checked_usage_u32(total.turns, usage.turns, line)?,
        tool_calls: checked_usage_u32(total.tool_calls, usage.tool_calls, line)?,
        elapsed_milliseconds: checked_usage_u64(
            total.elapsed_milliseconds,
            usage.elapsed_milliseconds,
            line,
        )?,
        cost_millionths_of_dollar: checked_usage_u64(
            total.cost_millionths_of_dollar,
            usage.cost_millionths_of_dollar,
            line,
        )?,
    };
    *total = sum;
    Ok(())
}

fn checked_usage_u32(left: u32, right: u32, line: u64) -> Result<u32, SkillEvalError> {
    left.checked_add(right)
        .ok_or_else(|| invalid_event(line, "usage arithmetic overflow"))
}

fn checked_usage_u64(left: u64, right: u64, line: u64) -> Result<u64, SkillEvalError> {
    left.checked_add(right)
        .ok_or_else(|| invalid_event(line, "usage arithmetic overflow"))
}

fn validate_report_event(
    run_id: &RunId,
    event: &RunEvent,
    line: u64,
    artifact_kinds: &BTreeMap<ArtifactName, crate::model::ArtifactKind>,
    change: Option<&ArtifactChange>,
) -> Result<(), SkillEvalError> {
    match event {
        RunEvent::RunStarted { configuration, .. } if configuration.run_id != *run_id => Err(
            invalid_event(line, "run_started identity differs from the requested run"),
        ),
        RunEvent::PublicationGateEvaluated { gate, .. }
            if artifact_kinds
                .get(&gate.change.artifact)
                .is_some_and(|kind| *kind != gate.change.kind) =>
        {
            Err(invalid_event(
                line,
                "publication gate artifact kind differs from run configuration",
            ))
        }
        RunEvent::PublicationGateEvaluated { gate, .. }
            if change.is_none_or(|frozen| frozen != &gate.change) =>
        {
            Err(invalid_event(
                line,
                "publication gate change differs from run configuration",
            ))
        }
        _ => Ok(()),
    }
}

fn require_running(state: &RunState) -> Result<(), SkillEvalError> {
    if state.status != RunStatus::Running || state.pause.is_some() {
        return Err(transition_error("event requires a running run"));
    }
    Ok(())
}

fn artifact_mut<'a>(
    state: &'a mut RunState,
    artifact: &ArtifactName,
) -> Result<&'a mut ArtifactQualificationState, SkillEvalError> {
    state
        .artifacts
        .get_mut(artifact)
        .ok_or_else(|| transition_error("event names an unknown artifact"))
}

fn candidate_matches_record(
    candidate: &crate::model::CandidateArtifact,
    record: &TrialRecord,
) -> bool {
    candidate.key == record.key
        && candidate.model == record.model
        && candidate.harness == record.harness
        && candidate.artifact_path == record.artifact_path
        && candidate.transcript_path == record.transcript_path
        && candidate.usage == record.candidate_usage
}

fn publication_status_rank(status: PublicationStatus) -> u8 {
    match status {
        PublicationStatus::AwaitingQualification => 0,
        PublicationStatus::AwaitingDecision => 1,
        PublicationStatus::Ready | PublicationStatus::Blocked => 2,
    }
}

fn refresh_run_status(state: &mut RunState) {
    state.status = if state
        .artifacts
        .values()
        .any(|artifact| artifact.status == ArtifactStatus::NeedsReview)
    {
        RunStatus::Failed
    } else if state.artifacts.values().any(|artifact| {
        matches!(
            artifact.status,
            ArtifactStatus::Pending | ArtifactStatus::Running
        )
    }) {
        RunStatus::Running
    } else if state
        .artifacts
        .values()
        .any(|artifact| artifact.status == ArtifactStatus::AwaitingDecision)
    {
        RunStatus::AwaitingDecision
    } else {
        RunStatus::Completed
    };
}

fn event_at_line(error: SkillEvalError, line: u64) -> SkillEvalError {
    match error {
        SkillEvalError::InvalidEvent { line: 0, message } => invalid_event(line, &message),
        error => error,
    }
}

fn transition_error(message: &str) -> SkillEvalError {
    invalid_event(0, message)
}

fn invalid_event(line: u64, message: &str) -> SkillEvalError {
    SkillEvalError::InvalidEvent {
        line,
        message: message.to_string(),
    }
}

pub(crate) fn evaluate_tier(
    role: EvidenceRole,
    trials: &[TrialRecord],
    reference: Option<&TierEvidence>,
    policy: &QualificationPolicy,
) -> Result<TierEvidence, SkillEvalError> {
    crate::statistics::evaluate_tier(role, trials, reference, policy)
}

pub(crate) fn find_boundary(
    evidence: &[TierEvidence],
    policy: &QualificationPolicy,
) -> Result<Option<QualificationBoundary>, SkillEvalError> {
    crate::statistics::find_boundary(evidence, policy)
}

// TODO(AGNT-0032.T146): Build the guarded no-call frontier preview.
/// Validates one reviewed plan and projects its complete cumulative frontier.
///
/// The inputs are a plan path and read-only runtime. The output is a no-call preview.
///
/// # Errors
///
/// Returns an error for invalid suite, plan, capability, policy, identity, or source evidence.
pub(crate) fn preview_frontier(
    _plan_path: &Path,
    _runtime: &dyn FrontierRuntime,
) -> Result<FrontierPreviewReport, SkillEvalError> {
    unimplemented!()
}

// TODO(AGNT-0032.T147): Start and resume the exact cumulative frontier lifecycle.
/// Creates and advances one cumulative first-party frontier run.
///
/// The inputs are a frozen plan path, runtime, and progress sink. The output is saved state.
///
/// # Errors
///
/// Returns an error for invalid authority, storage, identity, candidate, judge, or infrastructure state.
pub(crate) fn start_frontier(
    _plan_path: &Path,
    _runtime: &mut dyn FrontierRuntime,
    _progress: &mut dyn FrontierProgressSink,
) -> Result<FrontierRunState, SkillEvalError> {
    unimplemented!()
}

/// Continues one saved cumulative frontier without repeating terminal work.
///
/// The inputs are a run identity, runtime, and progress sink. The output is updated saved state.
///
/// # Errors
///
/// Returns an error for resume drift, invalid state, quota, storage, or execution failure.
pub(crate) fn resume_frontier(
    _run_id: &FrontierRunId,
    _runtime: &mut dyn FrontierRuntime,
    _progress: &mut dyn FrontierProgressSink,
) -> Result<FrontierRunState, SkillEvalError> {
    unimplemented!()
}

// TODO(AGNT-0032.T148): Build the cross-tier report and exact inspection.
/// Builds one cross-tier report and optional saved-baseline comparison.
///
/// The inputs are a run identity, optional baseline path, and runtime. The output is the report.
///
/// # Errors
///
/// Returns an error for absent, malformed, incomplete, conflicting, or drifted evidence.
pub(crate) fn build_frontier_report(
    _run_id: &FrontierRunId,
    _baseline_path: Option<&Path>,
    _runtime: &dyn FrontierRuntime,
) -> Result<FrontierReport, SkillEvalError> {
    unimplemented!()
}

/// Loads one exact frontier trial or infrastructure event.
///
/// The inputs are an exact selector and runtime. The output is the selected evidence.
///
/// # Errors
///
/// Returns an error for an unsafe, ambiguous, absent, or inconsistent selector.
pub(crate) fn inspect_frontier(
    _selector: &crate::model::FrontierTrialSelector,
    _runtime: &dyn FrontierRuntime,
) -> Result<FrontierInspection, SkillEvalError> {
    unimplemented!()
}

// TODO(AGNT-0032.T149): Record rejection or commit acceptance with one ledger suffix.
/// Records an owner decision and appends an accepted baseline when requested.
///
/// The inputs are a decision request and runtime. The output is the terminal saved state.
///
/// # Errors
///
/// Returns an error for an invalid decision, nonterminal run, stale ledger, or failed atomic write.
pub(crate) fn record_frontier_decision(
    _request: &FrontierDecisionRequest,
    _runtime: &mut dyn FrontierRuntime,
) -> Result<FrontierRunState, SkillEvalError> {
    unimplemented!()
}

// TODO(AGNT-0032.T159): Publish only current accepted active routes.
/// Applies one accepted frontier's active routes to the owned routing map.
///
/// The inputs are an accepted run identity and runtime. The output records routes and byte change.
///
/// # Errors
///
/// Returns an error for unresolved, rejected, stale, drifted, unsafe, or unwritable evidence.
pub(crate) fn apply_frontier_baseline(
    _run_id: &FrontierRunId,
    _runtime: &mut dyn FrontierRuntime,
) -> Result<FrontierApplyReport, SkillEvalError> {
    unimplemented!()
}

#[cfg(test)]
include!("../tests/start.rs");
#[cfg(test)]
include!("../tests/resume.rs");
#[cfg(test)]
include!("../tests/decision.rs");
#[cfg(test)]
include!("../tests/report_destinations.rs");
#[cfg(test)]
include!("../tests/publication.rs");
#[cfg(test)]
include!("../tests/audit.rs");
#[cfg(test)]
include!("../tests/pool_start.rs");
#[cfg(test)]
include!("../tests/pool_resume.rs");
#[cfg(test)]
include!("../tests/pool_qualification.rs");
#[cfg(test)]
qualification_tests!();
#[cfg(test)]
resume_tests!();
#[cfg(test)]
decision_tests!();
#[cfg(test)]
report_destination_tests!();
#[cfg(test)]
publication_tests!();
#[cfg(test)]
audit_tests!();
#[cfg(test)]
pool_start_tests!();
#[cfg(test)]
pool_resume_tests!();
#[cfg(test)]
pool_qualification_tests!();

#[cfg(test)]
mod state {
    use std::cell::Cell;
    use std::path::PathBuf;

    use crate::model::{
        ArtifactChange, ArtifactDefinition, ArtifactDiscovery, ArtifactKind, ArtifactName,
        ArtifactStatus, CandidateArtifact, CaseDiscovery, CaseDrive, CaseId, ConfidenceInterval,
        Decision, DecisionRecord, EvidenceRole, HarnessIdentity, ModelIdentity, OwnEvalEvidence,
        PauseReason, PublicationGate, PublicationStatus, QualificationBoundary,
        QualificationPolicy, QualificationPurpose, RunConfiguration, RunEvent, RunId, RunMode,
        RunStatus, SkillEvalError, Tier, TierDestination, TierEvidence, TierStatus, Timestamp,
        TrialKey, TrialRecord, TrialSelector, TrialUsage, TrialVerdict,
    };
    use crate::ports::RunStore;

    use super::{apply_event, build_report, empty_run_state};

    #[test]
    fn replay_is_idempotent_and_preserves_boundary_first_report_state() {
        let mut record = trial_record(1, usage(10));
        record.judge_usage = usage(20);
        let evidence = evidence(
            Tier::T2,
            TierStatus::Accepted,
            record.candidate_usage.clone(),
        );
        let boundary = QualificationBoundary {
            failing: None,
            accepted: evidence.clone(),
        };
        let decision = decision(Decision::Accepted);
        let final_gate = gate(PublicationStatus::Ready);
        let events = vec![
            run_started(),
            RunEvent::PublicationGateEvaluated {
                at: timestamp(),
                gate: gate(PublicationStatus::AwaitingQualification),
            },
            trial_started(&record),
            candidate_executed(&record),
            RunEvent::RunPaused {
                at: timestamp(),
                reason: pause_reason(),
            },
            RunEvent::RunResumed { at: timestamp() },
            trial_completed(record.clone()),
            tier_evaluated(evidence),
            boundary_found(boundary.clone()),
            RunEvent::PublicationGateEvaluated {
                at: timestamp(),
                gate: gate(PublicationStatus::AwaitingDecision),
            },
            RunEvent::DecisionRecorded {
                at: timestamp(),
                decision: decision.clone(),
            },
            RunEvent::PublicationGateEvaluated {
                at: timestamp(),
                gate: final_gate.clone(),
            },
        ];
        let store = MemoryStore::new(events.clone());

        let first = build_report(&run_id(), &store).unwrap();
        let second = build_report(&run_id(), &store).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.status, RunStatus::Completed);
        assert_eq!(first.total_usage, combined_usage(30, 2));
        assert_eq!(first.artifacts[0].kind, ArtifactKind::Skill);
        assert_eq!(first.artifacts[0].boundary, Some(boundary));
        assert_eq!(first.artifacts[0].decision, Some(decision));
        assert_eq!(first.artifacts[0].publication_gate, Some(final_gate));
        assert_eq!(store.events, events);
        assert_eq!(store.append_count.get(), 0);
    }

    #[test]
    fn report_freezes_change_and_separates_reference_from_candidate_evidence() {
        let reference = reference_evidence(Tier::T4, usage(2));
        let candidate = evidence(Tier::T2, TierStatus::Accepted, usage(1));
        let store = MemoryStore::new(vec![
            run_started(),
            tier_evaluated(reference.clone()),
            tier_evaluated(candidate.clone()),
        ]);

        let report = build_report(&run_id(), &store).unwrap();

        assert_eq!(report.mode, RunMode::Execute);
        assert_eq!(report.change, Some(change()));
        assert_eq!(report.artifacts[0].reference, Some(reference));
        assert_eq!(report.artifacts[0].tiers, vec![candidate]);

        let mut changed_gate = gate(PublicationStatus::AwaitingQualification);
        changed_gate.change.candidate_revision = "later".to_string();
        let changed = MemoryStore::new(vec![
            run_started(),
            RunEvent::PublicationGateEvaluated {
                at: timestamp(),
                gate: changed_gate,
            },
        ]);
        assert_invalid_at(build_report(&run_id(), &changed), 2);
    }

    #[test]
    fn usage_overflow_fails() {
        let mut first = trial_record(1, zero_usage());
        first.judge_usage = usage(u64::MAX);
        let mut second = trial_record(2, zero_usage());
        second.judge_usage = usage(1);
        let store = MemoryStore::new(vec![
            run_started(),
            trial_started(&first),
            candidate_executed(&first),
            trial_completed(first),
            trial_started(&second),
            candidate_executed(&second),
            trial_completed(second),
        ]);

        assert_invalid_at(build_report(&run_id(), &store), 7);
        assert_eq!(store.append_count.get(), 0);
    }

    #[test]
    fn start_state_transition_is_required_and_unique() {
        let mut state = empty_run_state();
        assert_invalid(apply_event(
            &mut state,
            &tier_evaluated(evidence(Tier::T2, TierStatus::Accepted, usage(1))),
        ));

        apply_event(&mut state, &run_started()).unwrap();
        assert_eq!(state.status, RunStatus::Running);
        assert_invalid(apply_event(&mut state, &run_started()));

        let mut unknown = tier_evaluated(evidence(Tier::T2, TierStatus::Accepted, usage(1)));
        let RunEvent::TierEvaluated { artifact, .. } = &mut unknown else {
            unreachable!();
        };
        *artifact = ArtifactName("unknown".to_string());
        assert_invalid(apply_event(&mut state, &unknown));
    }

    #[test]
    fn pool_completion_is_a_terminal_aggregate_transition_without_artifact_evidence() {
        let completion = RunEvent::PoolChildCompleted {
            at: timestamp(),
            artifact: artifact_name(),
            tier: Tier::T2,
        };
        let mut artifact_state = empty_run_state_after_start();
        assert_invalid(apply_event(&mut artifact_state, &completion));

        let mut started = run_started();
        let RunEvent::RunStarted { configuration, .. } = &mut started else {
            unreachable!();
        };
        configuration.policy.purpose = QualificationPurpose::ModelPool;
        configuration.policy.candidate_tiers = vec![Tier::T2];
        configuration.policy.reference_tier = Tier::T1;
        let mut pool_state = empty_run_state();
        apply_event(&mut pool_state, &started).unwrap();
        apply_event(&mut pool_state, &completion).unwrap();
        assert_eq!(pool_state.status, RunStatus::Completed);
        assert_eq!(
            pool_state.artifacts[&artifact_name()].status,
            ArtifactStatus::PoolCompleted
        );
        assert!(pool_state.artifacts[&artifact_name()].tiers.is_empty());
        assert!(pool_state.artifacts[&artifact_name()].boundary.is_none());
        assert!(pool_state.decisions.is_empty());
        assert_invalid(apply_event(&mut pool_state, &completion));
    }

    #[test]
    fn dry_discovery_is_terminal_without_model_evidence() {
        let discoveries = vec![discovery()];
        let store = MemoryStore::new(vec![
            dry_run_started(),
            RunEvent::DiscoveryCompleted {
                at: timestamp(),
                artifacts: discoveries.clone(),
            },
        ]);

        let report = build_report(&run_id(), &store).unwrap();

        assert_eq!(report.mode, RunMode::DryRun);
        assert_eq!(report.change, Some(change()));
        assert_eq!(report.status, RunStatus::Discovered);
        assert_eq!(report.discoveries, discoveries);
        assert_eq!(report.total_usage, zero_usage());
        assert!(report.artifacts[0].reference.is_none());
        assert!(report.artifacts[0].tiers.is_empty());

        let mut state = empty_run_state();
        apply_event(&mut state, &dry_run_started()).unwrap();
        apply_event(
            &mut state,
            &RunEvent::DiscoveryCompleted {
                at: timestamp(),
                artifacts: vec![discovery()],
            },
        )
        .unwrap();
        assert_invalid(apply_event(
            &mut state,
            &candidate_executed(&trial_record(1, usage(1))),
        ));
    }

    #[test]
    fn start_rejects_duplicate_artifacts() {
        let mut event = run_started();
        let RunEvent::RunStarted { configuration, .. } = &mut event else {
            unreachable!();
        };
        configuration.artifacts.push(artifact_definition());

        assert_invalid(apply_event(&mut empty_run_state(), &event));
    }

    #[test]
    fn candidate_checkpoint_is_pending_until_judged_record_arrives() {
        let record = trial_record(1, usage(1));
        let candidate = candidate_from_record(&record);
        let mut state = empty_run_state_after_start();

        apply_event(&mut state, &trial_started(&record)).unwrap();
        apply_event(&mut state, &candidate_executed(&record)).unwrap();
        assert_eq!(
            state.artifacts[&artifact_name()].pending_candidates,
            vec![candidate]
        );

        apply_event(&mut state, &trial_completed(record)).unwrap();
        assert!(
            state.artifacts[&artifact_name()]
                .pending_candidates
                .is_empty()
        );
    }

    #[test]
    fn tier_state_is_unique() {
        let accepted = evidence(Tier::T2, TierStatus::Accepted, usage(1));
        let duplicate = MemoryStore::new(vec![
            run_started(),
            tier_evaluated(accepted.clone()),
            tier_evaluated(accepted),
        ]);
        assert_invalid_at(build_report(&run_id(), &duplicate), 3);
    }

    #[test]
    fn tier_state_rejects_incomplete_or_nonterminal_evidence() {
        let (mut state, mut accepted) = running_state();
        accepted.completed_trials = 0;
        assert_invalid(apply_event(&mut state, &tier_evaluated(accepted)));

        let (mut state, mut accepted) = running_state();
        accepted.status = TierStatus::Running;
        assert_invalid(apply_event(&mut state, &tier_evaluated(accepted)));
    }

    #[test]
    fn pause_and_resume_state_transitions_are_monotonic() {
        let (mut state, _) = running_state();
        assert_invalid(apply_event(
            &mut empty_run_state_after_start(),
            &RunEvent::RunResumed { at: timestamp() },
        ));

        apply_event(
            &mut state,
            &RunEvent::RunPaused {
                at: timestamp(),
                reason: pause_reason(),
            },
        )
        .unwrap();
        assert_eq!(state.status, RunStatus::Paused);
        assert_eq!(state.pause, Some(pause_reason()));
        assert_invalid(apply_event(
            &mut state,
            &RunEvent::RunPaused {
                at: timestamp(),
                reason: pause_reason(),
            },
        ));

        apply_event(&mut state, &RunEvent::RunResumed { at: timestamp() }).unwrap();
        assert_eq!(state.status, RunStatus::Running);
        assert_eq!(state.pause, None);
        assert_invalid(apply_event(
            &mut state,
            &RunEvent::RunResumed { at: timestamp() },
        ));
    }

    #[test]
    fn boundary_and_decision_state_transitions_require_evidence() {
        let (mut running, accepted) = running_state();
        let boundary = QualificationBoundary {
            failing: None,
            accepted: accepted.clone(),
        };
        assert_invalid(apply_event(&mut running, &boundary_found(boundary.clone())));
        assert_invalid(apply_event(
            &mut running,
            &RunEvent::DecisionRecorded {
                at: timestamp(),
                decision: decision(Decision::Accepted),
            },
        ));

        apply_event(&mut running, &tier_evaluated(accepted)).unwrap();
        apply_event(&mut running, &boundary_found(boundary.clone())).unwrap();
        assert_eq!(running.status, RunStatus::AwaitingDecision);
        assert_invalid(apply_event(&mut running, &boundary_found(boundary.clone())));

        let accepted_decision = decision(Decision::Accepted);
        apply_event(
            &mut running,
            &RunEvent::DecisionRecorded {
                at: timestamp(),
                decision: accepted_decision.clone(),
            },
        )
        .unwrap();
        assert_eq!(running.status, RunStatus::Completed);
        assert_invalid(apply_event(
            &mut running,
            &RunEvent::DecisionRecorded {
                at: timestamp(),
                decision: accepted_decision,
            },
        ));
    }

    #[test]
    fn failing_boundary_rung_must_have_matching_failed_evidence() {
        let (mut state, accepted) = state_with_evidence();
        let failing = evidence(Tier::T1, TierStatus::Failed, usage(1));
        assert_invalid(apply_event(
            &mut state,
            &boundary_found(QualificationBoundary {
                failing: Some(failing),
                accepted,
            }),
        ));
    }

    #[test]
    fn review_state_is_terminal_and_requires_evidence() {
        let (mut state, _) = running_state();
        assert_invalid(apply_event(
            &mut state,
            &RunEvent::ReviewRequired {
                at: timestamp(),
                artifact: artifact_name(),
                reason: "non-monotonic".to_string(),
            },
        ));

        let (mut state, _) = state_with_evidence();
        apply_event(
            &mut state,
            &RunEvent::ReviewRequired {
                at: timestamp(),
                artifact: artifact_name(),
                reason: "non-monotonic".to_string(),
            },
        )
        .unwrap();
        assert_eq!(state.status, RunStatus::Failed);
        assert_invalid(apply_event(
            &mut state,
            &tier_evaluated(evidence(Tier::T1, TierStatus::Failed, usage(1))),
        ));
    }

    #[test]
    fn paused_report_preserves_pending_candidate_for_resume() {
        let record = trial_record(1, usage(10));
        let candidate = candidate_from_record(&record);
        let store = MemoryStore::new(vec![
            run_started(),
            trial_started(&record),
            candidate_executed(&record),
            RunEvent::RunPaused {
                at: timestamp(),
                reason: pause_reason(),
            },
        ]);

        let report = build_report(&run_id(), &store).unwrap();

        assert_eq!(report.status, RunStatus::Paused);
        assert_eq!(report.pause, Some(pause_reason()));
        assert_eq!(report.total_usage, record.candidate_usage);
        assert_eq!(report.artifacts[0].pending_candidates, vec![candidate]);
        assert_eq!(
            report.artifacts[0].status,
            crate::model::ArtifactStatus::Paused
        );
    }

    #[test]
    fn rejected_decision_and_blocked_gate_are_preserved() {
        let (mut state, accepted) = state_with_evidence();
        apply_event(
            &mut state,
            &boundary_found(QualificationBoundary {
                failing: None,
                accepted,
            }),
        )
        .unwrap();
        apply_event(
            &mut state,
            &RunEvent::DecisionRecorded {
                at: timestamp(),
                decision: decision(Decision::Rejected),
            },
        )
        .unwrap();
        apply_event(
            &mut state,
            &RunEvent::PublicationGateEvaluated {
                at: timestamp(),
                gate: gate(PublicationStatus::Blocked),
            },
        )
        .unwrap();

        assert_eq!(state.status, RunStatus::Completed);
        assert_eq!(
            state.decisions[&artifact_name()].decision,
            Decision::Rejected
        );
        assert_eq!(
            state.publication_gates[&artifact_name()].status,
            PublicationStatus::Blocked
        );
    }

    #[test]
    fn publication_gate_state_requires_matching_readiness_evidence() {
        let mut state = empty_run_state_after_start();
        assert_invalid(apply_event(
            &mut state,
            &RunEvent::PublicationGateEvaluated {
                at: timestamp(),
                gate: gate(PublicationStatus::Ready),
            },
        ));
        apply_event(
            &mut state,
            &RunEvent::PublicationGateEvaluated {
                at: timestamp(),
                gate: gate(PublicationStatus::AwaitingQualification),
            },
        )
        .unwrap();
        assert_invalid(apply_event(
            &mut state,
            &RunEvent::PublicationGateEvaluated {
                at: timestamp(),
                gate: gate(PublicationStatus::AwaitingQualification),
            },
        ));

        let (mut waiting, accepted) = state_with_evidence();
        let boundary = QualificationBoundary {
            failing: None,
            accepted,
        };
        apply_event(&mut waiting, &boundary_found(boundary)).unwrap();
        apply_event(
            &mut waiting,
            &RunEvent::PublicationGateEvaluated {
                at: timestamp(),
                gate: gate(PublicationStatus::AwaitingDecision),
            },
        )
        .unwrap();

        let (mut completed, accepted) = state_with_evidence();
        apply_event(
            &mut completed,
            &boundary_found(QualificationBoundary {
                failing: None,
                accepted,
            }),
        )
        .unwrap();
        apply_event(
            &mut completed,
            &RunEvent::DecisionRecorded {
                at: timestamp(),
                decision: decision(Decision::Accepted),
            },
        )
        .unwrap();
        apply_event(
            &mut completed,
            &RunEvent::PublicationGateEvaluated {
                at: timestamp(),
                gate: gate(PublicationStatus::Ready),
            },
        )
        .unwrap();
    }

    #[test]
    fn report_requires_one_matching_start_and_artifact_kind() {
        let empty = MemoryStore::new(Vec::new());
        assert_invalid_at(build_report(&run_id(), &empty), 1);

        let wrong_run = MemoryStore::new(vec![run_started()]);
        assert_invalid_at(build_report(&RunId("different".to_string()), &wrong_run), 1);

        let mut wrong_kind_gate = gate(PublicationStatus::AwaitingQualification);
        wrong_kind_gate.change.kind = ArtifactKind::Agent;
        let wrong_kind = MemoryStore::new(vec![
            run_started(),
            RunEvent::PublicationGateEvaluated {
                at: timestamp(),
                gate: wrong_kind_gate,
            },
        ]);
        assert_invalid_at(build_report(&run_id(), &wrong_kind), 2);
    }

    fn running_state() -> (crate::model::RunState, TierEvidence) {
        (
            empty_run_state_after_start(),
            evidence(Tier::T2, TierStatus::Accepted, usage(1)),
        )
    }

    fn state_with_evidence() -> (crate::model::RunState, TierEvidence) {
        let (mut state, accepted) = running_state();
        apply_event(&mut state, &tier_evaluated(accepted.clone())).unwrap();
        (state, accepted)
    }

    fn empty_run_state_after_start() -> crate::model::RunState {
        let mut state = empty_run_state();
        apply_event(&mut state, &run_started()).unwrap();
        state
    }

    fn run_started() -> RunEvent {
        run_started_with_mode(RunMode::Execute)
    }

    fn dry_run_started() -> RunEvent {
        run_started_with_mode(RunMode::DryRun)
    }

    fn run_started_with_mode(mode: RunMode) -> RunEvent {
        RunEvent::RunStarted {
            at: timestamp(),
            configuration: RunConfiguration {
                run_id: run_id(),
                mode,
                artifacts: vec![artifact_definition()],
                change: Some(change()),
                policy: QualificationPolicy {
                    purpose: QualificationPurpose::Artifact,
                    candidate_tiers: vec![Tier::T2],
                    reference_tier: Tier::T4,
                    judge_tier: Tier::T5,
                    repeats_per_case: 1,
                    minimum_score: 7,
                    noninferiority_margin: 0.1,
                    confidence_level: 0.95,
                },
                qualification_routes: Default::default(),
                created_at: timestamp(),
            },
        }
    }

    fn artifact_definition() -> ArtifactDefinition {
        ArtifactDefinition {
            name: artifact_name(),
            kind: ArtifactKind::Skill,
            root: PathBuf::from("skills/example"),
            revision: "candidate".to_string(),
            required_destinations: vec![TierDestination::SkillMinimum],
            current_tiers: Vec::new(),
            cases: Vec::new(),
        }
    }

    fn trial_started(record: &TrialRecord) -> RunEvent {
        RunEvent::TrialStarted {
            at: timestamp(),
            key: record.key.clone(),
            models: vec![record.model.clone()],
            harness: record.harness.clone(),
        }
    }

    fn candidate_executed(record: &TrialRecord) -> RunEvent {
        RunEvent::CandidateExecuted {
            at: timestamp(),
            candidate: candidate_from_record(record),
        }
    }

    fn candidate_from_record(record: &TrialRecord) -> CandidateArtifact {
        CandidateArtifact {
            key: record.key.clone(),
            model: record.model.clone(),
            harness: record.harness.clone(),
            artifact_path: record.artifact_path.clone(),
            transcript_path: record.transcript_path.clone(),
            usage: record.candidate_usage.clone(),
        }
    }

    fn trial_completed(record: TrialRecord) -> RunEvent {
        RunEvent::TrialCompleted {
            at: timestamp(),
            record,
        }
    }

    fn tier_evaluated(evidence: TierEvidence) -> RunEvent {
        RunEvent::TierEvaluated {
            at: timestamp(),
            artifact: artifact_name(),
            evidence,
        }
    }

    fn boundary_found(boundary: QualificationBoundary) -> RunEvent {
        RunEvent::BoundaryFound {
            at: timestamp(),
            artifact: artifact_name(),
            boundary,
        }
    }

    fn trial_record(attempt: u16, usage: TrialUsage) -> TrialRecord {
        TrialRecord {
            key: TrialKey {
                artifact: artifact_name(),
                tier: Tier::T2,
                route_index: 0,
                case: CaseId("case-1".to_string()),
                attempt,
            },
            model: ModelIdentity {
                tier: Tier::T2,
                provider: "fixture".to_string(),
                model: "candidate".to_string(),
                thinking: "low".to_string(),
            },
            harness: HarnessIdentity {
                runner_version: "1".to_string(),
                pi_version: "1".to_string(),
                artifact_revision: "candidate".to_string(),
                tool_policy_digest: "policy".to_string(),
            },
            artifact_path: PathBuf::from("artifacts/case-1.txt"),
            transcript_path: PathBuf::from("transcripts/case-1.jsonl"),
            candidate_usage: usage,
            judge_model: ModelIdentity {
                tier: Tier::T5,
                provider: "judge".to_string(),
                model: "grader".to_string(),
                thinking: "high".to_string(),
            },
            judge_usage: zero_usage(),
            verdict: TrialVerdict {
                score: 8,
                is_catastrophic: false,
                failure_mode: None,
                checks: Vec::new(),
            },
        }
    }

    fn discovery() -> ArtifactDiscovery {
        ArtifactDiscovery {
            artifact: artifact_name(),
            kind: ArtifactKind::Skill,
            revision: "candidate".to_string(),
            cases: vec![CaseDiscovery {
                id: CaseId("case-1".to_string()),
                drive: CaseDrive::Response,
                is_holdout: false,
            }],
        }
    }

    fn evidence(tier: Tier, status: TierStatus, total_usage: TrialUsage) -> TierEvidence {
        evidence_with_role(EvidenceRole::Candidate, tier, status, total_usage)
    }

    fn reference_evidence(tier: Tier, total_usage: TrialUsage) -> TierEvidence {
        evidence_with_role(
            EvidenceRole::Reference,
            tier,
            TierStatus::Accepted,
            total_usage,
        )
    }

    fn evidence_with_role(
        role: EvidenceRole,
        tier: Tier,
        status: TierStatus,
        total_usage: TrialUsage,
    ) -> TierEvidence {
        let artifact_revision = match role {
            EvidenceRole::Reference => "incumbent",
            EvidenceRole::Candidate => "candidate",
        };
        TierEvidence {
            role,
            tier,
            model: ModelIdentity {
                tier,
                provider: "fixture".to_string(),
                model: artifact_revision.to_string(),
                thinking: "low".to_string(),
            },
            harnesses: vec![HarnessIdentity {
                runner_version: "1".to_string(),
                pi_version: "1".to_string(),
                artifact_revision: artifact_revision.to_string(),
                tool_policy_digest: "policy".to_string(),
            }],
            status,
            completed_trials: 1,
            expected_trials: 1,
            passed_trials: u32::from(status == TierStatus::Accepted),
            score: ConfidenceInterval {
                lower: 0.7,
                estimate: 0.8,
                upper: 0.9,
            },
            candidate_usage: total_usage.clone(),
            judge_usage: zero_usage(),
            total_usage,
        }
    }

    fn decision(decision: Decision) -> DecisionRecord {
        DecisionRecord {
            artifact: artifact_name(),
            decision,
            assignments: Vec::new(),
            reason: None,
            decided_at: timestamp(),
        }
    }

    fn gate(status: PublicationStatus) -> PublicationGate {
        PublicationGate {
            change: change(),
            status,
            assignments: Vec::new(),
            reason: None,
        }
    }

    fn change() -> ArtifactChange {
        ArtifactChange {
            artifact: artifact_name(),
            kind: ArtifactKind::Skill,
            incumbent_revision: "old".to_string(),
            candidate_revision: "new".to_string(),
            own_eval: OwnEvalEvidence {
                artifact_revision: "candidate".to_string(),
                path: PathBuf::from("evals/result.json"),
            },
        }
    }

    fn usage(input_tokens: u64) -> TrialUsage {
        TrialUsage {
            input_tokens,
            output_tokens: 1,
            cache_read_tokens: 1,
            cache_write_tokens: 1,
            turns: 1,
            tool_calls: 1,
            elapsed_milliseconds: 1,
            cost_millionths_of_dollar: 1,
        }
    }

    fn zero_usage() -> TrialUsage {
        combined_usage(0, 0)
    }

    fn combined_usage(input_tokens: u64, other_fields: u64) -> TrialUsage {
        TrialUsage {
            input_tokens,
            output_tokens: other_fields,
            cache_read_tokens: other_fields,
            cache_write_tokens: other_fields,
            turns: other_fields as u32,
            tool_calls: other_fields as u32,
            elapsed_milliseconds: other_fields,
            cost_millionths_of_dollar: other_fields,
        }
    }

    fn pause_reason() -> PauseReason {
        PauseReason::Infrastructure {
            message: "retry".to_string(),
        }
    }

    fn artifact_name() -> ArtifactName {
        ArtifactName("example".to_string())
    }

    fn run_id() -> RunId {
        RunId("run-1".to_string())
    }

    fn timestamp() -> Timestamp {
        Timestamp("2026-08-22T05:00:00-0400".to_string())
    }

    fn assert_invalid<T>(result: Result<T, SkillEvalError>) {
        assert!(matches!(result, Err(SkillEvalError::InvalidEvent { .. })));
    }

    fn assert_invalid_at<T>(result: Result<T, SkillEvalError>, expected_line: u64) {
        assert!(matches!(
            result,
            Err(SkillEvalError::InvalidEvent { line, .. }) if line == expected_line
        ));
    }

    struct MemoryStore {
        events: Vec<RunEvent>,
        append_count: Cell<u32>,
    }

    impl MemoryStore {
        fn new(events: Vec<RunEvent>) -> Self {
            Self {
                events,
                append_count: Cell::new(0),
            }
        }
    }

    impl RunStore for MemoryStore {
        fn append(&mut self, _run_id: &RunId, _event: &RunEvent) -> Result<(), SkillEvalError> {
            self.append_count.set(self.append_count.get() + 1);
            Ok(())
        }

        fn replay(
            &self,
            _run_id: &RunId,
            visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
        ) -> Result<(), SkillEvalError> {
            for event in &self.events {
                visitor(event.clone())?;
            }
            Ok(())
        }

        fn find_trial(&self, _selector: &TrialSelector) -> Result<TrialRecord, SkillEvalError> {
            Err(SkillEvalError::NotFound("not used".to_string()))
        }
    }
}
