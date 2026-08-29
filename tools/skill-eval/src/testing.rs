#[cfg(test)]
use std::collections::{BTreeMap, VecDeque};
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use sha2::{Digest, Sha256};

#[cfg(test)]
use crate::model::{
    ArtifactDefinition, ArtifactKind, ArtifactName, CandidateArtifact, CaseDefinition, CaseDrive,
    CaseId, CheckResult, ExecutionDefinition, FrontierApplyReport, FrontierBaselineLedger,
    FrontierCaseGroup, FrontierCaseReference, FrontierConfidenceMethod, FrontierEntrant,
    FrontierInspection, FrontierPlan, FrontierPolicy, FrontierRunId, FrontierRunState,
    FrontierSuite, FrontierSuiteIdentity, FrontierTierSuite, FrontierTrialSelector,
    HarnessIdentity, JudgeInput, JudgeResult, ModelIdentity, PromptJudgeRequest, PromptJudgeResult,
    RunEvent, RunId, SkillEvalError, T1ScreenSnapshotIdentity, Tier, TierAssignment,
    TierDestination, Timestamp, TrialKey, TrialRecord, TrialSelector, TrialUsage, TrialVerdict,
};
#[cfg(test)]
use crate::ports::{
    ArtifactSource, CandidateRunner, Clock, FrontierRuntime, HarnessResolver, Judge, ModelResolver,
    QualificationRuntime, RunIdSource, RunStore, TierWriter, Verifier,
};
#[cfg(test)]
use crate::store::FileRunStore;

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) enum ScriptedOutcome {
    Pass,
    Fail,
    Catastrophic,
    Quota,
}

#[cfg(test)]
pub(crate) struct TemporaryRoot {
    path: PathBuf,
}

#[cfg(test)]
impl TemporaryRoot {
    pub(crate) fn new(label: &str) -> Self {
        static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skill-eval-{label}-{}-{sequence:04}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
impl Drop for TemporaryRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
// TODO(AGNT-0032.T153): Drive a complete deterministic frontier without network access.
pub(crate) struct FakeQualificationRuntime {
    artifact: ArtifactDefinition,
    store: FileRunStore,
    runs_root: PathBuf,
    next_run_id: u64,
    execute_calls: u32,
    outcomes: VecDeque<(Tier, ScriptedOutcome)>,
    qualification_routes: BTreeMap<Tier, Vec<ModelIdentity>>,
    runner_version: String,
    pi_version: String,
    route_provider: String,
    is_effective_fallback: bool,
    written_assignments: Vec<Vec<TierAssignment>>,
}

#[cfg(test)]
impl FakeQualificationRuntime {
    pub(crate) fn new(root: &TemporaryRoot) -> Self {
        let artifact_root = root.path().join("artifact");
        fs::create_dir_all(artifact_root.join("evals")).unwrap();
        fs::write(artifact_root.join("evals/rubric.md"), "synthetic rubric\n").unwrap();
        fs::write(artifact_root.join("evals/own.json"), "{}\n").unwrap();
        let runs_root = root.path().join("runs");
        let store = FileRunStore::new(&runs_root).unwrap();
        let runs_root = fs::canonicalize(runs_root).unwrap();
        Self {
            artifact: synthetic_artifact(artifact_root),
            store,
            runs_root,
            next_run_id: 0,
            execute_calls: 0,
            outcomes: VecDeque::new(),
            qualification_routes: BTreeMap::new(),
            runner_version: "fake-runner-v1".to_owned(),
            pi_version: "fake-pi-v1".to_owned(),
            route_provider: "fake-provider".to_owned(),
            is_effective_fallback: true,
            written_assignments: Vec::new(),
        }
    }

    pub(crate) fn artifact(&self) -> &ArtifactDefinition {
        &self.artifact
    }

    pub(crate) fn script(
        &mut self,
        tier: Tier,
        outcomes: impl IntoIterator<Item = ScriptedOutcome>,
    ) {
        self.outcomes
            .extend(outcomes.into_iter().map(|outcome| (tier, outcome)));
    }

    pub(crate) fn set_qualification_routes(&mut self, tier: Tier, routes: Vec<ModelIdentity>) {
        self.qualification_routes.insert(tier, routes);
    }

    pub(crate) fn execute_call_count(&self) -> u32 {
        self.execute_calls
    }

    pub(crate) fn drift_runner_identity(&mut self) {
        self.runner_version = "drifted-runner".to_owned();
    }

    pub(crate) fn use_requested_model(&mut self) {
        self.is_effective_fallback = false;
    }

    pub(crate) fn written_assignments(&self) -> &[Vec<TierAssignment>] {
        &self.written_assignments
    }

    fn next_outcome(&mut self, tier: Tier) -> ScriptedOutcome {
        let position = self
            .outcomes
            .iter()
            .position(|(scripted_tier, _)| *scripted_tier == tier);
        position
            .and_then(|index| self.outcomes.remove(index))
            .map_or(ScriptedOutcome::Pass, |(_, outcome)| outcome)
    }
}

#[cfg(test)]
impl ArtifactSource for FakeQualificationRuntime {
    fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
        if root == self.artifact.root {
            Ok(self.artifact.clone())
        } else {
            Err(SkillEvalError::NotFound(root.display().to_string()))
        }
    }
}

#[cfg(test)]
impl ModelResolver for FakeQualificationRuntime {
    fn candidates(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
        Ok(vec![
            model(tier, &self.route_provider, "requested"),
            model(tier, &self.route_provider, "effective"),
        ])
    }

