use crate::model::{
    Decision, DecisionRecord, PromptJudgeRequest, PromptJudgeResult, QualificationBoundary,
    QualificationPolicy, QualificationReport, QualifyRequest, RunEvent, RunId, RunState,
    SkillEvalError, SkillName, SkillRoutingDecision, TierEvidence, TrialRecord, TrialSelector,
};
use crate::ports::{Clock, ProgressSink, QualificationRuntime, RunStore};

pub(crate) fn start_qualification(
    request: QualifyRequest,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    unimplemented!()
}

pub(crate) fn resume_qualification(
    run_id: &RunId,
    runtime: &mut dyn QualificationRuntime,
    progress: &mut dyn ProgressSink,
) -> Result<QualificationReport, SkillEvalError> {
    unimplemented!()
}

pub(crate) fn build_report(
    run_id: &RunId,
    store: &dyn RunStore,
) -> Result<QualificationReport, SkillEvalError> {
    unimplemented!()
}

pub(crate) fn inspect_trial(
    selector: &TrialSelector,
    store: &dyn RunStore,
) -> Result<TrialRecord, SkillEvalError> {
    unimplemented!()
}

pub(crate) fn record_decision(
    run_id: &RunId,
    skill: &SkillName,
    decision: Decision,
    reason: Option<String>,
    store: &mut dyn RunStore,
    clock: &dyn Clock,
) -> Result<DecisionRecord, SkillEvalError> {
    unimplemented!()
}

pub(crate) fn routing_decision(
    report: &QualificationReport,
    skill: &SkillName,
) -> Result<Option<SkillRoutingDecision>, SkillEvalError> {
    unimplemented!()
}

pub(crate) fn judge_prompt(
    request: &PromptJudgeRequest,
    runtime: &mut dyn QualificationRuntime,
) -> Result<PromptJudgeResult, SkillEvalError> {
    unimplemented!()
}

pub(crate) fn apply_event(state: &mut RunState, event: &RunEvent) -> Result<(), SkillEvalError> {
    unimplemented!()
}

pub(crate) fn evaluate_tier(
    trials: &[TrialRecord],
    reference: &TierEvidence,
    policy: &QualificationPolicy,
) -> Result<TierEvidence, SkillEvalError> {
    unimplemented!()
}

pub(crate) fn find_boundary(
    evidence: &[TierEvidence],
    policy: &QualificationPolicy,
) -> Result<Option<QualificationBoundary>, SkillEvalError> {
    unimplemented!()
}
