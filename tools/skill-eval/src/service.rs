use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use crate::model::{
    ArtifactChange, ArtifactDefinition, ArtifactDiscovery, ArtifactKind, ArtifactName,
    ArtifactQualificationState, ArtifactReport, ArtifactStatus, AuditBrief, AuditBriefRequest,
    CaseDiscovery, Decision, DecisionRecord, EvidenceRole, JudgeInput, ParentResponsibility,
    PauseReason, PoolQualifyRequest, PoolRunId, PoolRunState, PromptJudgeRequest,
    PromptJudgeResult, PublicationGate, PublicationStatus, QualificationBoundary,
    QualificationPolicy, QualificationReport, QualifyRequest, RunConfiguration, RunEvent, RunId,
    RunMode, RunState, RunStatus, SkillEvalError, SkillRoutingDecision, Tier, TierAssignment,
    TierDestination, TierEvidence, TierStatus, TrialKey, TrialRecord, TrialSelector, TrialUsage,
};
use crate::ports::{
    Clock, PoolProgressSink, PoolRuntime, ProgressSink, QualificationRuntime, RunStore, TierWriter,
};

// TODO(AGNT-0032.T88): Persist preallocated child identities before the first exact model call.
// TODO(AGNT-0032.T90): Advance passing calibration entrants through full qualification and ranking.
pub(crate) fn start_pool_qualification(
    _request: PoolQualifyRequest,
    _runtime: &mut dyn PoolRuntime,
    _progress: &mut dyn PoolProgressSink,
) -> Result<PoolRunState, SkillEvalError> {
    unimplemented!("AGNT-0032.T88")
}

// TODO(AGNT-0032.T89): Resume the first incomplete preallocated child without duplicate work.
pub(crate) fn resume_pool_qualification(
    _run_id: &PoolRunId,
    _runtime: &mut dyn PoolRuntime,
    _progress: &mut dyn PoolProgressSink,
) -> Result<PoolRunState, SkillEvalError> {
    unimplemented!("AGNT-0032.T89")
}

// TODO(AGNT-0032.T91): Return the complete pool state for command rendering.
pub(crate) fn build_pool_report(
    _run_id: &PoolRunId,
    _store: &dyn crate::ports::PoolStore,
) -> Result<PoolRunState, SkillEvalError> {
    unimplemented!("AGNT-0032.T91")
}

pub(crate) fn start_qualification_with_run_id(
    _run_id: RunId,
    _request: QualifyRequest,
    _runtime: &mut dyn QualificationRuntime,
    _progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    unimplemented!("AGNT-0032.T88")
}

