#[macro_export]
macro_rules! qualification_tests {
    () => {
        mod qualification {
            use std::cell::{Cell, RefCell};
            use std::collections::BTreeMap;
            use std::path::{Component, Path, PathBuf};
            use std::rc::Rc;

            use $crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, CandidateArtifact, CaseDefinition,
                CaseDrive, CaseId, CheckResult, ExecutionDefinition, HarnessIdentity, JudgeInput,
                JudgeResult, ModelIdentity, PromptJudgeRequest, PromptJudgeResult,
                QualificationPolicy, QualificationPurpose, QualifyRequest, RunEvent, RunId,
                RunStatus, SkillEvalError,
                Tier, TierAssignment, TierDestination, Timestamp, TrialKey, TrialRecord,
                TrialSelector, TrialUsage, TrialVerdict,
            };
            use $crate::ports::{
                ArtifactSource, CandidateRunner, Clock, HarnessResolver, Judge, ModelResolver,
                ProgressSink, QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
            };

            use super::start_qualification;

            #[test]
            fn t2_failure_climbs_to_t3() {
                let (mut runtime, mut progress) = runtime_and_progress();
                runtime.scores.insert(Tier::T2, 6);
                let mut request = request(vec![Tier::T2, Tier::T3], false);
                request.policy.repeats_per_case = 2;

                let report = start_qualification(request, &mut runtime, &mut progress).unwrap();

                let boundary = report.artifacts[0].boundary.as_ref().unwrap();
                assert_eq!(boundary.failing.as_ref().unwrap().tier, Tier::T2);
                assert_eq!(boundary.accepted.tier, Tier::T3);
                assert_eq!(boundary.failing.as_ref().unwrap().completed_trials, 2);
                assert_eq!(boundary.accepted.completed_trials, 2);
                assert_eq!(candidate_tiers(&runtime), vec![Tier::T2, Tier::T3]);
                assert_persisted_before_progress(&runtime, &progress);
            }

            #[test]
            fn t2_pass_probes_t1() {
                let (mut runtime, mut progress) = runtime_and_progress();
                runtime.scores.insert(Tier::T1, 6);

                let report = start_qualification(
                    request(vec![Tier::T2, Tier::T1, Tier::T3], false),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                let boundary = report.artifacts[0].boundary.as_ref().unwrap();
                assert_eq!(boundary.failing.as_ref().unwrap().tier, Tier::T1);
                assert_eq!(boundary.accepted.tier, Tier::T2);
                assert_eq!(candidate_tiers(&runtime), vec![Tier::T2, Tier::T1]);
            }

            #[test]
            fn t1_is_base_boundary() {
                let (mut runtime, mut progress) = runtime_and_progress();

                let report = start_qualification(
                    request(vec![Tier::T1, Tier::T2], false),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                let boundary = report.artifacts[0].boundary.as_ref().unwrap();
                assert_eq!(boundary.accepted.tier, Tier::T1);
                assert_eq!(boundary.failing, None);
                assert_eq!(candidate_tiers(&runtime), vec![Tier::T1]);
            }

            #[test]
            fn dry_run_discovers_without_runtime_calls() {
                let (mut runtime, mut progress) = runtime_and_progress();
                runtime
                    .artifacts
                    .get_mut(Path::new("artifact"))
                    .unwrap()
                    .cases = vec![
                    case("response", CaseDrive::Response, false),
                    case(
                        "fixture",
                        CaseDrive::Fixture {
                            source: PathBuf::from("fixture"),
                            verify_commands: Vec::new(),
                        },
                        true,
                    ),
                ];

                let report =
                    start_qualification(request(vec![Tier::T1], true), &mut runtime, &mut progress)
                        .unwrap();

                assert_eq!(report.status, RunStatus::Discovered);
                assert_eq!(report.discoveries[0].revision, "candidate");
                assert_eq!(report.discoveries[0].cases.len(), 2);
                assert_eq!(report.discoveries[0].cases[1].is_holdout, true);
                assert_eq!(runtime.model_calls.get(), 0);
                assert_eq!(runtime.execute_calls, 0);
                assert_eq!(runtime.judge_calls, 0);
                assert_persisted_before_progress(&runtime, &progress);
            }

            #[test]
            fn judge_pause_preserves_candidate() {
                let (mut runtime, mut progress) = runtime_and_progress();
                runtime.pause_tier = Some(Tier::T2);

                let report = start_qualification(
                    request(vec![Tier::T2], false),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                assert_eq!(report.status, RunStatus::Paused);
                assert_eq!(report.artifacts[0].pending_candidates.len(), 1);
                assert_eq!(report.artifacts[0].pending_candidates[0].key.tier, Tier::T2);
                let events = runtime.events_for(&report.run_id);
                let tail = &events[events.len() - 3..];
                assert!(matches!(tail[0], RunEvent::TrialStarted { .. }));
                assert!(matches!(tail[1], RunEvent::CandidateExecuted { .. }));
                assert!(matches!(tail[2], RunEvent::RunPaused { .. }));
                assert_eq!(report.total_usage, usage(7));
            }

            #[test]
            fn run_ids_are_unique_path_components() {
                let (mut runtime, mut progress) = runtime_and_progress();

                let first =
                    start_qualification(request(vec![Tier::T1], true), &mut runtime, &mut progress)
                        .unwrap();
                let second =
                    start_qualification(request(vec![Tier::T1], true), &mut runtime, &mut progress)
                        .unwrap();

                assert_ne!(first.run_id, second.run_id);
                for run_id in [first.run_id, second.run_id] {
                    let mut components = Path::new(&run_id.0).components();
                    assert!(matches!(components.next(), Some(Component::Normal(_))));
                    assert!(components.next().is_none());
                }
            }

            #[test]
            fn only_model_pool_purpose_allows_a_same_tier_external_t5_judge() {
                let (mut artifact_runtime, mut artifact_progress) = runtime_and_progress();
                let artifact_result = start_qualification(
                    request(vec![Tier::T5], false),
                    &mut artifact_runtime,
                    &mut artifact_progress,
                );
                assert!(matches!(
                    artifact_result,
                    Err(SkillEvalError::InvalidConfiguration(_))
                ));
                assert_eq!(artifact_runtime.model_calls.get(), 0);

                let (mut pool_runtime, mut pool_progress) = runtime_and_progress();
                let mut pool_request = request(vec![Tier::T5], false);
                pool_request.policy.purpose = QualificationPurpose::ModelPool;
                let report = start_qualification(
                    pool_request,
                    &mut pool_runtime,
                    &mut pool_progress,
                )
                .unwrap();

                assert_eq!(candidate_tiers(&pool_runtime), vec![Tier::T5]);
                let t5_trial = pool_runtime
                    .events_for(&report.run_id)
                    .into_iter()
                    .find_map(|event| match event {
                        RunEvent::TrialCompleted { record, .. } if record.key.tier == Tier::T5 => {
                            Some(record)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(t5_trial.model.tier, Tier::T5);
                assert_eq!(t5_trial.judge_model.tier, Tier::T5);
                assert_ne!(t5_trial.model.provider, t5_trial.judge_model.provider);
            }

            #[test]
            fn model_pool_purpose_still_rejects_exact_self_grading() {
                let (mut runtime, mut progress) = runtime_and_progress();
                runtime.judge_provider = "candidate".to_owned();
                let mut request = request(vec![Tier::T5], false);
                request.policy.purpose = QualificationPurpose::ModelPool;

                let result = start_qualification(request, &mut runtime, &mut progress);

                assert!(matches!(
                    result,
                    Err(SkillEvalError::InvalidConfiguration(message))
                        if message.contains("resolved to the candidate model")
                ));
            }

            #[test]
            fn candidate_and_judge_usage_are_separate() {
                let (mut runtime, mut progress) = runtime_and_progress();

                let report = start_qualification(
                    request(vec![Tier::T1], false),
                    &mut runtime,
                    &mut progress,
                )
                .unwrap();

                let candidate = &report.artifacts[0].tiers[0];
                assert_eq!(candidate.candidate_usage, usage(2));
                assert_eq!(candidate.judge_usage, usage(3));
                assert_eq!(candidate.total_usage, usage(5));
                assert_eq!(candidate.model.provider, "candidate");
                let completed = runtime
                    .events_for(&report.run_id)
                    .into_iter()
                    .find_map(|event| match event {
                        RunEvent::TrialCompleted { record, .. } if record.key.tier == Tier::T1 => {
                            Some(record)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(completed.candidate_usage, usage(2));
                assert_eq!(completed.judge_usage, usage(3));
                assert_eq!(completed.judge_model.provider, "judge");
                assert_eq!(report.total_usage, usage(10));
            }

            fn candidate_tiers(runtime: &FakeRuntime) -> Vec<Tier> {
                runtime
                    .all_events()
                    .into_iter()
                    .filter_map(|event| match event {
                        RunEvent::TierEvaluated { evidence, .. }
                            if evidence.role == $crate::model::EvidenceRole::Candidate =>
                        {
                            Some(evidence.tier)
                        }
                        _ => None,
                    })
                    .collect()
            }

            fn assert_persisted_before_progress(runtime: &FakeRuntime, progress: &FakeProgress) {
                assert_eq!(runtime.all_events(), progress.events);
                assert!(progress.is_persisted_before_emit);
            }

            fn runtime_and_progress() -> (FakeRuntime, FakeProgress) {
                let persisted = Rc::new(RefCell::new(Vec::new()));
                (
                    FakeRuntime::new(persisted.clone()),
                    FakeProgress {
                        persisted,
                        events: Vec::new(),
                        is_persisted_before_emit: true,
                    },
                )
            }

            fn request(candidate_tiers: Vec<Tier>, is_dry_run: bool) -> QualifyRequest {
                QualifyRequest {
                    artifact_roots: vec![PathBuf::from("artifact")],
                    change: None,
                    policy: QualificationPolicy {
                        purpose: QualificationPurpose::Artifact,
                        candidate_tiers,
                        reference_tier: Tier::T4,
                        judge_tier: Tier::T5,
                        repeats_per_case: 1,
                        minimum_score: 7,
                        noninferiority_margin: 0.1,
                        confidence_level: 0.95,
                    },
                    is_dry_run,
                }
            }

            fn artifact() -> ArtifactDefinition {
                ArtifactDefinition {
                    name: ArtifactName("artifact".to_string()),
                    kind: ArtifactKind::Skill,
                    root: PathBuf::from("artifact"),
                    revision: "candidate".to_string(),
                    required_destinations: vec![TierDestination::SkillMinimum],
                    current_tiers: Vec::new(),
                    cases: vec![case("case-1", CaseDrive::Response, false)],
                }
            }

            fn case(id: &str, drive: CaseDrive, is_holdout: bool) -> CaseDefinition {
                CaseDefinition {
                    id: CaseId(id.to_string()),
                    input: "input".to_string(),
                    expect: "expect".to_string(),
                    source: "fixture".to_string(),
                    is_holdout,
                    support_files: Vec::new(),
                    execution: ExecutionDefinition {
                        drive,
                        allowed_tools: Vec::new(),
                        timeout_seconds: 10,
                    },
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

            struct FakeRuntime {
                artifacts: BTreeMap<PathBuf, ArtifactDefinition>,
                scores: BTreeMap<Tier, u8>,
                runs: BTreeMap<RunId, Vec<RunEvent>>,
                persisted: Rc<RefCell<Vec<RunEvent>>>,
                next_id: u64,
                model_calls: Cell<u32>,
                execute_calls: u32,
                judge_calls: u32,
                pause_tier: Option<Tier>,
                judge_provider: String,
            }

            impl FakeRuntime {
                fn new(persisted: Rc<RefCell<Vec<RunEvent>>>) -> Self {
                    Self {
                        artifacts: BTreeMap::from([(PathBuf::from("artifact"), artifact())]),
                        scores: BTreeMap::new(),
                        runs: BTreeMap::new(),
                        persisted,
                        next_id: 0,
                        model_calls: Cell::new(0),
                        execute_calls: 0,
                        judge_calls: 0,
                        pause_tier: None,
                        judge_provider: "judge".to_owned(),
                    }
                }

                fn events_for(&self, run_id: &RunId) -> Vec<RunEvent> {
                    self.runs.get(run_id).cloned().unwrap_or_default()
                }

                fn all_events(&self) -> Vec<RunEvent> {
                    self.persisted.borrow().clone()
                }
            }

            impl ArtifactSource for FakeRuntime {
                fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
                    self.artifacts
                        .get(root)
                        .cloned()
                        .ok_or_else(|| SkillEvalError::NotFound(root.display().to_string()))
                }
            }

            impl ModelResolver for FakeRuntime {
                fn candidates(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    self.model_calls.set(self.model_calls.get() + 1);
                    Ok(vec![model(tier, "candidate"), model(tier, "fallback")])
                }

                fn qualification_routes(
                    &self,
                    tier: Tier,
                ) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    self.model_calls.set(self.model_calls.get() + 1);
                    Ok(vec![model(tier, "candidate")])
                }

                fn exact_candidate(
                    &self,
                    requested: &ModelIdentity,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    Ok(requested.clone())
                }

                fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
                    self.model_calls.set(self.model_calls.get() + 1);
                    Ok(Tier::T5)
                }

                fn judge(
                    &self,
                    judge_tier: Tier,
                    _candidate: Option<&ModelIdentity>,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    self.model_calls.set(self.model_calls.get() + 1);
                    Ok(model(judge_tier, &self.judge_provider))
                }
            }

            impl HarnessResolver for FakeRuntime {
                fn identity(
                    &self,
                    artifact: &ArtifactDefinition,
                    _execution: &ExecutionDefinition,
                ) -> Result<HarnessIdentity, SkillEvalError> {
                    Ok(HarnessIdentity {
                        runner_version: "runner-1".to_string(),
                        pi_version: "pi-1".to_string(),
                        artifact_revision: artifact.revision.clone(),
                        tool_policy_digest: "policy".to_string(),
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
                    Ok(Vec::new())
                }
            }

            impl Judge for FakeRuntime {
                fn grade(
                    &mut self,
                    model: &ModelIdentity,
                    input: &JudgeInput,
                ) -> Result<JudgeResult, SkillEvalError> {
                    self.judge_calls += 1;
                    if self.pause_tier == Some(input.candidate.key.tier) {
                        self.pause_tier = None;
                        return Err(SkillEvalError::Quota {
                            model: model.clone(),
                            reset_at: Some(Timestamp("later".to_string())),
                        });
                    }
                    Ok(JudgeResult {
                        verdict: TrialVerdict {
                            score: self
                                .scores
                                .get(&input.candidate.key.tier)
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
                    Timestamp("2026-08-22T22:00:00-0400".to_string())
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

            struct FakeProgress {
                persisted: Rc<RefCell<Vec<RunEvent>>>,
                events: Vec<RunEvent>,
                is_persisted_before_emit: bool,
            }

            impl ProgressSink for FakeProgress {
                fn emit(&mut self, event: &RunEvent) -> Result<(), SkillEvalError> {
                    if self.persisted.borrow().last() != Some(event) {
                        self.is_persisted_before_emit = false;
                    }
                    self.events.push(event.clone());
                    Ok(())
                }
            }
        }
    };
}
