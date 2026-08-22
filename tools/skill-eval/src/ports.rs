use std::path::Path;

use crate::model::{
    ArtifactDefinition, CandidateArtifact, CaseDefinition, CheckResult, HarnessIdentity,
    JudgeInput, ModelIdentity, PromptJudgeRequest, PromptJudgeResult, RunEvent, RunId,
    SkillEvalError, Tier, TierAssignment, Timestamp, TrialKey, TrialRecord, TrialSelector,
    TrialVerdict,
};

pub(crate) trait ArtifactSource {
    fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError>;
}

pub(crate) trait ModelResolver {
    fn candidates(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError>;

    fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError>;

    fn judge(
        &self,
        judge_tier: Tier,
        candidate: Option<&ModelIdentity>,
    ) -> Result<ModelIdentity, SkillEvalError>;
}

pub(crate) trait CandidateRunner {
    fn execute(
        &mut self,
        key: &TrialKey,
        artifact: &ArtifactDefinition,
        case: &CaseDefinition,
        model: &ModelIdentity,
        harness: &HarnessIdentity,
    ) -> Result<CandidateArtifact, SkillEvalError>;
}

pub(crate) trait Verifier {
    fn verify(
        &mut self,
        case: &CaseDefinition,
        candidate: &CandidateArtifact,
    ) -> Result<Vec<CheckResult>, SkillEvalError>;
}

pub(crate) trait Judge {
    fn grade(
        &mut self,
        model: &ModelIdentity,
        input: &JudgeInput,
    ) -> Result<TrialVerdict, SkillEvalError>;

    fn grade_prompt(
        &mut self,
        model: &ModelIdentity,
        request: &PromptJudgeRequest,
    ) -> Result<PromptJudgeResult, SkillEvalError>;
}

pub(crate) trait RunStore {
    fn append(&mut self, run_id: &RunId, event: &RunEvent) -> Result<(), SkillEvalError>;

    fn replay(
        &self,
        run_id: &RunId,
        visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
    ) -> Result<(), SkillEvalError>;

    fn find_trial(&self, selector: &TrialSelector) -> Result<TrialRecord, SkillEvalError>;
}

pub(crate) trait Clock {
    fn now(&self) -> Timestamp;
}

pub(crate) trait ProgressSink {
    fn emit(&mut self, event: &RunEvent) -> Result<(), SkillEvalError>;
}

pub(crate) trait TierWriter {
    fn write(
        &mut self,
        artifact: &ArtifactDefinition,
        assignments: &[TierAssignment],
    ) -> Result<(), SkillEvalError>;
}

pub(crate) trait QualificationRuntime:
    ArtifactSource + ModelResolver + CandidateRunner + Verifier + Judge + RunStore + Clock + TierWriter
{
}