    fn qualification_routes(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
        if let Some(routes) = self.qualification_routes.get(&tier) {
            return Ok(routes.clone());
        }
        let role = if self.is_effective_fallback {
            "effective"
        } else {
            "requested"
        };
        Ok(vec![model(tier, &self.route_provider, role)])
    }

    fn exact_candidate(&self, requested: &ModelIdentity) -> Result<ModelIdentity, SkillEvalError> {
        Ok(requested.clone())
    }

    fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
        Ok(Tier::T5)
    }

    fn judge(
        &self,
        judge_tier: Tier,
        _candidate: Option<&ModelIdentity>,
    ) -> Result<ModelIdentity, SkillEvalError> {
        Ok(model(judge_tier, "fake-judge", "external"))
    }
}

#[cfg(test)]
impl HarnessResolver for FakeQualificationRuntime {
    fn identity(
        &self,
        artifact: &ArtifactDefinition,
        execution: &ExecutionDefinition,
    ) -> Result<HarnessIdentity, SkillEvalError> {
        Ok(HarnessIdentity {
            runner_version: self.runner_version.clone(),
            pi_version: self.pi_version.clone(),
            artifact_revision: artifact.revision.clone(),
            tool_policy_digest: format!(
                "tools:{};timeout:{}",
                execution.allowed_tools.join(","),
                execution.timeout_seconds
            ),
        })
    }
}

#[cfg(test)]
impl RunIdSource for FakeQualificationRuntime {
    fn next(&mut self) -> Result<RunId, SkillEvalError> {
        self.next_run_id += 1;
        Ok(RunId(format!("fake-run-{:04}", self.next_run_id)))
    }
}

#[cfg(test)]
impl CandidateRunner for FakeQualificationRuntime {
    fn execute(
        &mut self,
        run_id: &RunId,
        key: &TrialKey,
        _artifact: &ArtifactDefinition,
        _case: &CaseDefinition,
        model: &ModelIdentity,
        harness: &HarnessIdentity,
        _candidate_timeout_seconds: Option<u32>,
    ) -> Result<CandidateArtifact, SkillEvalError> {
        self.execute_calls += 1;
        Ok(CandidateArtifact {
            key: key.clone(),
            model: model.clone(),
            harness: harness.clone(),
            artifact_path: self.runs_root.join(&run_id.0).join(format!(
                "artifacts/{:?}-{}-{}.txt",
                key.tier, key.case.0, key.attempt
            )),
            transcript_path: self.runs_root.join(&run_id.0).join(format!(
                "transcripts/{:?}-{}-{}.jsonl",
                key.tier, key.case.0, key.attempt
            )),
            usage: usage(2),
        })
    }
}

