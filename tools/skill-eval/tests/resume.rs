#[macro_export]
macro_rules! resume_tests {
    () => {
        mod resume {
            use std::cell::{Cell, RefCell};
            use std::collections::BTreeMap;
            use std::path::{Path, PathBuf};
            use std::rc::Rc;

            use $crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, CandidateArtifact, CaseDefinition,
                CaseDrive, CaseId, CheckResult, ExecutionDefinition, HarnessIdentity, JudgeInput,
                JudgeResult, ModelIdentity, PromptJudgeRequest, PromptJudgeResult,
                QualificationPolicy, QualificationPurpose, QualifyRequest, RunConfiguration,
                RunEvent, RunId, RunMode, RunStatus, SkillEvalError, Tier, TierAssignment,
                TierDestination, Timestamp, TrialKey, TrialRecord, TrialSelector, TrialUsage,
                TrialVerdict,
            };
            use $crate::ports::{
                ArtifactSource, CandidateRunner, Clock, HarnessResolver, Judge, ModelResolver,
                ProgressSink, QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
            };

            use super::{resume_qualification, start_qualification};

            #[test]
            fn quota_resume_is_idempotent_and_keeps_candidate_checkpoint() {
                let (mut runtime, mut progress) = runtime_and_progress();
                runtime.pause_tier = Some(Tier::T1);
                let paused = start_qualification(request(), &mut runtime, &mut progress).unwrap();
                assert_eq!(paused.status, RunStatus::Paused);
                let execute_calls = runtime.execute_calls;
                let candidate = paused.artifacts[0].pending_candidates[0].clone();
                progress.events.clear();

                let resumed =
                    resume_qualification(&paused.run_id, &mut runtime, &mut progress).unwrap();

                assert_eq!(resumed.status, RunStatus::AwaitingDecision);
                assert_eq!(runtime.execute_calls, execute_calls);
                assert_eq!(
                    progress.events.first(),
                    Some(&RunEvent::RunResumed { at: now() })
                );
                let completed = runtime
                    .events_for(&paused.run_id)
                    .into_iter()
                    .find_map(|event| match event {
                        RunEvent::TrialCompleted { record, .. } if record.key == candidate.key => {
                            Some(record)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(completed.artifact_path, candidate.artifact_path);
                assert_eq!(completed.transcript_path, candidate.transcript_path);
                assert_eq!(completed.model, candidate.model);
                assert_eq!(completed.harness, candidate.harness);
                assert_eq!(completed.candidate_usage, candidate.usage);

                let calls = runtime.all_model_calls();
                let event_count = runtime.events_for(&paused.run_id).len();
                progress.events.clear();
                let repeated =
                    resume_qualification(&paused.run_id, &mut runtime, &mut progress).unwrap();
                assert_eq!(repeated, resumed);
                assert_eq!(runtime.all_model_calls(), calls);
                assert_eq!(runtime.events_for(&paused.run_id).len(), event_count);
                assert!(progress.events.is_empty());
            }

            #[test]
            fn resume_does_not_repeat_a_completed_exact_route() {
                let (mut runtime, mut progress) = runtime_and_progress();
                runtime.routes.insert(
                    Tier::T1,
                    vec![model(Tier::T1, "first"), model(Tier::T1, "second")],
                );
                runtime.route_scores.insert((Tier::T1, 0), 6);
                runtime.pause_route = Some((Tier::T1, 1));
                let paused = start_qualification(request(), &mut runtime, &mut progress).unwrap();
                assert_eq!(paused.status, RunStatus::Paused);
                let first_key = TrialKey {
                    artifact: ArtifactName("artifact".to_owned()),
                    tier: Tier::T1,
                    route_index: 0,
                    case: CaseId("case-1".to_owned()),
                    attempt: 1,
                };
                assert_eq!(
                    event_count(&runtime, &paused.run_id, &first_key, "executed"),
                    1
                );
                assert_eq!(
                    event_count(&runtime, &paused.run_id, &first_key, "completed"),
                    1
                );
                let execute_calls = runtime.execute_calls;

                let resumed =
                    resume_qualification(&paused.run_id, &mut runtime, &mut progress).unwrap();

                assert_eq!(resumed.status, RunStatus::AwaitingDecision);
                assert_eq!(runtime.execute_calls, execute_calls);
                assert_eq!(
                    event_count(&runtime, &paused.run_id, &first_key, "executed"),
                    1
                );
                assert_eq!(
                    event_count(&runtime, &paused.run_id, &first_key, "completed"),
                    1
                );
            }

            #[test]
            fn resume_uses_frozen_unstarted_route_after_live_route_changes() {
                let (mut runtime, mut progress) = runtime_and_progress();
                let original_second = model(Tier::T1, "second");
                runtime.routes.insert(
                    Tier::T1,
                    vec![model(Tier::T1, "first"), original_second.clone()],
                );
                runtime.route_scores.insert((Tier::T1, 0), 6);
                runtime.pause_route = Some((Tier::T1, 0));
                let paused = start_qualification(request(), &mut runtime, &mut progress).unwrap();
                assert_eq!(paused.status, RunStatus::Paused);
                let frozen_routes = runtime
                    .events_for(&paused.run_id)
                    .into_iter()
                    .find_map(|event| match event {
                        RunEvent::RunStarted { configuration, .. } => {
                            Some(configuration.qualification_routes)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(
                    frozen_routes[&Tier::T1],
                    vec![model(Tier::T1, "first"), original_second.clone()]
                );
                let second_key = TrialKey {
                    artifact: ArtifactName("artifact".to_owned()),
                    tier: Tier::T1,
                    route_index: 1,
                    case: CaseId("case-1".to_owned()),
                    attempt: 1,
                };
                assert_eq!(event_count(&runtime, &paused.run_id, &second_key, "started"), 0);
                runtime.routes.get_mut(&Tier::T1).unwrap()[1] =
                    model(Tier::T1, "live-replacement");
                runtime.exact_requests.borrow_mut().clear();

                let resumed =
                    resume_qualification(&paused.run_id, &mut runtime, &mut progress).unwrap();

                assert_eq!(resumed.status, RunStatus::AwaitingDecision);
                assert!(runtime.exact_requests.borrow().contains(&original_second));
                let resumed_second = runtime
                    .events_for(&paused.run_id)
                    .into_iter()
                    .find_map(|event| match event {
                        RunEvent::CandidateExecuted { candidate, .. }
                            if candidate.key == second_key =>
                        {
                            Some(candidate.model)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(resumed_second, original_second);
            }

            #[test]
            fn legacy_artifact_resume_without_frozen_routes_fails_closed() {
                let (mut runtime, mut progress) = runtime_and_progress();
                runtime.pause_tier = Some(Tier::T1);
                let paused = start_qualification(request(), &mut runtime, &mut progress).unwrap();
                let events = runtime.runs.get_mut(&paused.run_id).unwrap();
                let configuration = events
                    .iter_mut()
                    .find_map(|event| match event {
                        RunEvent::RunStarted { configuration, .. } => Some(configuration),
                        _ => None,
                    })
                    .unwrap();
                configuration.qualification_routes.clear();
                let event_count = events.len();

                let error = resume_qualification(
                    &paused.run_id,
                    &mut runtime,
                    &mut FakeProgress::default(),
                )
                .unwrap_err();

                assert!(matches!(
                    error,
                    SkillEvalError::InvalidConfiguration(message)
                        if message.contains("start a new run")
                ));
                assert_eq!(runtime.events_for(&paused.run_id).len(), event_count);
            }

            #[test]
            fn resume_after_trial_checkpoint_executes_missing_candidate_once() {
                let (mut runtime, mut progress) = runtime_and_progress();
                runtime.is_fail_execute_once = true;
                let error =
                    start_qualification(request(), &mut runtime, &mut progress).unwrap_err();
                assert!(matches!(error, SkillEvalError::Process { .. }));
                let run_id = RunId("run-1".to_string());
                runtime
                    .append(
                        &run_id,
                        &RunEvent::RunPaused {
                            at: now(),
                            reason: $crate::model::PauseReason::Infrastructure {
                                message: "candidate process stopped".to_string(),
                            },
                        },
                    )
                    .unwrap();
                let calls_before_resume = runtime.execute_calls;
                progress.events.clear();

                let report = resume_qualification(&run_id, &mut runtime, &mut progress).unwrap();

                assert_eq!(report.status, RunStatus::AwaitingDecision);
                assert_eq!(runtime.execute_calls - calls_before_resume, 2);
                let reference = TrialKey {
                    artifact: ArtifactName("artifact".to_string()),
                    tier: Tier::T4,
                    route_index: 0,
                    case: CaseId("case-1".to_string()),
                    attempt: 1,
                };
                assert_eq!(event_count(&runtime, &run_id, &reference, "started"), 1);
                assert_eq!(event_count(&runtime, &run_id, &reference, "executed"), 1);
                assert_eq!(event_count(&runtime, &run_id, &reference, "completed"), 1);
                assert!(matches!(progress.events[0], RunEvent::RunResumed { .. }));
            }

            #[test]
            fn resume_rejects_artifact_route_harness_policy_and_checkpoint_drift() {
                for drift in ["artifact", "route", "harness", "policy", "checkpoint"] {
                    let (mut runtime, mut progress) = runtime_and_progress();
                    runtime.pause_tier = Some(Tier::T1);
                    let paused =
                        start_qualification(request(), &mut runtime, &mut progress).unwrap();
                    match drift {
                        "artifact" => {
                            runtime.artifact.revision = "changed".to_string();
                        }
                        "route" => {
                            runtime.is_exact_identity_drift = true;
                        }
                        "harness" => {
                            runtime.harness_version = "changed".to_string();
                        }
                        "policy" => {
                            runtime.judge_tier = Tier::T4;
                        }
                        "checkpoint" => {
                            let events = runtime.runs.get_mut(&paused.run_id).unwrap();
                            let candidate = events
                                .iter_mut()
                                .find_map(|event| match event {
                                    RunEvent::CandidateExecuted { candidate, .. }
                                        if candidate.key.tier == Tier::T1 =>
                                    {
                                        Some(candidate)
                                    }
                                    _ => None,
                                })
                                .unwrap();
                            candidate.model.provider = "outside-route".to_string();
                        }
                        _ => unreachable!(),
                    }
                    let event_count = runtime.events_for(&paused.run_id).len();
                    let error = resume_qualification(
                        &paused.run_id,
                        &mut runtime,
                        &mut FakeProgress::default(),
                    )
                    .unwrap_err();
                    assert!(
                        matches!(error, SkillEvalError::InvalidConfiguration(_)),
                        "unexpected {drift} error: {error:?}"
                    );
                    assert_eq!(runtime.events_for(&paused.run_id).len(), event_count);
                }
            }

            #[test]
            fn resume_rejects_a_run_that_is_not_paused() {
                let (mut runtime, _) = runtime_and_progress();
                let run_id = RunId("running".to_string());
                runtime
                    .append(
                        &run_id,
                        &RunEvent::RunStarted {
                            at: now(),
                            configuration: configuration(run_id.clone()),
                        },
                    )
                    .unwrap();

                let error =
                    resume_qualification(&run_id, &mut runtime, &mut FakeProgress::default())
                        .unwrap_err();

                assert!(matches!(error, SkillEvalError::InvalidConfiguration(_)));
                assert_eq!(runtime.all_model_calls(), 0);
            }

            #[test]
            fn completed_resume_performs_no_calls_and_returns_same_report() {
                let (mut runtime, mut progress) = runtime_and_progress();
                let report = start_qualification(request(), &mut runtime, &mut progress).unwrap();
                assert_eq!(report.status, RunStatus::AwaitingDecision);
                let calls = runtime.all_model_calls();
                let event_count = runtime.events_for(&report.run_id).len();

                let repeated = resume_qualification(
                    &report.run_id,
                    &mut runtime,
                    &mut FakeProgress::default(),
                )
                .unwrap();

                assert_eq!(repeated, report);
                assert_eq!(runtime.all_model_calls(), calls);
                assert_eq!(runtime.events_for(&report.run_id).len(), event_count);
            }

            fn event_count(
                runtime: &FakeRuntime,
                run_id: &RunId,
                key: &TrialKey,
                kind: &str,
            ) -> usize {
                runtime
                    .events_for(run_id)
                    .iter()
                    .filter(|event| match (kind, event) {
                        ("started", RunEvent::TrialStarted { key: current, .. }) => current == key,
                        ("executed", RunEvent::CandidateExecuted { candidate, .. }) => {
                            candidate.key == *key
                        }
                        ("completed", RunEvent::TrialCompleted { record, .. }) => {
                            record.key == *key
                        }
                        _ => false,
                    })
                    .count()
            }

            fn runtime_and_progress() -> (FakeRuntime, FakeProgress) {
                let persisted = Rc::new(RefCell::new(Vec::new()));
                (
                    FakeRuntime::new(persisted.clone()),
                    FakeProgress {
                        persisted: Some(persisted),
                        events: Vec::new(),
                        is_persisted_before_emit: true,
                    },
                )
            }

            fn request() -> QualifyRequest {
                QualifyRequest {
                    artifact_roots: vec![PathBuf::from("artifact")],
                    change: None,
                    policy: policy(),
                    is_dry_run: false,
                }
            }

            fn configuration(run_id: RunId) -> RunConfiguration {
                RunConfiguration {
                    run_id,
                    mode: RunMode::Execute,
                    artifacts: vec![artifact()],
                    change: None,
                    policy: policy(),
                    qualification_routes: Default::default(),
                    created_at: now(),
                }
            }

            fn policy() -> QualificationPolicy {
                QualificationPolicy {
                    purpose: QualificationPurpose::Artifact,
                    candidate_tiers: vec![Tier::T1],
                    reference_tier: Tier::T4,
                    judge_tier: Tier::T5,
                    repeats_per_case: 1,
                    minimum_score: 7,
                    noninferiority_margin: 0.1,
                    confidence_level: 0.95,
                }
            }

            fn artifact() -> ArtifactDefinition {
                ArtifactDefinition {
                    name: ArtifactName("artifact".to_string()),
                    kind: ArtifactKind::Skill,
                    root: PathBuf::from("artifact"),
                    revision: "revision-1".to_string(),
                    required_destinations: vec![TierDestination::SkillMinimum],
                    current_tiers: Vec::new(),
                    cases: vec![CaseDefinition {
                        id: CaseId("case-1".to_string()),
                        input: "input".to_string(),
                        expect: "expect".to_string(),
                        source: "fixture".to_string(),
                        is_holdout: false,
                        support_files: Vec::new(),
                        execution: ExecutionDefinition {
                            drive: CaseDrive::Response,
                            allowed_tools: Vec::new(),
                            timeout_seconds: 10,
                        },
                    }],
                }
            }

            fn model(tier: Tier, provider: &str) -> ModelIdentity {
                ModelIdentity {
                    tier,
                    provider: provider.to_string(),
                    model: format!("{provider}-{tier:?}"),
                    thinking: "low".to_string(),
                }
            }

            fn usage(value: u64) -> TrialUsage {
                TrialUsage {
                    input_tokens: value,
                    output_tokens: value,
                    cache_read_tokens: value,
                    cache_write_tokens: value,
                    turns: value as u32,
                    tool_calls: value as u32,
                    elapsed_milliseconds: value,
                    cost_millionths_of_dollar: value,
                }
            }

            fn now() -> Timestamp {
                Timestamp("2026-08-23T12:00:00-0400".to_string())
            }

            struct FakeRuntime {
                artifact: ArtifactDefinition,
                runs: BTreeMap<RunId, Vec<RunEvent>>,
                persisted: Rc<RefCell<Vec<RunEvent>>>,
                next_id: u64,
                resolver_calls: Cell<u32>,
                execute_calls: u32,
                verify_calls: u32,
                grade_calls: u32,
                pause_tier: Option<Tier>,
                pause_route: Option<(Tier, u16)>,
                route_scores: BTreeMap<(Tier, u16), u8>,
                routes: BTreeMap<Tier, Vec<ModelIdentity>>,
                exact_requests: RefCell<Vec<ModelIdentity>>,
                is_fail_execute_once: bool,
                is_exact_identity_drift: bool,
                route_provider: String,
                harness_version: String,
                judge_tier: Tier,
            }

            impl FakeRuntime {
                fn new(persisted: Rc<RefCell<Vec<RunEvent>>>) -> Self {
                    Self {
                        artifact: artifact(),
                        runs: BTreeMap::new(),
                        persisted,
                        next_id: 0,
                        resolver_calls: Cell::new(0),
                        execute_calls: 0,
                        verify_calls: 0,
                        grade_calls: 0,
                        pause_tier: None,
                        pause_route: None,
                        route_scores: BTreeMap::new(),
                        routes: BTreeMap::new(),
                        exact_requests: RefCell::new(Vec::new()),
                        is_fail_execute_once: false,
                        is_exact_identity_drift: false,
                        route_provider: "candidate".to_string(),
                        harness_version: "runner-1".to_string(),
                        judge_tier: Tier::T5,
                    }
                }

                fn events_for(&self, run_id: &RunId) -> Vec<RunEvent> {
                    self.runs.get(run_id).cloned().unwrap_or_default()
                }

                fn all_model_calls(&self) -> u32 {
                    self.resolver_calls.get()
                        + self.execute_calls
                        + self.verify_calls
                        + self.grade_calls
                }
            }

            impl ArtifactSource for FakeRuntime {
                fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
                    if root == Path::new("artifact") {
                        Ok(self.artifact.clone())
                    } else {
                        Err(SkillEvalError::NotFound(root.display().to_string()))
                    }
                }
            }

            impl ModelResolver for FakeRuntime {
                fn candidates(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    self.resolver_calls.set(self.resolver_calls.get() + 1);
                    Ok(vec![model(tier, &self.route_provider)])
                }

                fn qualification_routes(
                    &self,
                    tier: Tier,
                ) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    self.resolver_calls.set(self.resolver_calls.get() + 1);
                    Ok(self
                        .routes
                        .get(&tier)
                        .cloned()
                        .unwrap_or_else(|| vec![model(tier, &self.route_provider)]))
                }

                fn exact_candidate(
                    &self,
                    requested: &ModelIdentity,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    self.exact_requests.borrow_mut().push(requested.clone());
                    let mut effective = requested.clone();
                    if self.is_exact_identity_drift {
                        effective.provider = "changed".to_owned();
                    }
                    Ok(effective)
                }

                fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
                    self.resolver_calls.set(self.resolver_calls.get() + 1);
                    Ok(self.judge_tier)
                }

                fn judge(
                    &self,
                    judge_tier: Tier,
                    _candidate: Option<&ModelIdentity>,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    self.resolver_calls.set(self.resolver_calls.get() + 1);
                    Ok(model(judge_tier, "judge"))
                }
            }

            impl HarnessResolver for FakeRuntime {
                fn identity(
                    &self,
                    artifact: &ArtifactDefinition,
                    _execution: &ExecutionDefinition,
                ) -> Result<HarnessIdentity, SkillEvalError> {
                    Ok(HarnessIdentity {
                        runner_version: self.harness_version.clone(),
                        pi_version: "pi-1".to_string(),
                        artifact_revision: artifact.revision.clone(),
                        tool_policy_digest: "policy-1".to_string(),
                    })
                }
            }

            impl RunIdSource for FakeRuntime {
                fn next(&mut self) -> Result<RunId, SkillEvalError> {
                    self.next_id += 1;
                    Ok(RunId(format!("run-{}", self.next_id)))
                }
            }

            impl CandidateRunner for FakeRuntime {
                fn execute(
                    &mut self,
                    run_id: &RunId,
                    key: &TrialKey,
                    _artifact: &ArtifactDefinition,
                    case: &CaseDefinition,
                    model: &ModelIdentity,
                    harness: &HarnessIdentity,
                    candidate_timeout_seconds: Option<u32>,
                ) -> Result<CandidateArtifact, SkillEvalError> {
                    assert_eq!(
                        candidate_timeout_seconds,
                        Some(case.execution.timeout_seconds)
                    );
                    self.execute_calls += 1;
                    if self.is_fail_execute_once {
                        self.is_fail_execute_once = false;
                        return Err(SkillEvalError::Process {
                            program: "pi".to_string(),
                            exit_code: Some(1),
                            standard_error: "stopped".to_string(),
                        });
                    }
                    Ok(CandidateArtifact {
                        key: key.clone(),
                        model: model.clone(),
                        harness: harness.clone(),
                        artifact_path: PathBuf::from(&run_id.0)
                            .join(format!("artifacts/{:?}.txt", key.tier)),
                        transcript_path: PathBuf::from(&run_id.0)
                            .join(format!("transcripts/{:?}.jsonl", key.tier)),
                        usage: usage(2),
                    })
                }
            }

            impl Verifier for FakeRuntime {
                fn verify(
                    &mut self,
                    _case: &CaseDefinition,
                    _candidate: &CandidateArtifact,
                ) -> Result<Vec<CheckResult>, SkillEvalError> {
                    self.verify_calls += 1;
                    Ok(Vec::new())
                }
            }

            impl Judge for FakeRuntime {
                fn grade(
                    &mut self,
                    model: &ModelIdentity,
                    input: &JudgeInput,
                ) -> Result<JudgeResult, SkillEvalError> {
                    self.grade_calls += 1;
                    if self.pause_tier == Some(input.candidate.key.tier)
                        || self.pause_route
                            == Some((input.candidate.key.tier, input.candidate.key.route_index))
                    {
                        self.pause_tier = None;
                        self.pause_route = None;
                        return Err(SkillEvalError::Quota {
                            model: model.clone(),
                            reset_at: Some(Timestamp("later".to_string())),
                        });
                    }
                    Ok(JudgeResult {
                        verdict: TrialVerdict {
                            score: self
                                .route_scores
                                .get(&(input.candidate.key.tier, input.candidate.key.route_index))
                                .copied()
                                .unwrap_or(8),
                            is_catastrophic: false,
                            failure_mode: None,
                            checks: input.checks.clone(),
                        },
                        model: model.clone(),
                        usage: usage(3),
                    })
                }

                fn grade_prompt(
                    &mut self,
                    model: &ModelIdentity,
                    _request: &PromptJudgeRequest,
                ) -> Result<PromptJudgeResult, SkillEvalError> {
                    Ok(PromptJudgeResult {
                        model: model.clone(),
                        response: "ok".to_string(),
                        usage: usage(1),
                    })
                }
            }

            impl RunStore for FakeRuntime {
                fn append(
                    &mut self,
                    run_id: &RunId,
                    event: &RunEvent,
                ) -> Result<(), SkillEvalError> {
                    self.runs
                        .entry(run_id.clone())
                        .or_default()
                        .push(event.clone());
                    self.persisted.borrow_mut().push(event.clone());
                    Ok(())
                }

                fn replay(
                    &self,
                    run_id: &RunId,
                    visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
                ) -> Result<(), SkillEvalError> {
                    for event in self.events_for(run_id) {
                        visitor(event)?;
                    }
                    Ok(())
                }

                fn find_trial(
                    &self,
                    selector: &TrialSelector,
                ) -> Result<TrialRecord, SkillEvalError> {
                    self.events_for(&selector.run_id)
                        .into_iter()
                        .find_map(|event| match event {
                            RunEvent::TrialCompleted { record, .. }
                                if record.key.artifact == selector.artifact
                                    && record.key.tier == selector.tier
                                    && record.key.case == selector.case
                                    && record.key.attempt == selector.attempt =>
                            {
                                Some(record)
                            }
                            _ => None,
                        })
                        .ok_or_else(|| SkillEvalError::NotFound("trial".to_string()))
                }
            }

            impl Clock for FakeRuntime {
                fn now(&self) -> Timestamp {
                    now()
                }
            }

            impl TierWriter for FakeRuntime {
                fn write(
                    &mut self,
                    _artifact: &ArtifactDefinition,
                    _assignments: &[TierAssignment],
                ) -> Result<(), SkillEvalError> {
                    Ok(())
                }
            }

            impl QualificationRuntime for FakeRuntime {}

            #[derive(Default)]
            struct FakeProgress {
                persisted: Option<Rc<RefCell<Vec<RunEvent>>>>,
                events: Vec<RunEvent>,
                is_persisted_before_emit: bool,
            }

            impl ProgressSink for FakeProgress {
                fn emit(&mut self, event: &RunEvent) -> Result<(), SkillEvalError> {
                    if let Some(persisted) = &self.persisted
                        && persisted.borrow().last() != Some(event)
                    {
                        self.is_persisted_before_emit = false;
                    }
                    self.events.push(event.clone());
                    Ok(())
                }
            }
        }
    };
}
