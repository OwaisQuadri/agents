#[cfg(test)]
use std::collections::{BTreeMap, VecDeque};
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use crate::model::{
    ArtifactDefinition, ArtifactKind, ArtifactName, CandidateArtifact, CaseDefinition, CaseDrive,
    CaseId, CheckResult, ExecutionDefinition, HarnessIdentity, JudgeInput, JudgeResult,
    ModelIdentity, PromptJudgeRequest, PromptJudgeResult, RunEvent, RunId, SkillEvalError, Tier,
    TierAssignment, TierDestination, Timestamp, TrialKey, TrialRecord, TrialSelector, TrialUsage,
    TrialVerdict,
};
#[cfg(test)]
use crate::ports::{
    ArtifactSource, CandidateRunner, Clock, HarnessResolver, Judge, ModelResolver,
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
qualification_harness_tests!();