#[cfg(test)]
impl Verifier for FakeQualificationRuntime {
    fn verify(
        &mut self,
        _case: &CaseDefinition,
        _candidate: &CandidateArtifact,
    ) -> Result<Vec<CheckResult>, SkillEvalError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
impl Judge for FakeQualificationRuntime {
    fn grade(
        &mut self,
        model: &ModelIdentity,
        input: &JudgeInput,
    ) -> Result<JudgeResult, SkillEvalError> {
        let outcome = self.next_outcome(input.candidate.key.tier);
        if matches!(outcome, ScriptedOutcome::Quota) {
            return Err(SkillEvalError::Quota {
                model: model.clone(),
                reset_at: Some(Timestamp("synthetic-reset".to_owned())),
            });
        }
        let (score, is_catastrophic, failure_mode) = match outcome {
            ScriptedOutcome::Pass => (9, false, None),
            ScriptedOutcome::Fail => (4, false, Some("synthetic failure".to_owned())),
            ScriptedOutcome::Catastrophic => (10, true, Some("synthetic catastrophe".to_owned())),
            ScriptedOutcome::Quota => unreachable!(),
        };
        Ok(JudgeResult {
            verdict: TrialVerdict {
                score,
                is_catastrophic,
                failure_mode,
                checks: input.checks.clone(),
            },
            model: model.clone(),
            usage: usage(3),
        })
    }

    fn grade_prompt(
        &mut self,
        model: &ModelIdentity,
        request: &PromptJudgeRequest,
    ) -> Result<PromptJudgeResult, SkillEvalError> {
        Ok(PromptJudgeResult {
            model: model.clone(),
            response: format!("synthetic judgment: {}", request.prompt),
            usage: usage(1),
        })
    }
}

#[cfg(test)]
impl RunStore for FakeQualificationRuntime {
    fn append(&mut self, run_id: &RunId, event: &RunEvent) -> Result<(), SkillEvalError> {
        self.store.append(run_id, event)
    }

    fn replay(
        &self,
        run_id: &RunId,
        visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
    ) -> Result<(), SkillEvalError> {
        self.store.replay(run_id, visitor)
    }

    fn find_trial(&self, selector: &TrialSelector) -> Result<TrialRecord, SkillEvalError> {
        self.store.find_trial(selector)
    }
}

#[cfg(test)]
impl Clock for FakeQualificationRuntime {
    fn now(&self) -> Timestamp {
        Timestamp("2026-08-24T12:00:00-0400".to_owned())
    }
}

#[cfg(test)]
impl TierWriter for FakeQualificationRuntime {
    fn write(
        &mut self,
        _artifact: &ArtifactDefinition,
        assignments: &[TierAssignment],
    ) -> Result<(), SkillEvalError> {
        self.written_assignments.push(assignments.to_vec());
        Ok(())
    }
}

#[cfg(test)]
impl QualificationRuntime for FakeQualificationRuntime {}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FakeFrontierAttemptKind {
    Completed,
    Infrastructure,
    Quota,
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FakeFrontierAttempt {
    pub(crate) run_id: FrontierRunId,
    pub(crate) model: ModelIdentity,
    pub(crate) key: TrialKey,
    pub(crate) kind: FakeFrontierAttemptKind,
}

#[cfg(test)]
pub(crate) struct FakeFrontierRuntime {
    repository_root: PathBuf,
    plan: FrontierPlan,
    suite: FrontierSuite,
    artifacts: BTreeMap<PathBuf, ArtifactDefinition>,
    states: BTreeMap<FrontierRunId, FrontierRunState>,
    next_frontier_run_id: u8,
    interruptions: VecDeque<FakeFrontierAttemptKind>,
    attempts: Vec<FakeFrontierAttempt>,
    saved_trials: Vec<(FrontierRunId, TrialRecord)>,
    baseline_ledger: FrontierBaselineLedger,
    routing_configuration_sha256: String,
}

#[cfg(test)]
impl FakeFrontierRuntime {
    pub(crate) fn new(root: &TemporaryRoot) -> Self {
        let repository_root = fs::canonicalize(root.path()).unwrap();
        fs::create_dir_all(repository_root.join("config")).unwrap();
        fs::write(
            repository_root.join("config/model-tiers.json"),
            include_bytes!("../tests/fixtures/frontier-full/model-tiers.json"),
        )
        .unwrap();
        let capabilities_path = PathBuf::from("fixtures/capabilities.json");
        write_fixture(&repository_root, &capabilities_path, b"{}\n");
        let capabilities_sha256 =
            sha256(&fs::read(repository_root.join(&capabilities_path)).unwrap());
        let (suite, artifacts) = frontier_suite_and_artifacts(&repository_root);
        let suite_path = PathBuf::from("fixtures/suite.json");
        write_json_fixture(&repository_root, &suite_path, &suite);
        let suite_sha256 = sha256(&fs::read(repository_root.join(&suite_path)).unwrap());
        let plan = frontier_plan(
            suite_path,
            suite_sha256,
            capabilities_path,
            capabilities_sha256,
        );
        write_fixture(
            &repository_root,
            Path::new("fixtures/frontier-plan.json"),
            &serde_json::to_vec(&plan).unwrap(),
        );
        let routing_configuration_sha256 =
            sha256(&fs::read(repository_root.join("config/model-tiers.json")).unwrap());
        Self {
            repository_root,
            plan,
            suite,
            artifacts,
            states: BTreeMap::new(),
            next_frontier_run_id: 0,
            interruptions: VecDeque::from([FakeFrontierAttemptKind::Infrastructure]),
            attempts: Vec::new(),
            saved_trials: Vec::new(),
            baseline_ledger: FrontierBaselineLedger {
                version: 1,
                baselines: Vec::new(),
            },
            routing_configuration_sha256,
        }
    }