pub(crate) fn start_qualification(
    request: QualifyRequest,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    validate_start_request(&request)?;

    let run_id = runtime.next()?;
    validate_run_id(&run_id)?;

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

    let created_at = runtime.now();
    let mode = if request.is_dry_run {
        RunMode::DryRun
    } else {
        RunMode::Execute
    };
    let configuration = RunConfiguration {
        run_id: run_id.clone(),
        mode,
        artifacts: artifacts.clone(),
        change: request.change.clone(),
        policy: request.policy.clone(),
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
        append_and_emit(runtime, progress, &run_id, event)?;
        return build_report(&run_id, runtime);
    }

    let configured_judge_tier = runtime.configured_judge_tier()?;
    if configured_judge_tier != request.policy.judge_tier {
        return Err(SkillEvalError::InvalidConfiguration(
            "configured judge tier differs from qualification policy".to_string(),
        ));
    }

    for artifact in &artifacts {
        let reference_trials = match run_tier_trials(
            &run_id,
            artifact,
            request.policy.reference_tier,
            &request.policy,
            runtime,
            progress,
        ) {
            Ok(trials) => trials,
            Err(StartTrialError::BeforeCheckpoint(error)) => return Err(error),
            Err(StartTrialError::AfterCheckpoint(error)) => {
                append_pause(runtime, progress, &run_id, error)?;
                return build_report(&run_id, runtime);
            }
        };
        let reference = evaluate_tier(
            EvidenceRole::Reference,
            &reference_trials,
            None,
            &request.policy,
        )?;
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
            let trials =
                match run_tier_trials(&run_id, artifact, *tier, &request.policy, runtime, progress)
                {
                    Ok(trials) => trials,
                    Err(StartTrialError::BeforeCheckpoint(error)) => return Err(error),
                    Err(StartTrialError::AfterCheckpoint(error)) => {
                        append_pause(runtime, progress, &run_id, error)?;
                        return build_report(&run_id, runtime);
                    }
                };
            let evidence = match evaluate_tier(
                EvidenceRole::Candidate,
                &trials,
                Some(&reference),
                &request.policy,
            ) {
                Ok(evidence) => evidence,
                Err(_) => {
                    append_review(
                        runtime,
                        progress,
                        &run_id,
                        &artifact.name,
                        "candidate evidence is unsupported",
                    )?;
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

enum StartTrialError {
    BeforeCheckpoint(SkillEvalError),
    AfterCheckpoint(SkillEvalError),
}

fn run_tier_trials(
    run_id: &RunId,
    artifact: &ArtifactDefinition,
    tier: Tier,
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
                case: case.id.clone(),
                attempt,
            };
            let models = runtime
                .candidates(tier)
                .map_err(StartTrialError::BeforeCheckpoint)?;
            let primary = models.first().cloned().ok_or_else(|| {
                StartTrialError::BeforeCheckpoint(SkillEvalError::InvalidConfiguration(format!(
                    "tier {tier:?} has an empty model route"
                )))
            })?;
            let at = runtime.now();
            append_and_emit(
                runtime,
                progress,
                run_id,
                RunEvent::TrialStarted {
                    at,
                    key: key.clone(),
                    models: models.clone(),
                    harness: harness.clone(),
                },
            )
            .map_err(StartTrialError::BeforeCheckpoint)?;

            let candidate = runtime
                .execute(run_id, &key, artifact, case, &primary, &harness)
                .map_err(StartTrialError::BeforeCheckpoint)?;
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
    if tiers.len() != request.policy.candidate_tiers.len()
        || tiers.contains(&request.policy.reference_tier)
        || request.policy.judge_tier <= request.policy.reference_tier
        || tiers.iter().any(|tier| request.policy.judge_tier <= *tier)
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
    if runtime.configured_judge_tier()? != configuration.policy.judge_tier {
        return Err(resume_drift("qualification policy"));
    }

    let mut planned_keys = BTreeSet::new();
    let started_tiers = replay
        .started
        .keys()
        .map(|key| key.tier)
        .collect::<BTreeSet<_>>();
    let mut routes = BTreeMap::new();
    for tier in started_tiers {
        routes.insert(tier, runtime.candidates(tier)?);
    }

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
                for attempt in 1..=configuration.policy.repeats_per_case {
                    let key = TrialKey {
                        artifact: artifact.name.clone(),
                        tier,
                        case: case.id.clone(),
                        attempt,
                    };
                    planned_keys.insert(key.clone());
                    if let Some(started) = replay.started.get(&key) {
                        if routes.get(&tier) != Some(&started.models) {
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
            let trials = match resume_tier_trials(
                &configuration.run_id,
                artifact,
                configuration.policy.reference_tier,
                &configuration.policy,
                replay,
                runtime,
                progress,
            ) {
                Ok(trials) => trials,
                Err(StartTrialError::BeforeCheckpoint(error)) => return Err(error),
                Err(StartTrialError::AfterCheckpoint(error)) => {
                    append_pause(runtime, progress, &configuration.run_id, error)?;
                    return build_report(&configuration.run_id, runtime);
                }
            };
            let evidence = evaluate_tier(
                EvidenceRole::Reference,
                &trials,
                None,
                &configuration.policy,
            )?;
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
            let trials = match resume_tier_trials(
                &configuration.run_id,
                artifact,
                *tier,
                &configuration.policy,
                replay,
                runtime,
                progress,
            ) {
                Ok(trials) => trials,
                Err(StartTrialError::BeforeCheckpoint(error)) => return Err(error),
                Err(StartTrialError::AfterCheckpoint(error)) => {
                    append_pause(runtime, progress, &configuration.run_id, error)?;
                    return build_report(&configuration.run_id, runtime);
                }
            };
            let evidence = match evaluate_tier(
                EvidenceRole::Candidate,
                &trials,
                Some(&reference),
                &configuration.policy,
            ) {
                Ok(evidence) => evidence,
                Err(_) => {
                    append_review(
                        runtime,
                        progress,
                        &configuration.run_id,
                        &artifact.name,
                        "candidate evidence is unsupported",
                    )?;
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

fn resume_tier_trials(
    run_id: &RunId,
    artifact: &ArtifactDefinition,
    tier: Tier,
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
                case: case.id.clone(),
                attempt,
            };
            if let Some(record) = replay.completed.get(&key) {
                trials.push(record.clone());
                continue;
            }

            let (models, harness) = if let Some(started) = replay.started.get(&key) {
                (started.models.clone(), started.harness.clone())
            } else {
                let harness = runtime
                    .identity(artifact, &case.execution)
                    .map_err(StartTrialError::BeforeCheckpoint)?;
                let models = runtime
                    .candidates(tier)
                    .map_err(StartTrialError::BeforeCheckpoint)?;
                if models.is_empty() {
                    return Err(StartTrialError::BeforeCheckpoint(
                        SkillEvalError::InvalidConfiguration(format!(
                            "tier {tier:?} has an empty model route"
                        )),
                    ));
                }
                let at = runtime.now();
                append_and_emit(
                    runtime,
                    progress,
                    run_id,
                    RunEvent::TrialStarted {
                        at,
                        key: key.clone(),
                        models: models.clone(),
                        harness: harness.clone(),
                    },
                )
                .map_err(StartTrialError::BeforeCheckpoint)?;
                (models, harness)
            };

            let candidate = if let Some(candidate) = replay.candidates.get(&key) {
                candidate.clone()
            } else {
                let primary = models.first().ok_or_else(|| {
                    StartTrialError::BeforeCheckpoint(SkillEvalError::InvalidConfiguration(
                        format!("tier {tier:?} has an empty model route"),
                    ))
                })?;
                let candidate = runtime
                    .execute(run_id, &key, artifact, case, primary, &harness)
                    .map_err(StartTrialError::BeforeCheckpoint)?;
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
                case: case.id.clone(),
                attempt: 1,
            };
            let result =
                runtime.execute(&audit_run_id, &key, &artifact, case, incumbent, &harness)?;
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
            if state.mode != crate::model::RunMode::DryRun {
                return Err(transition_error(
                    "discovery completion requires a dry-run mode",
                ));
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
            state.status = RunStatus::Discovered;
        }
    }
    Ok(())
}

fn empty_run_state() -> RunState {
    RunState {
        run_id: RunId(String::new()),
        mode: crate::model::RunMode::Execute,
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
mod state {
    use std::cell::Cell;
    use std::path::PathBuf;

    use crate::model::{
        ArtifactChange, ArtifactDefinition, ArtifactDiscovery, ArtifactKind, ArtifactName,
        CandidateArtifact, CaseDiscovery, CaseDrive, CaseId, ConfidenceInterval, Decision,
        DecisionRecord, EvidenceRole, HarnessIdentity, ModelIdentity, OwnEvalEvidence, PauseReason,
        PublicationGate, PublicationStatus, QualificationBoundary, QualificationPolicy,
        RunConfiguration, RunEvent, RunId, RunMode, RunStatus, SkillEvalError, Tier,
        TierDestination, TierEvidence, TierStatus, Timestamp, TrialKey, TrialRecord, TrialSelector,
        TrialUsage, TrialVerdict,
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
                    candidate_tiers: vec![Tier::T2],
                    reference_tier: Tier::T4,
                    judge_tier: Tier::T5,
                    repeats_per_case: 1,
                    minimum_score: 7,
                    noninferiority_margin: 0.1,
                    confidence_level: 0.95,
                },
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
