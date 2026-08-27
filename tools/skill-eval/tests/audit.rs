#[macro_export]
macro_rules! audit_tests {
    () => {
        mod audit_briefs {
            use std::cell::Cell;
            use std::fs;
            use std::os::unix::fs::symlink;
            use std::path::{Path, PathBuf};
            use std::sync::atomic::{AtomicU64, Ordering};

            use $crate::model::{
                ArtifactDefinition, ArtifactKind, ArtifactName, AuditBriefRequest,
                CandidateArtifact, CaseDefinition, CaseDrive, CaseId, CheckResult, CheckStatus,
                ExecutionDefinition, HarnessIdentity, JudgeInput, JudgeResult, ModelIdentity,
                PromptJudgeRequest, PromptJudgeResult, RunEvent, RunId, SkillEvalError, Tier,
                TierAssignment, TierDestination, Timestamp, TrialKey, TrialRecord, TrialSelector,
                TrialUsage, TrialVerdict,
            };
            use $crate::ports::{
                ArtifactSource, CandidateRunner, Clock, HarnessResolver, Judge, ModelResolver,
                QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
            };

            use super::prepare_audit_briefs;

            static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

            #[test]
            fn audit_fence_runs_only_blind_incumbent_cases() {
                let fixture = Fixture::new();
                let output = fixture.path.join("ignored/audits");
                let mut runtime = FakeRuntime::new(artifact(&fixture.path));

                let briefs = prepare_audit_briefs(
                    &AuditBriefRequest {
                        artifact_roots: vec![fixture.path.clone()],
                        output_root: output.clone(),
                    },
                    &mut runtime,
                )
                .unwrap();

                assert_eq!(runtime.executed, vec![CaseId("a-case".to_owned()), CaseId("b-case".to_owned())]);
                assert_eq!(runtime.identity_calls.get(), 2);
                assert_eq!(runtime.run_id_calls, 1);
                assert_eq!(runtime.execute_calls, 2);
                assert!(runtime.is_identity_fresh);
                assert_eq!(briefs.len(), 1);
                assert_eq!(briefs[0].failure_modes.len(), 1);
                assert_eq!(briefs[0].failure_modes[0].failure_mode, "missing gate");
                assert_eq!(briefs[0].failure_modes[0].count, 2);
                assert_eq!(briefs[0].reproductions.len(), 2);
                for path in &briefs[0].reproductions {
                    assert!(path.starts_with(fs::canonicalize(&output).unwrap()));
                }

                let emitted = read_tree(&output);
                for sentinel in [
                    "HELD_OUT_SENTINEL",
                    "PRIOR_VOTE_SENTINEL",
                    "CANDIDATE_TEXT_SENTINEL",
                    "PRIOR_GRADE_SENTINEL",
                    "HIDDEN_MODEL_SENTINEL",
                ] {
                    assert!(!emitted.contains(sentinel));
                }
                assert!(!emitted.contains("candidate artifact body"));
                assert!(!emitted.contains("judge-secret"));
            }

            #[test]
            fn existing_candidate_mutation_stops_before_runtime_work() {
                let fixture = Fixture::new();
                fs::write(fixture.path.join("candidate.md"), "CANDIDATE_TEXT_SENTINEL").unwrap();
                let output = fixture.path.join("ignored/audits");
                let mut runtime = FakeRuntime::new(artifact(&fixture.path));

                let result = prepare_audit_briefs(
                    &AuditBriefRequest {
                        artifact_roots: vec![fixture.path.clone()],
                        output_root: output.clone(),
                    },
                    &mut runtime,
                );

                assert!(matches!(result, Err(SkillEvalError::InvalidConfiguration(message)) if message.contains("candidate mutation")));
                assert_eq!(runtime.load_calls.get(), 0);
                assert_eq!(runtime.model_calls.get(), 0);
                assert_eq!(runtime.identity_calls.get(), 0);
                assert_eq!(runtime.execute_calls, 0);
                assert_eq!(runtime.judge_calls, 0);
                assert!(!output.exists());
            }

            #[test]
            fn candidate_mutation_directory_stops_before_runtime_work() {
                let fixture = Fixture::new();
                fs::create_dir_all(fixture.path.join("evals/candidate.md")).unwrap();
                let output = fixture.path.join("ignored/audits");
                let mut runtime = FakeRuntime::new(artifact(&fixture.path));

                let result = prepare_audit_briefs(
                    &AuditBriefRequest {
                        artifact_roots: vec![fixture.path.clone()],
                        output_root: output.clone(),
                    },
                    &mut runtime,
                );

                assert!(matches!(result, Err(SkillEvalError::InvalidConfiguration(message)) if message.contains("candidate mutation")));
                assert_runtime_is_unused(&runtime);
                assert!(!output.exists());
            }

            #[test]
            fn candidate_mutation_symlink_stops_before_runtime_work() {
                let fixture = Fixture::new();
                let sentinel = fixture.path.join("sentinel");
                fs::write(&sentinel, b"unchanged sentinel bytes").unwrap();
                symlink(&sentinel, fixture.path.join("candidate.md")).unwrap();
                let output = fixture.path.join("ignored/audits");
                let mut runtime = FakeRuntime::new(artifact(&fixture.path));

                let result = prepare_audit_briefs(
                    &AuditBriefRequest {
                        artifact_roots: vec![fixture.path.clone()],
                        output_root: output.clone(),
                    },
                    &mut runtime,
                );

                assert!(matches!(result, Err(SkillEvalError::InvalidConfiguration(message)) if message.contains("candidate mutation")));
                assert_runtime_is_unused(&runtime);
                assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged sentinel bytes");
                assert!(!output.exists());
            }

            #[test]
            fn post_load_candidate_check_rejects_root_normalization_drift() {
                let request_fixture = Fixture::new();
                let loaded_fixture = Fixture::new();
                fs::write(loaded_fixture.path.join("candidate.md"), b"mutation").unwrap();
                let output = request_fixture.path.join("ignored/audits");
                let mut runtime = FakeRuntime::new(artifact(&loaded_fixture.path));

                let result = prepare_audit_briefs(
                    &AuditBriefRequest {
                        artifact_roots: vec![request_fixture.path.clone()],
                        output_root: output.clone(),
                    },
                    &mut runtime,
                );

                assert!(matches!(result, Err(SkillEvalError::InvalidConfiguration(message)) if message.contains("candidate mutation")));
                assert_eq!(runtime.load_calls.get(), 1);
                assert_eq!(runtime.model_calls.get(), 0);
                assert_eq!(runtime.identity_calls.get(), 0);
                assert_eq!(runtime.execute_calls, 0);
                assert_eq!(runtime.judge_calls, 0);
                assert!(!output.exists());
            }

            #[test]
            fn escaping_output_stops_before_execution() {
                let fixture = Fixture::new();
                let mut runtime = FakeRuntime::new(artifact(&fixture.path));

                let sentinel = fixture.path.join("sentinel");
                fs::write(&sentinel, b"unchanged sentinel bytes").unwrap();
                let result = prepare_audit_briefs(
                    &AuditBriefRequest {
                        artifact_roots: vec![fixture.path.clone()],
                        output_root: PathBuf::from("safe/../escape"),
                    },
                    &mut runtime,
                );

                assert!(matches!(result, Err(SkillEvalError::InvalidArguments(_))));
                assert_eq!(runtime.execute_calls, 0);
                assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged sentinel bytes");
            }

            #[test]
            fn preexisting_artifact_directory_is_not_reused() {
                let fixture = Fixture::new();
                let output = fixture.path.join("ignored/audits");
                let artifact_root = output.join("artifact-0001");
                fs::create_dir_all(&artifact_root).unwrap();
                let sentinel = artifact_root.join("sentinel");
                fs::write(&sentinel, b"unchanged sentinel bytes").unwrap();
                let mut runtime = FakeRuntime::new(artifact(&fixture.path));

                let result = prepare_audit_briefs(
                    &AuditBriefRequest {
                        artifact_roots: vec![fixture.path.clone()],
                        output_root: output,
                    },
                    &mut runtime,
                );

                assert!(result.is_err());
                assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged sentinel bytes");
            }

            #[test]
            fn preexisting_artifact_file_is_not_overwritten() {
                let fixture = Fixture::new();
                let output = fixture.path.join("ignored/audits");
                fs::create_dir_all(&output).unwrap();
                let artifact_path = output.join("artifact-0001");
                fs::write(&artifact_path, b"unchanged sentinel bytes").unwrap();
                let mut runtime = FakeRuntime::new(artifact(&fixture.path));

                let result = prepare_audit_briefs(
                    &AuditBriefRequest {
                        artifact_roots: vec![fixture.path.clone()],
                        output_root: output,
                    },
                    &mut runtime,
                );

                assert!(result.is_err());
                assert_eq!(fs::read(&artifact_path).unwrap(), b"unchanged sentinel bytes");
            }

            #[test]
            fn preexisting_reproduction_directory_is_not_reused() {
                let fixture = Fixture::new();
                let output = fixture.path.join("ignored/audits");
                let reproduction_root = output.join("artifact-0001/reproductions");
                fs::create_dir_all(&reproduction_root).unwrap();
                let sentinel = reproduction_root.join("sentinel");
                fs::write(&sentinel, b"unchanged sentinel bytes").unwrap();
                let mut runtime = FakeRuntime::new(artifact(&fixture.path));

                let result = prepare_audit_briefs(
                    &AuditBriefRequest {
                        artifact_roots: vec![fixture.path.clone()],
                        output_root: output,
                    },
                    &mut runtime,
                );

                assert!(result.is_err());
                assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged sentinel bytes");
            }

            #[test]
            fn preexisting_brief_file_is_not_overwritten() {
                assert_output_file_collision("artifact-0001/brief.json");
            }

            #[test]
            fn preexisting_case_file_is_not_overwritten() {
                assert_output_file_collision("artifact-0001/reproductions/case-0001.json");
            }

            #[test]
            fn reproduction_output_does_not_follow_a_symbolic_link() {
                let fixture = Fixture::new();
                let output = fixture.path.join("ignored/audits");
                let reproductions = output.join("artifact-0001/reproductions");
                fs::create_dir_all(&reproductions).unwrap();
                let escaped = fixture.path.join("escaped.json");
                fs::write(&escaped, b"unchanged sentinel bytes").unwrap();
                symlink(&escaped, reproductions.join("case-0001.json")).unwrap();
                let mut runtime = FakeRuntime::new(artifact(&fixture.path));

                let result = prepare_audit_briefs(
                    &AuditBriefRequest {
                        artifact_roots: vec![fixture.path.clone()],
                        output_root: output,
                    },
                    &mut runtime,
                );

                assert!(result.is_err());
                assert_eq!(fs::read(&escaped).unwrap(), b"unchanged sentinel bytes");
            }

            fn assert_output_file_collision(relative: &str) {
                let fixture = Fixture::new();
                let output = fixture.path.join("ignored/audits");
                let collision = output.join(relative);
                fs::create_dir_all(collision.parent().unwrap()).unwrap();
                fs::write(&collision, b"unchanged sentinel bytes").unwrap();
                let mut runtime = FakeRuntime::new(artifact(&fixture.path));

                let result = prepare_audit_briefs(
                    &AuditBriefRequest {
                        artifact_roots: vec![fixture.path.clone()],
                        output_root: output,
                    },
                    &mut runtime,
                );

                assert!(result.is_err());
                assert_eq!(fs::read(&collision).unwrap(), b"unchanged sentinel bytes");
            }

            fn artifact(root: &Path) -> ArtifactDefinition {
                ArtifactDefinition {
                    name: ArtifactName("fixture-skill".to_owned()),
                    kind: ArtifactKind::Skill,
                    root: root.to_path_buf(),
                    revision: "incumbent-revision".to_owned(),
                    required_destinations: vec![TierDestination::SkillMinimum],
                    current_tiers: vec![TierAssignment {
                        destination: TierDestination::SkillMinimum,
                        tier: Tier::T3,
                    }],
                    cases: vec![
                        case("b-case", false),
                        case("held-out-secret", true),
                        non_executable_case(),
                        case("a-case", false),
                    ],
                }
            }

            fn case(id: &str, is_holdout: bool) -> CaseDefinition {
                CaseDefinition {
                    id: CaseId(id.to_owned()),
                    input: if is_holdout {
                        "HELD_OUT_SENTINEL".to_owned()
                    } else {
                        "authorized synthetic fixture".to_owned()
                    },
                    expect: "expected fixture result".to_owned(),
                    source: "synthetic".to_owned(),
                    is_holdout,
                    support_files: Vec::new(),
                    execution: ExecutionDefinition {
                        drive: CaseDrive::Response,
                        allowed_tools: Vec::new(),
                        timeout_seconds: 10,
                    },
                }
            }

            fn non_executable_case() -> CaseDefinition {
                let mut definition = case("non-executable", false);
                definition.execution.timeout_seconds = 0;
                definition
            }

            struct FakeRuntime {
                artifact: ArtifactDefinition,
                load_calls: Cell<u32>,
                model_calls: Cell<u32>,
                identity_calls: Cell<u32>,
                run_id_calls: u32,
                execute_calls: u32,
                judge_calls: u32,
                executed: Vec<CaseId>,
                is_identity_fresh: bool,
            }

            impl FakeRuntime {
                fn new(artifact: ArtifactDefinition) -> Self {
                    Self {
                        artifact,
                        load_calls: Cell::new(0),
                        model_calls: Cell::new(0),
                        identity_calls: Cell::new(0),
                        run_id_calls: 0,
                        execute_calls: 0,
                        judge_calls: 0,
                        executed: Vec::new(),
                        is_identity_fresh: true,
                    }
                }
            }

            impl ArtifactSource for FakeRuntime {
                fn load(&self, _root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
                    self.load_calls.set(self.load_calls.get() + 1);
                    Ok(self.artifact.clone())
                }
            }

            impl ModelResolver for FakeRuntime {
                fn candidates(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    self.model_calls.set(self.model_calls.get() + 1);
                    assert_eq!(tier, Tier::T3);
                    Ok(vec![model(tier, "HIDDEN_MODEL_SENTINEL")])
                }

                fn qualification_routes(
                    &self,
                    tier: Tier,
                ) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    self.candidates(tier)
                }

                fn exact_candidate(
                    &self,
                    _requested: &ModelIdentity,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    Err(SkillEvalError::InvalidConfiguration(
                        "exact model-pool candidate resolution is not implemented".to_owned(),
                    ))
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
                    Ok(model(judge_tier, "judge-secret"))
                }
            }

            impl HarnessResolver for FakeRuntime {
                fn identity(
                    &self,
                    artifact: &ArtifactDefinition,
                    _execution: &ExecutionDefinition,
                ) -> Result<HarnessIdentity, SkillEvalError> {
                    self.identity_calls.set(self.identity_calls.get() + 1);
                    Ok(HarnessIdentity {
                        runner_version: format!("runner-{}", self.identity_calls.get()),
                        pi_version: "pi".to_owned(),
                        artifact_revision: artifact.revision.clone(),
                        tool_policy_digest: "policy".to_owned(),
                    })
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
                    self.is_identity_fresh &= self.identity_calls.get() == self.execute_calls;
                    self.executed.push(case.id.clone());
                    Ok(CandidateArtifact {
                        key: key.clone(),
                        model: model.clone(),
                        harness: harness.clone(),
                        artifact_path: PathBuf::from(&run_id.0)
                            .join("CANDIDATE_TEXT_SENTINEL/artifact"),
                        transcript_path: PathBuf::from(&run_id.0)
                            .join("PRIOR_VOTE_SENTINEL/transcript"),
                        usage: usage(),
                    })
                }
            }

            impl Verifier for FakeRuntime {
                fn verify(
                    &mut self,
                    _case: &CaseDefinition,
                    _candidate: &CandidateArtifact,
                ) -> Result<Vec<CheckResult>, SkillEvalError> {
                    Ok(vec![CheckResult {
                        name: "private-check-name".to_owned(),
                        status: CheckStatus::Failed,
                        detail: Some("PRIOR_GRADE_SENTINEL".to_owned()),
                    }])
                }
            }

            impl Judge for FakeRuntime {
                fn grade(
                    &mut self,
                    model: &ModelIdentity,
                    _input: &JudgeInput,
                ) -> Result<JudgeResult, SkillEvalError> {
                    self.judge_calls += 1;
                    Ok(JudgeResult {
                        verdict: TrialVerdict {
                            score: 4,
                            is_catastrophic: false,
                            failure_mode: Some("missing gate".to_owned()),
                            checks: Vec::new(),
                        },
                        model: model.clone(),
                        usage: usage(),
                    })
                }

                fn grade_prompt(
                    &mut self,
                    _model: &ModelIdentity,
                    _request: &PromptJudgeRequest,
                ) -> Result<PromptJudgeResult, SkillEvalError> {
                    unreachable!()
                }
            }

            impl RunIdSource for FakeRuntime {
                fn next(&mut self) -> Result<RunId, SkillEvalError> {
                    self.run_id_calls += 1;
                    Ok(RunId("audit-run-1".to_owned()))
                }
            }

            impl RunStore for FakeRuntime {
                fn append(&mut self, _run_id: &RunId, _event: &RunEvent) -> Result<(), SkillEvalError> {
                    unreachable!()
                }

                fn replay(
                    &self,
                    _run_id: &RunId,
                    _visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
                ) -> Result<(), SkillEvalError> {
                    unreachable!()
                }

                fn find_trial(&self, _selector: &TrialSelector) -> Result<TrialRecord, SkillEvalError> {
                    unreachable!()
                }
            }

            impl Clock for FakeRuntime {
                fn now(&self) -> Timestamp {
                    Timestamp("now".to_owned())
                }
            }

            impl TierWriter for FakeRuntime {
                fn write(
                    &mut self,
                    _artifact: &ArtifactDefinition,
                    _assignments: &[TierAssignment],
                ) -> Result<(), SkillEvalError> {
                    unreachable!()
                }
            }

            impl QualificationRuntime for FakeRuntime {}

            fn assert_runtime_is_unused(runtime: &FakeRuntime) {
                assert_eq!(runtime.load_calls.get(), 0);
                assert_eq!(runtime.model_calls.get(), 0);
                assert_eq!(runtime.identity_calls.get(), 0);
                assert_eq!(runtime.execute_calls, 0);
                assert_eq!(runtime.judge_calls, 0);
            }

            fn model(tier: Tier, name: &str) -> ModelIdentity {
                ModelIdentity {
                    tier,
                    provider: "synthetic".to_owned(),
                    model: name.to_owned(),
                    thinking: "fixed".to_owned(),
                }
            }

            fn usage() -> TrialUsage {
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

            fn read_tree(root: &Path) -> String {
                let mut paths = fs::read_dir(root)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>();
                paths.sort();
                let mut output = String::new();
                for path in paths {
                    if path.is_dir() {
                        output.push_str(&read_tree(&path));
                    } else {
                        output.push_str(&fs::read_to_string(path).unwrap());
                    }
                }
                output
            }

            struct Fixture {
                path: PathBuf,
            }

            impl Fixture {
                fn new() -> Self {
                    let path = std::env::temp_dir().join(format!(
                        "skill-eval-audit-{}-{}",
                        std::process::id(),
                        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
                    ));
                    fs::create_dir_all(&path).unwrap();
                    Self { path }
                }
            }

            impl Drop for Fixture {
                fn drop(&mut self) {
                    fs::remove_dir_all(&self.path).unwrap();
                }
            }
        }
    };
}