    pub(crate) fn plan_path(&self) -> &Path {
        Path::new("fixtures/frontier-plan.json")
    }

    pub(crate) fn attempts(&self) -> &[FakeFrontierAttempt] {
        &self.attempts
    }

    pub(crate) fn saved_trials(&self, run_id: &FrontierRunId) -> Vec<TrialRecord> {
        self.saved_trials
            .iter()
            .filter(|(stored_run_id, _)| stored_run_id == run_id)
            .map(|(_, trial)| trial.clone())
            .collect()
    }

    pub(crate) fn baseline_ledger(&self) -> FrontierBaselineLedger {
        self.baseline_ledger.clone()
    }

    pub(crate) fn routing_bytes(&self) -> Vec<u8> {
        fs::read(self.repository_root.join("config/model-tiers.json")).unwrap()
    }

    fn record_attempt(
        &mut self,
        run_id: &RunId,
        model: &ModelIdentity,
        key: &TrialKey,
        kind: FakeFrontierAttemptKind,
    ) {
        self.attempts.push(FakeFrontierAttempt {
            run_id: FrontierRunId(run_id.0.clone()),
            model: model.clone(),
            key: key.clone(),
            kind,
        });
    }
}

#[cfg(test)]
impl ArtifactSource for FakeFrontierRuntime {
    fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
        self.artifacts
            .get(root)
            .cloned()
            .ok_or_else(|| SkillEvalError::NotFound(root.display().to_string()))
    }
}

#[cfg(test)]
impl ModelResolver for FakeFrontierRuntime {
    fn candidates(&self, _tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
        panic!("fake frontier resolved an unplanned candidate list")
    }

