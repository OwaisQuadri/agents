#[macro_export]
macro_rules! frontier_preview_tests {
    () => {
        mod frontier_preview {
            use std::cell::Cell;
            use std::collections::BTreeMap;
            use std::path::{Path, PathBuf};
            use std::rc::Rc;

            use $crate::model::{
                ArtifactDefinition, CandidateArtifact, CaseDefinition, CaseId, CheckResult,
                ExecutionDefinition, FrontierApplyReport, FrontierBaselineLedger,
                FrontierCaseGroup, FrontierCaseReference, FrontierConfidenceMethod,
                FrontierEntrant, FrontierInspection, FrontierPlan, FrontierPolicy, FrontierRunId,
                FrontierRunState, FrontierSuite, FrontierSuiteIdentity, FrontierTierSuite,
                HarnessIdentity, JudgeInput, JudgeResult, ModelIdentity, PromptJudgeRequest,
                PromptJudgeResult, RunEvent, RunId, SkillEvalError, T1ScreenSnapshotIdentity, Tier,
                TierAssignment, Timestamp, TrialKey, TrialRecord, TrialSelector,
            };
            use $crate::ports::{
                ArtifactSource, CandidateRunner, Clock, FrontierRuntime, HarnessResolver, Judge,
                ModelResolver, QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
            };

            use super::{frontier_plan_digest, preview_frontier};

            #[test]
            fn frontier_preview_projects_exact_bounds_without_calls() {
                let runtime = PreviewRuntime::valid();

                let report = preview_frontier(Path::new("plan.json"), &runtime).unwrap();

                assert_eq!(runtime.loads.get(), 1);
                assert_eq!(runtime.clocks.get(), 1);
                assert_eq!(report.tier_case_counts, expected_counts());
                assert_eq!(report.route_count, 3);
                assert_eq!(report.candidate_calls.minimum, 92);
                assert_eq!(report.candidate_calls.maximum, 2_930);
                assert_eq!(report.judge_calls, report.candidate_calls);
                assert_eq!(report.maximum_spending_millionths_of_dollar, 20_510);
                assert!(report.is_owner_approval_required);
                assert_eq!(report.plan_sha256.len(), 64);
                assert_eq!(
                    report.plan_sha256,
                    frontier_plan_digest(&runtime.plan).unwrap()
                );

                let repeated = preview_frontier(Path::new("plan.json"), &runtime).unwrap();
                assert_eq!(report, repeated);
                assert_eq!(runtime.loads.get(), 2);
                assert_eq!(runtime.clocks.get(), 2);
            }

            #[test]
            fn sufficient_owner_authority_clears_the_approval_gate() {
                let mut insufficient = PreviewRuntime::valid();
                insufficient.plan.policy.spending_limit_millionths_of_dollar = 20_509;
                let report = preview_frontier(Path::new("plan.json"), &insufficient).unwrap();
                assert!(report.is_owner_approval_required);

                let mut sufficient = PreviewRuntime::valid();
                sufficient.plan.policy.spending_limit_millionths_of_dollar = 20_510;
                let report = preview_frontier(Path::new("plan.json"), &sufficient).unwrap();
                assert!(!report.is_owner_approval_required);
                assert_eq!(insufficient.loads.get(), 1);
                assert_eq!(insufficient.clocks.get(), 1);
                assert_eq!(sufficient.loads.get(), 1);
                assert_eq!(sufficient.clocks.get(), 1);
            }

            #[test]
            fn overflow_and_zero_trial_cost_fail_by_name() {
                let mut zero = PreviewRuntime::valid();
                zero.plan.policy.maximum_trial_cost_millionths_of_dollar = 0;
                assert_error(&zero, "maximum trial cost is zero");

                let mut overflow = PreviewRuntime::valid();
                overflow.plan.policy.maximum_trial_cost_millionths_of_dollar = u64::MAX;
                assert_error(&overflow, "maximum spending arithmetic overflow");
            }

            #[test]
            fn stale_capability_catalog_and_identity_inputs_fail_by_name() {
                let mut capability = PreviewRuntime::valid();
                capability.plan.capabilities.observed_at_unix_seconds -= 3_601;
                assert_error(&capability, "capability snapshot is stale");

                let mut catalog = PreviewRuntime::valid();
                catalog.plan.entrants[0].catalog_observed_at =
                    Timestamp("2029-12-31T22:59:59+0000".to_owned());
                assert_error(&catalog, "entrant catalog is stale");

                let mut plan = PreviewRuntime::valid();
                plan.plan.version = 2;
                assert_error(&plan, "plan version must be 1");

                let mut suite = PreviewRuntime::valid();
                suite.suite.version = 2;
                assert_error(&suite, "suite identity or version is invalid");

                let mut capabilities = PreviewRuntime::valid();
                capabilities.plan.capabilities.sha256 = "stale".to_owned();
                assert_error(&capabilities, "capability identity or version is invalid");

                let mut source = PreviewRuntime::valid();
                source.load_error = Some("frontier source revision is stale");
                assert_error(&source, "frontier source revision is stale");
            }

            #[test]
            fn invalid_suite_groups_counts_weights_confirmations_and_keys_fail() {
                let mut groups = PreviewRuntime::valid();
                for case in &mut groups.suite.tiers.get_mut(&Tier::T1).unwrap().cases {
                    if case.group == FrontierCaseGroup::Normal {
                        case.group = FrontierCaseGroup::Edge;
                    }
                }
                assert_error(&groups, "groups or confirmations are invalid");

                let mut count = PreviewRuntime::valid();
                count.suite.tiers.get_mut(&Tier::T1).unwrap().cases.pop();
                assert_error(&count, "has fewer than 30 cases");

                let mut weights = PreviewRuntime::valid();
                weights
                    .suite
                    .tiers
                    .get_mut(&Tier::T2)
                    .unwrap()
                    .group_weights_basis_points
                    .insert(FrontierCaseGroup::Normal, 3_999);
                assert_error(&weights, "group weights are invalid");

                let mut confirmations = PreviewRuntime::valid();
                for case in &mut confirmations.suite.tiers.get_mut(&Tier::T3).unwrap().cases {
                    case.is_confirmation = false;
                }
                assert_error(&confirmations, "groups or confirmations are invalid");

                let mut duplicate = PreviewRuntime::valid();
                let repeated = duplicate.suite.tiers[&Tier::T1].cases[0].clone();
                duplicate.suite.tiers.get_mut(&Tier::T2).unwrap().cases[0] = repeated;
                assert_error(&duplicate, "case keys are invalid or not disjoint");
            }

            #[test]
            fn invalid_policy_routes_and_provider_identities_fail_without_calls() {
                let mut attempts = PreviewRuntime::valid();
                attempts.plan.policy.confirmation_trials_per_case = 2;
                assert_error(&attempts, "policy is invalid");

                let mut infrastructure = PreviewRuntime::valid();
                infrastructure.plan.policy.maximum_infrastructure_attempts = 3;
                assert_error(&infrastructure, "policy is invalid");

                let mut order = PreviewRuntime::valid();
                order.plan.entrants.swap(0, 1);
                assert_error(&order, "entrant route order is invalid");

                let mut thinking = PreviewRuntime::valid();
                thinking.plan.entrants[0].thinking_levels =
                    vec!["low".to_owned(), "minimal".to_owned()];
                assert_error(&thinking, "entrant thinking order is invalid");

                let mut provider = PreviewRuntime::valid();
                provider.plan.entrants[0].provider = "openrouter".to_owned();
                assert_error(&provider, "is not first-party");

                let mut judge = PreviewRuntime::valid();
                judge.plan.judge.provider = judge.plan.entrants[0].provider.clone();
                judge.plan.judge.model = judge.plan.entrants[0].model.clone();
                assert_error(&judge, "external judge identity is invalid");

                let mut policy = PreviewRuntime::valid();
                policy.plan.policy.is_first_party_only = false;
                assert_error(&policy, "policy is invalid");
            }

            fn assert_error(runtime: &PreviewRuntime, expected: &str) {
                let error = preview_frontier(Path::new("plan.json"), runtime).unwrap_err();
                assert!(format!("{error:?}").contains(expected), "{error:?}");
                assert_eq!(runtime.loads.get(), 1);
                assert!(runtime.clocks.get() <= 1);
            }

            fn expected_counts() -> BTreeMap<Tier, u16> {
                BTreeMap::from([
                    (Tier::T1, 30),
                    (Tier::T2, 31),
                    (Tier::T3, 32),
                    (Tier::T4, 33),
                    (Tier::T5, 34),
                ])
            }

            struct PreviewRuntime {
                plan: FrontierPlan,
                suite: FrontierSuite,
                load_error: Option<&'static str>,
                loads: Rc<Cell<u32>>,
                clocks: Rc<Cell<u32>>,
            }

            impl PreviewRuntime {
                fn valid() -> Self {
                    Self {
                        plan: plan(),
                        suite: suite(),
                        load_error: None,
                        loads: Rc::new(Cell::new(0)),
                        clocks: Rc::new(Cell::new(0)),
                    }
                }
            }

            impl QualificationRuntime for PreviewRuntime {}

            impl FrontierRuntime for PreviewRuntime {
                fn load_frontier_plan(
                    &self,
                    _path: &Path,
                ) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError> {
                    self.loads.set(self.loads.get() + 1);
                    if let Some(message) = self.load_error {
                        return Err(SkillEvalError::InvalidConfiguration(message.to_owned()));
                    }
                    Ok((self.plan.clone(), self.suite.clone()))
                }

                fn next_frontier_run_id(&mut self) -> Result<FrontierRunId, SkillEvalError> {
                    panic!("preview called next_frontier_run_id")
                }

                fn create_frontier(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    panic!("preview wrote frontier state")
                }

                fn load_frontier(
                    &self,
                    _run_id: &FrontierRunId,
                ) -> Result<FrontierRunState, SkillEvalError> {
                    panic!("preview read frontier state")
                }

                fn save_frontier(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<(), SkillEvalError> {
                    panic!("preview wrote frontier state")
                }

                fn save_frontier_trial(
                    &mut self,
                    _run_id: &FrontierRunId,
                    _trial: &TrialRecord,
                ) -> Result<(), SkillEvalError> {
                    panic!("preview wrote a trial")
                }

                fn inspect_frontier(
                    &self,
                    _selector: &$crate::model::FrontierTrialSelector,
                ) -> Result<FrontierInspection, SkillEvalError> {
                    panic!("preview inspected evidence")
                }

                fn load_frontier_baselines(
                    &self,
                    _path: &Path,
                ) -> Result<FrontierBaselineLedger, SkillEvalError> {
                    panic!("preview read a baseline")
                }

                fn accept_frontier_baseline(
                    &mut self,
                    _state: &FrontierRunState,
                    _path: &Path,
                    _ledger: &FrontierBaselineLedger,
                ) -> Result<(), SkillEvalError> {
                    panic!("preview wrote a baseline")
                }

                fn apply_frontier_routes(
                    &mut self,
                    _state: &FrontierRunState,
                ) -> Result<FrontierApplyReport, SkillEvalError> {
                    panic!("preview wrote routes")
                }
            }

            impl Clock for PreviewRuntime {
                fn now(&self) -> Timestamp {
                    self.clocks.set(self.clocks.get() + 1);
                    Timestamp("2030-01-01T00:00:00+0000".to_owned())
                }
            }

            impl ArtifactSource for PreviewRuntime {
                fn load(&self, _root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
                    panic!("preview loaded an artifact")
                }
            }

            impl ModelResolver for PreviewRuntime {
                fn candidates(&self, _tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    panic!("preview resolved candidates")
                }

                fn qualification_routes(
                    &self,
                    _tier: Tier,
                ) -> Result<Vec<ModelIdentity>, SkillEvalError> {
                    panic!("preview resolved routes")
                }

                fn exact_candidate(
                    &self,
                    _requested: &ModelIdentity,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    panic!("preview resolved a model")
                }

                fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
                    panic!("preview resolved a judge tier")
                }

                fn judge(
                    &self,
                    _judge_tier: Tier,
                    _candidate: Option<&ModelIdentity>,
                ) -> Result<ModelIdentity, SkillEvalError> {
                    panic!("preview resolved a judge")
                }
            }

            impl HarnessResolver for PreviewRuntime {
                fn identity(
                    &self,
                    _artifact: &ArtifactDefinition,
                    _execution: &ExecutionDefinition,
                ) -> Result<HarnessIdentity, SkillEvalError> {
                    panic!("preview resolved a harness")
                }
            }

            impl RunIdSource for PreviewRuntime {
                fn next(&mut self) -> Result<RunId, SkillEvalError> {
                    panic!("preview allocated a run")
                }
            }

            impl CandidateRunner for PreviewRuntime {
                fn execute(
                    &mut self,
                    _run_id: &RunId,
                    _key: &TrialKey,
                    _artifact: &ArtifactDefinition,
                    _case: &CaseDefinition,
                    _model: &ModelIdentity,
                    _harness: &HarnessIdentity,
                    _candidate_timeout_seconds: Option<u32>,
                ) -> Result<CandidateArtifact, SkillEvalError> {
                    panic!("preview called a candidate")
                }
            }

            impl Verifier for PreviewRuntime {
                fn verify(
                    &mut self,
                    _case: &CaseDefinition,
                    _candidate: &CandidateArtifact,
                ) -> Result<Vec<CheckResult>, SkillEvalError> {
                    panic!("preview called a verifier")
                }
            }

            impl Judge for PreviewRuntime {
                fn grade(
                    &mut self,
                    _model: &ModelIdentity,
                    _input: &JudgeInput,
                ) -> Result<JudgeResult, SkillEvalError> {
                    panic!("preview called a judge")
                }

                fn grade_prompt(
                    &mut self,
                    _model: &ModelIdentity,
                    _request: &PromptJudgeRequest,
                ) -> Result<PromptJudgeResult, SkillEvalError> {
                    panic!("preview called a prompt judge")
                }
            }

            impl RunStore for PreviewRuntime {
                fn append(
                    &mut self,
                    _run_id: &RunId,
                    _event: &RunEvent,
                ) -> Result<(), SkillEvalError> {
                    panic!("preview appended an event")
                }

                fn replay(
                    &self,
                    _run_id: &RunId,
                    _visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
                ) -> Result<(), SkillEvalError> {
                    panic!("preview replayed events")
                }

                fn find_trial(
                    &self,
                    _selector: &TrialSelector,
                ) -> Result<TrialRecord, SkillEvalError> {
                    panic!("preview read a trial")
                }
            }

            impl TierWriter for PreviewRuntime {
                fn write(
                    &mut self,
                    _artifact: &ArtifactDefinition,
                    _assignments: &[TierAssignment],
                ) -> Result<(), SkillEvalError> {
                    panic!("preview wrote tiers")
                }
            }

            fn plan() -> FrontierPlan {
                FrontierPlan {
                    version: 1,
                    suite: FrontierSuiteIdentity {
                        path: PathBuf::from("suite.json"),
                        sha256: "a".repeat(64),
                        version: 1,
                    },
                    capabilities: T1ScreenSnapshotIdentity {
                        path: PathBuf::from("capabilities.json"),
                        sha256: "b".repeat(64),
                        version: 1,
                        observed_at_unix_seconds: 1_893_456_000,
                        pi_version: "1.0.0".to_owned(),
                    },
                    entrants: vec![
                        FrontierEntrant {
                            provider: "anthropic".to_owned(),
                            model: "luna".to_owned(),
                            entry_tier: Tier::T1,
                            thinking_levels: vec!["off".to_owned(), "minimal".to_owned()],
                            catalog_observed_at: Timestamp("2030-01-01T00:00:00+0000".to_owned()),
                        },
                        FrontierEntrant {
                            provider: "openai-codex".to_owned(),
                            model: "spark".to_owned(),
                            entry_tier: Tier::T3,
                            thinking_levels: vec!["low".to_owned()],
                            catalog_observed_at: Timestamp("2030-01-01T00:00:00+0000".to_owned()),
                        },
                    ],
                    judge: ModelIdentity {
                        provider: "anthropic".to_owned(),
                        model: "external-judge".to_owned(),
                        tier: Tier::T5,
                        thinking: "high".to_owned(),
                    },
                    policy: FrontierPolicy {
                        screening_trials_per_case: 1,
                        confirmation_trials_per_case: 3,
                        maximum_trials_per_case: 5,
                        minimum_trial_score: 8,
                        minimum_weighted_pass_basis_points: 8_500,
                        minimum_lower_bound_basis_points: 8_000,
                        confidence_level_basis_points: 9_500,
                        confidence_method: FrontierConfidenceMethod::StratifiedBootstrap,
                        confidence_resamples: 1_000,
                        maximum_infrastructure_attempts: 2,
                        maximum_catalog_age_seconds: 3_600,
                        active_pool_size: 5,
                        maximum_trial_cost_millionths_of_dollar: 7,
                        spending_limit_millionths_of_dollar: 0,
                        is_provider_limit_enforced: true,
                        is_first_party_only: true,
                    },
                }
            }

            fn suite() -> FrontierSuite {
                FrontierSuite {
                    version: 1,
                    tiers: expected_counts()
                        .into_iter()
                        .map(|(tier, count)| (tier, tier_suite(tier, count)))
                        .collect(),
                }
            }

            fn tier_suite(tier: Tier, count: u16) -> FrontierTierSuite {
                let cases = (0..count)
                    .map(|index| FrontierCaseReference {
                        artifact_path: PathBuf::from(format!("skills/{tier:?}-{index}")),
                        artifact_revision: format!("revision-{tier:?}-{index}"),
                        case: CaseId(format!("case-{index}")),
                        group: match index % 4 {
                            0 => FrontierCaseGroup::Normal,
                            1 => FrontierCaseGroup::Edge,
                            2 => FrontierCaseGroup::Adversarial,
                            _ => FrontierCaseGroup::Critical,
                        },
                        is_confirmation: index == 0,
                    })
                    .collect();
                FrontierTierSuite {
                    group_weights_basis_points: BTreeMap::from([
                        (FrontierCaseGroup::Normal, 4_000),
                        (FrontierCaseGroup::Edge, 2_000),
                        (FrontierCaseGroup::Adversarial, 2_000),
                        (FrontierCaseGroup::Critical, 2_000),
                    ]),
                    cases,
                }
            }
        }
    };
}