    fn qualification_routes(&self, _tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError> {
        panic!("fake frontier resolved unplanned qualification routes")
    }

    fn exact_candidate(&self, requested: &ModelIdentity) -> Result<ModelIdentity, SkillEvalError> {
        Ok(requested.clone())
    }

    fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError> {
        panic!("fake frontier resolved an unplanned judge tier")
    }

    fn judge(
        &self,
        judge_tier: Tier,
        _candidate: Option<&ModelIdentity>,
    ) -> Result<ModelIdentity, SkillEvalError> {
        if judge_tier != self.plan.judge.tier {
            return Err(SkillEvalError::InvalidConfiguration(
                "fake frontier judge tier drifted".to_owned(),
            ));
        }
        Ok(self.plan.judge.clone())
    }
}

#[cfg(test)]
impl HarnessResolver for FakeFrontierRuntime {
    fn identity(
        &self,
        artifact: &ArtifactDefinition,
        execution: &ExecutionDefinition,
    ) -> Result<HarnessIdentity, SkillEvalError> {
        Ok(HarnessIdentity {
            runner_version: "fake-frontier-runner-v1".to_owned(),
            pi_version: "fake-pi-v1".to_owned(),
            artifact_revision: artifact.revision.clone(),
            tool_policy_digest: format!(
                "tools:{};timeout:{}",
                execution.allowed_tools.join(","),
                execution.timeout_seconds
            ),
        })
    }
}

#[cfg(test)]
impl RunIdSource for FakeFrontierRuntime {
    fn next(&mut self) -> Result<RunId, SkillEvalError> {
        panic!("fake frontier allocated an unplanned ordinary run")
    }
}

#[cfg(test)]
impl CandidateRunner for FakeFrontierRuntime {
    fn execute(
        &mut self,
        run_id: &RunId,
        key: &TrialKey,
        _artifact: &ArtifactDefinition,
        _case: &CaseDefinition,
        model: &ModelIdentity,
        harness: &HarnessIdentity,
        candidate_timeout_seconds: Option<u32>,
    ) -> Result<CandidateArtifact, SkillEvalError> {
        if candidate_timeout_seconds.is_some() {
            return Err(SkillEvalError::InvalidConfiguration(
                "fake frontier received a candidate timeout".to_owned(),
            ));
        }
        if let Some(kind) = self.interruptions.pop_front() {
            self.record_attempt(run_id, model, key, kind);
            return match kind {
                FakeFrontierAttemptKind::Infrastructure => Err(SkillEvalError::Process {
                    program: "fake-frontier".to_owned(),
                    exit_code: Some(75),
                    standard_error: "synthetic retryable infrastructure failure".to_owned(),
                }),
                FakeFrontierAttemptKind::Quota => Err(SkillEvalError::Quota {
                    model: model.clone(),
                    reset_at: Some(self.now()),
                }),
                FakeFrontierAttemptKind::Completed => unreachable!(),
            };
        }
        self.record_attempt(run_id, model, key, FakeFrontierAttemptKind::Completed);
        let run_root = self
            .repository_root
            .join(".map/skill-eval/frontier")
            .join(&run_id.0);
        let artifact_path = run_root.join("artifacts").join(format!(
            "{:?}-{}-{}-{}.txt",
            key.tier, key.route_index, key.case.0, key.attempt
        ));
        let transcript_path = run_root.join("transcripts").join(format!(
            "{:?}-{}-{}-{}.jsonl",
            key.tier, key.route_index, key.case.0, key.attempt
        ));
        fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
        fs::create_dir_all(transcript_path.parent().unwrap()).unwrap();
        fs::write(&artifact_path, b"synthetic candidate evidence\n").unwrap();
        fs::write(&transcript_path, b"{\"kind\":\"synthetic\"}\n").unwrap();
        Ok(CandidateArtifact {
            key: key.clone(),
            model: model.clone(),
            harness: harness.clone(),
            artifact_path,
            transcript_path,
            usage: usage(1),
        })
    }
}

#[cfg(test)]
impl Verifier for FakeFrontierRuntime {
    fn verify(
        &mut self,
        _case: &CaseDefinition,
        _candidate: &CandidateArtifact,
    ) -> Result<Vec<CheckResult>, SkillEvalError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
impl Judge for FakeFrontierRuntime {
    fn grade(
        &mut self,
        model: &ModelIdentity,
        input: &JudgeInput,
    ) -> Result<JudgeResult, SkillEvalError> {
        let is_screening_failure_case = input.candidate.key.case.0.ends_with("case-00")
            || input.candidate.key.case.0.ends_with("case-01");
        let is_failure = input.candidate.model.model == "fail"
            || is_screening_failure_case
                && (input.candidate.model.model == "indeterminate"
                    || input.candidate.key.attempt == 1);
        Ok(JudgeResult {
            verdict: TrialVerdict {
                score: if is_failure { 0 } else { 10 },
                is_catastrophic: false,
                failure_mode: is_failure.then(|| "synthetic failure".to_owned()),
                checks: input.checks.clone(),
            },
            model: model.clone(),
            usage: usage(1),
        })
    }

    fn grade_prompt(
        &mut self,
        _model: &ModelIdentity,
        _request: &PromptJudgeRequest,
    ) -> Result<PromptJudgeResult, SkillEvalError> {
        panic!("fake frontier made an unplanned prompt evaluation")
    }
}

#[cfg(test)]
impl RunStore for FakeFrontierRuntime {
    fn append(&mut self, _run_id: &RunId, _event: &RunEvent) -> Result<(), SkillEvalError> {
        panic!("fake frontier appended an unplanned ordinary event")
    }

    fn replay(
        &self,
        _run_id: &RunId,
        _visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
    ) -> Result<(), SkillEvalError> {
        panic!("fake frontier replayed an unplanned ordinary run")
    }

    fn find_trial(&self, _selector: &TrialSelector) -> Result<TrialRecord, SkillEvalError> {
        panic!("fake frontier searched an unplanned ordinary trial")
    }
}

#[cfg(test)]
impl Clock for FakeFrontierRuntime {
    fn now(&self) -> Timestamp {
        Timestamp("2030-01-01T00:00:00+0000".to_owned())
    }
}

#[cfg(test)]
impl TierWriter for FakeFrontierRuntime {
    fn write(
        &mut self,
        _artifact: &ArtifactDefinition,
        _assignments: &[TierAssignment],
    ) -> Result<(), SkillEvalError> {
        panic!("fake frontier wrote unplanned artifact tiers")
    }
}

#[cfg(test)]
impl QualificationRuntime for FakeFrontierRuntime {}

#[cfg(test)]
impl FrontierRuntime for FakeFrontierRuntime {
    fn load_frontier_plan(
        &self,
        path: &Path,
    ) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError> {
        if path != self.plan_path() {
            return Err(SkillEvalError::NotFound(path.display().to_string()));
        }
        Ok((self.plan.clone(), self.suite.clone()))
    }

    fn next_frontier_run_id(&mut self) -> Result<FrontierRunId, SkillEvalError> {
        self.next_frontier_run_id += 1;
        Ok(FrontierRunId(format!(
            "frontier-full-{}",
            self.next_frontier_run_id
        )))
    }

    fn create_frontier(&mut self, state: &FrontierRunState) -> Result<(), SkillEvalError> {
        if self
            .states
            .insert(state.configuration.run_id.clone(), state.clone())
            .is_some()
        {
            return Err(SkillEvalError::InvalidConfiguration(
                "fake frontier repeated a run id".to_owned(),
            ));
        }
        Ok(())
    }

    fn load_frontier(&self, run_id: &FrontierRunId) -> Result<FrontierRunState, SkillEvalError> {
        self.states
            .get(run_id)
            .cloned()
            .ok_or_else(|| SkillEvalError::NotFound(run_id.0.clone()))
    }

    fn save_frontier(&mut self, state: &FrontierRunState) -> Result<(), SkillEvalError> {
        self.states
            .insert(state.configuration.run_id.clone(), state.clone());
        Ok(())
    }

    fn save_frontier_trial(
        &mut self,
        run_id: &FrontierRunId,
        trial: &TrialRecord,
    ) -> Result<(), SkillEvalError> {
        if self
            .saved_trials
            .iter()
            .any(|(stored_run_id, stored)| stored_run_id == run_id && stored.key == trial.key)
        {
            return Err(SkillEvalError::InvalidConfiguration(
                "fake frontier repeated a terminal trial key".to_owned(),
            ));
        }
        self.saved_trials.push((run_id.clone(), trial.clone()));
        Ok(())
    }

    fn load_frontier_trials(
        &self,
        run_id: &FrontierRunId,
    ) -> Result<Vec<TrialRecord>, SkillEvalError> {
        Ok(self
            .saved_trials
            .iter()
            .filter(|(stored_run_id, _)| stored_run_id == run_id)
            .map(|(_, trial)| trial.clone())
            .collect())
    }

    fn inspect_frontier(
        &self,
        selector: &FrontierTrialSelector,
    ) -> Result<FrontierInspection, SkillEvalError> {
        if let Some((_, trial)) = self.saved_trials.iter().find(|(run_id, trial)| {
            run_id == &selector.run_id
                && trial.model.provider == selector.provider
                && trial.model.model == selector.model
                && trial.model.tier == selector.tier
                && trial.model.thinking == selector.thinking
                && trial.key.artifact == selector.artifact
                && trial.key.case == selector.case
                && trial.key.attempt == selector.attempt
        }) {
            return Ok(FrontierInspection::Trial {
                trial: trial.clone(),
            });
        }
        if let Some(event) = self
            .states
            .get(&selector.run_id)
            .into_iter()
            .flat_map(|state| &state.infrastructure_events)
            .find(|event| {
                event.model.provider == selector.provider
                    && event.model.model == selector.model
                    && event.model.tier == selector.tier
                    && event.model.thinking == selector.thinking
                    && event.artifact == selector.artifact
                    && event.case == selector.case
                    && event.attempt == selector.attempt
            })
        {
            return Ok(FrontierInspection::Infrastructure {
                event: event.clone(),
            });
        }
        Err(SkillEvalError::NotFound(
            "fake frontier inspection selector has no exact record".to_owned(),
        ))
    }

    fn load_frontier_baselines(
        &self,
        path: &Path,
    ) -> Result<FrontierBaselineLedger, SkillEvalError> {
        if path != Path::new("config/model-frontier-baseline.json")
            || self.baseline_ledger.baselines.is_empty()
        {
            return Err(SkillEvalError::NotFound(path.display().to_string()));
        }
        Ok(self.baseline_ledger.clone())
    }

    fn accept_frontier_baseline(
        &mut self,
        state: &FrontierRunState,
        path: &Path,
        ledger: &FrontierBaselineLedger,
    ) -> Result<(), SkillEvalError> {
        if path != Path::new("config/model-frontier-baseline.json") {
            return Err(SkillEvalError::NotFound(path.display().to_string()));
        }
        self.states
            .insert(state.configuration.run_id.clone(), state.clone());
        self.baseline_ledger = ledger.clone();
        Ok(())
    }

    fn apply_frontier_routes(
        &mut self,
        state: &FrontierRunState,
    ) -> Result<FrontierApplyReport, SkillEvalError> {
        let ledger = self.baseline_ledger();
        let baseline = ledger.baselines.last().ok_or_else(|| {
            SkillEvalError::NotFound("fake frontier baseline is absent".to_owned())
        })?;
        let report = crate::cli::apply_frontier_routes_at(
            &self.repository_root,
            &self.routing_configuration_sha256,
            state,
            baseline,
        )?;
        if report.is_changed {
            self.routing_configuration_sha256 = sha256(&self.routing_bytes());
        }
        Ok(report)
    }
}

#[cfg(test)]
fn frontier_plan(
    suite_path: PathBuf,
    suite_sha256: String,
    capabilities_path: PathBuf,
    capabilities_sha256: String,
) -> FrontierPlan {
    let tiers = [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5];
    let mut entrants = tiers
        .into_iter()
        .enumerate()
        .map(|(index, tier)| FrontierEntrant {
            provider: if index.is_multiple_of(2) {
                "anthropic".to_owned()
            } else {
                "openai-codex".to_owned()
            },
            model: format!("pass-{tier:?}"),
            entry_tier: tier,
            thinking_levels: vec!["off".to_owned()],
            catalog_observed_at: Timestamp("2030-01-01T00:00:00+0000".to_owned()),
        })
        .collect::<Vec<_>>();
    entrants.push(FrontierEntrant {
        provider: "anthropic".to_owned(),
        model: "fail".to_owned(),
        entry_tier: Tier::T1,
        thinking_levels: vec!["off".to_owned()],
        catalog_observed_at: Timestamp("2030-01-01T00:00:00+0000".to_owned()),
    });
    entrants.push(FrontierEntrant {
        provider: "openai-codex".to_owned(),
        model: "indeterminate".to_owned(),
        entry_tier: Tier::T2,
        thinking_levels: vec!["off".to_owned()],
        catalog_observed_at: Timestamp("2030-01-01T00:00:00+0000".to_owned()),
    });
    entrants.sort_by(|left, right| {
        left.entry_tier
            .cmp(&right.entry_tier)
            .then_with(|| left.provider.cmp(&right.provider))
            .then_with(|| left.model.cmp(&right.model))
    });
    FrontierPlan {
        version: 1,
        suite: FrontierSuiteIdentity {
            path: suite_path,
            sha256: suite_sha256,
            version: 1,
        },
        capabilities: T1ScreenSnapshotIdentity {
            path: capabilities_path,
            sha256: capabilities_sha256,
            version: 1,
            observed_at_unix_seconds: 1_893_456_000,
            pi_version: "fake-pi-v1".to_owned(),
        },
        entrants,
        judge: ModelIdentity {
            provider: "anthropic".to_owned(),
            model: "fake-judge".to_owned(),
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
            confidence_resamples: 100,
            maximum_infrastructure_attempts: 2,
            maximum_catalog_age_seconds: 3_600,
            active_pool_size: 1,
            maximum_trial_cost_millionths_of_dollar: 2,
            spending_limit_millionths_of_dollar: 14_400,
            is_provider_limit_enforced: true,
            is_first_party_only: true,
        },
    }
}

#[cfg(test)]
fn frontier_suite_and_artifacts(
    repository_root: &Path,
) -> (FrontierSuite, BTreeMap<PathBuf, ArtifactDefinition>) {
    let mut artifacts = BTreeMap::new();
    let tiers = [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5];
    let tier_suites = tiers
        .into_iter()
        .map(|tier| {
            let artifact_path = PathBuf::from(format!("artifacts/{tier:?}"));
            let revision = format!("revision-{tier:?}");
            let artifact_root = repository_root.join(&artifact_path);
            fs::create_dir_all(artifact_root.join("evals")).unwrap();
            fs::write(artifact_root.join("evals/rubric.md"), b"synthetic rubric\n").unwrap();
            let cases = (0..30)
                .map(|index| CaseDefinition {
                    id: CaseId(format!("{tier:?}-case-{index:02}")),
                    input: "synthetic input".to_owned(),
                    expect: "synthetic expected result".to_owned(),
                    source: "frontier full fixture".to_owned(),
                    is_holdout: true,
                    support_files: Vec::new(),
                    execution: ExecutionDefinition {
                        drive: CaseDrive::Response,
                        allowed_tools: vec!["read".to_owned()],
                        timeout_seconds: 10,
                    },
                })
                .collect::<Vec<_>>();
            artifacts.insert(
                artifact_path.clone(),
                ArtifactDefinition {
                    name: ArtifactName(format!("{tier:?}")),
                    kind: ArtifactKind::Skill,
                    root: artifact_path.clone(),
                    revision: revision.clone(),
                    required_destinations: vec![TierDestination::SkillMinimum],
                    current_tiers: Vec::new(),
                    cases: cases.clone(),
                },
            );
            let references = cases
                .into_iter()
                .enumerate()
                .map(|(index, case)| FrontierCaseReference {
                    artifact_path: artifact_path.clone(),
                    artifact_revision: revision.clone(),
                    case: case.id,
                    group: match index {
                        0..=13 => FrontierCaseGroup::Normal,
                        14..=19 => FrontierCaseGroup::Edge,
                        20..=24 => FrontierCaseGroup::Adversarial,
                        _ => FrontierCaseGroup::Critical,
                    },
                    is_confirmation: true,
                })
                .collect();
            (
                tier,
                FrontierTierSuite {
                    group_weights_basis_points: BTreeMap::from([
                        (FrontierCaseGroup::Normal, 9_700),
                        (FrontierCaseGroup::Edge, 100),
                        (FrontierCaseGroup::Adversarial, 100),
                        (FrontierCaseGroup::Critical, 100),
                    ]),
                    cases: references,
                },
            )
        })
        .collect();
    (
        FrontierSuite {
            version: 1,
            tiers: tier_suites,
        },
        artifacts,
    )
}

#[cfg(test)]
fn write_fixture(repository_root: &Path, path: &Path, bytes: &[u8]) {
    let destination = repository_root.join(path);
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(destination, bytes).unwrap();
}

#[cfg(test)]
fn write_json_fixture<T: serde::Serialize>(repository_root: &Path, path: &Path, value: &T) {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap();
    bytes.push(b'\n');
    write_fixture(repository_root, path, &bytes);
}

#[cfg(test)]
fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
fn synthetic_artifact(root: PathBuf) -> ArtifactDefinition {
    ArtifactDefinition {
        name: ArtifactName("synthetic-skill".to_owned()),
        kind: ArtifactKind::Skill,
        root,
        revision: "synthetic-revision-v1".to_owned(),
        required_destinations: vec![TierDestination::SkillMinimum, TierDestination::SkillTarget],
        current_tiers: vec![
            TierAssignment {
                destination: TierDestination::SkillMinimum,
                tier: Tier::T3,
            },
            TierAssignment {
                destination: TierDestination::SkillTarget,
                tier: Tier::T3,
            },
        ],
        cases: vec![CaseDefinition {
            id: CaseId("synthetic-case".to_owned()),
            input: "synthetic input".to_owned(),
            expect: "synthetic expected result".to_owned(),
            source: "synthetic fixture".to_owned(),
            is_holdout: false,
            support_files: Vec::new(),
            execution: ExecutionDefinition {
                drive: CaseDrive::Response,
                allowed_tools: vec!["read".to_owned()],
                timeout_seconds: 10,
            },
        }],
    }
}

#[cfg(test)]
fn model(tier: Tier, provider: &str, role: &str) -> ModelIdentity {
    ModelIdentity {
        tier,
        provider: provider.to_owned(),
        model: format!("{role}-{tier:?}"),
        thinking: "low".to_owned(),
    }
}

#[cfg(test)]
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

#[cfg(test)]
include!("../tests/qualification.rs");
#[cfg(test)]
include!("../tests/frontier_full.rs");
#[cfg(test)]
qualification_harness_tests!();
#[cfg(test)]
frontier_full_tests!();
