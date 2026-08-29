use std::path::Path;

use crate::model::{
    ArtifactDefinition, CandidateArtifact, CandidateEnvironmentEntry, CaseDefinition, CheckResult,
    ExecutionDefinition, FrontierApplyReport, FrontierBaselineLedger, FrontierFailureStage,
    FrontierInspection, FrontierPlan, FrontierRunId, FrontierRunState, FrontierSuite,
    FrontierSuiteConstructionPlan, FrontierSuiteInventory, FrontierSuiteProposal,
    FrontierSuitePublication, FrontierSuiteReviewSet, FrontierTrialJob, FrontierTrialOutcome,
    HarnessIdentity, JudgeInput, JudgeResult, ModelIdentity, PoolPlan, PoolRunId, PoolRunState,
    PromptJudgeRequest, PromptJudgeResult, RunEvent, RunId, SkillEvalError, T1ScreenCampaignId,
    T1ScreenCampaignState, T1ScreenRunId, T1ScreenRunState, Tier, TierAssignment, Timestamp,
    TrialKey, TrialRecord, TrialSelector,
};

pub(crate) trait ArtifactSource {
    fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError>;
}

pub(crate) trait ModelResolver {
    fn candidates(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError>;

    fn qualification_routes(&self, tier: Tier) -> Result<Vec<ModelIdentity>, SkillEvalError>;

    fn exact_candidate(&self, requested: &ModelIdentity) -> Result<ModelIdentity, SkillEvalError>;

    fn configured_judge_tier(&self) -> Result<Tier, SkillEvalError>;

    fn judge(
        &self,
        judge_tier: Tier,
        candidate: Option<&ModelIdentity>,
    ) -> Result<ModelIdentity, SkillEvalError>;

    fn pool_judge(&self, _candidate: &ModelIdentity) -> Result<ModelIdentity, SkillEvalError> {
        Err(SkillEvalError::InvalidConfiguration(
            "pool judge resolution is not implemented".to_owned(),
        ))
    }
}

pub(crate) trait HarnessResolver {
    fn identity(
        &self,
        artifact: &ArtifactDefinition,
        execution: &ExecutionDefinition,
    ) -> Result<HarnessIdentity, SkillEvalError>;
}

pub(crate) trait RunIdSource {
    fn next(&mut self) -> Result<RunId, SkillEvalError>;
}

pub(crate) trait PoolRunIdSource {
    fn next_pool(&mut self) -> Result<PoolRunId, SkillEvalError>;
}

pub(crate) trait CandidateRunner {
    #[expect(
        clippy::too_many_arguments,
        reason = "candidate execution needs the frozen trial and timeout context"
    )]
    fn execute(
        &mut self,
        run_id: &RunId,
        key: &TrialKey,
        artifact: &ArtifactDefinition,
        case: &CaseDefinition,
        model: &ModelIdentity,
        harness: &HarnessIdentity,
        candidate_timeout_seconds: Option<u32>,
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
    ) -> Result<JudgeResult, SkillEvalError>;

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

pub(crate) trait PoolPlanSource {
    fn load_pool_plan(&self, path: &Path) -> Result<PoolPlan, SkillEvalError>;

    fn validate_pool_plan_freshness(
        &self,
        plan: &PoolPlan,
        now: &Timestamp,
    ) -> Result<(), SkillEvalError>;
}

pub(crate) trait PoolStore {
    fn create_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError>;

    fn load_pool(&self, run_id: &PoolRunId) -> Result<PoolRunState, SkillEvalError>;

    fn save_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError>;
}

pub(crate) trait T1ScreenStore {
    fn create_t1_screen(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError>;

    fn load_t1_screen(&self, run_id: &T1ScreenRunId) -> Result<T1ScreenRunState, SkillEvalError>;

    fn save_t1_screen(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError>;

    fn load_t1_screen_campaign(
        &self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<T1ScreenCampaignState, SkillEvalError>;

    fn reconcile_t1_screen_campaign(
        &mut self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<T1ScreenCampaignState, SkillEvalError>;

    fn pause_t1_screen_campaign_for_budget(
        &mut self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<T1ScreenCampaignState, SkillEvalError>;

    fn register_t1_screen_campaign_run(
        &mut self,
        state: &T1ScreenRunState,
    ) -> Result<T1ScreenCampaignState, SkillEvalError>;

    fn reconcile_t1_screen_campaign_run(
        &mut self,
        state: &T1ScreenRunState,
    ) -> Result<T1ScreenCampaignState, SkillEvalError>;
}

pub(crate) trait Clock {
    fn now(&self) -> Timestamp;
}

pub(crate) trait ProgressSink {
    fn emit(&mut self, event: &RunEvent) -> Result<(), SkillEvalError>;
}

pub(crate) trait PoolProgressSink {
    fn emit_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError>;
}

pub(crate) trait T1ScreenProgressSink {
    fn emit_t1_screen(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError>;
}

pub(crate) trait FrontierProgressSink {
    /// Emits one cumulative frontier state update.
    ///
    /// The input is the latest validated state. The method produces no value.
    ///
    /// # Errors
    ///
    /// Returns an error when the sink cannot serialize or write the update.
    fn emit_frontier(&mut self, state: &FrontierRunState) -> Result<(), SkillEvalError>;
}

pub(crate) trait FrontierSuiteRuntime: ArtifactSource + Clock {
    /// Loads one frozen complete-bank construction plan.
    ///
    /// The input is a repository-relative path. The output is the validated plan.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe path, malformed version, invalid tier set, invalid weights,
    /// weak reviewer policy, or a policy that counts cross-tier reuse.
    fn load_frontier_suite_construction_plan(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteConstructionPlan, SkillEvalError>;

    /// Loads one frozen complete-bank inventory.
    ///
    /// The input is a repository-relative path. The output is the validated inventory.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe path, malformed version, duplicate identity, or source drift.
    fn load_frontier_suite_inventory(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteInventory, SkillEvalError>;

    /// Loads one immutable set of offline case reviews.
    ///
    /// The input is a repository-relative path. The output is the parsed review set.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe path, malformed version, duplicate record, or invalid field.
    fn load_frontier_suite_review_set(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteReviewSet, SkillEvalError>;

    /// Loads one ready or blocked all-tier proposal.
    ///
    /// The input is a repository-relative path. The output is the validated proposal.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe path, malformed version, broken digest, or invalid capacity.
    fn load_frontier_suite_proposal(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteProposal, SkillEvalError>;

    /// Atomically writes one complete-bank inventory.
    ///
    /// The inputs are a repository-relative destination and validated inventory. The method
    /// produces no value.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe path, existing conflicting bytes, or failed atomic write.
    fn save_frontier_suite_inventory(
        &mut self,
        path: &Path,
        inventory: &FrontierSuiteInventory,
    ) -> Result<(), SkillEvalError>;

    /// Atomically writes one ready or blocked all-tier proposal.
    ///
    /// The inputs are a repository-relative destination and validated proposal. The method
    /// produces no value.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe path, existing conflicting bytes, or failed atomic write.
    fn save_frontier_suite_proposal(
        &mut self,
        path: &Path,
        proposal: &FrontierSuiteProposal,
    ) -> Result<(), SkillEvalError>;

    /// Publishes the suite from one ready proposal in one atomic replacement.
    ///
    /// The inputs are a validated ready proposal, suite destination, and publication time. The
    /// output freezes proposal and suite digests in a publication receipt.
    ///
    /// # Errors
    ///
    /// Returns an error for a blocked or stale proposal, unsafe path, incomplete tier, duplicate
    /// case, changed authority, or failed atomic replacement.
    fn apply_frontier_suite_proposal(
        &mut self,
        proposal: &FrontierSuiteProposal,
        output: &Path,
        published_at: &Timestamp,
    ) -> Result<FrontierSuitePublication, SkillEvalError>;
}

pub(crate) trait FrontierRuntime: QualificationRuntime {
    fn lock_frontier_run(&mut self, _run_id: &FrontierRunId) -> Result<(), SkillEvalError> {
        Ok(())
    }

    fn run_frontier_wave(
        &mut self,
        jobs: Vec<FrontierTrialJob>,
    ) -> Result<Vec<FrontierTrialOutcome>, SkillEvalError> {
        let mut outcomes = Vec::with_capacity(jobs.len());
        for job in jobs {
            let mut failure_stage = FrontierFailureStage::Candidate;
            let result = (|| {
                let candidate = self.execute(
                    &job.run_id,
                    &job.key,
                    &job.artifact,
                    &job.case,
                    &job.model,
                    &job.harness,
                    None,
                )?;
                failure_stage = FrontierFailureStage::Verifier;
                let checks = self.verify(&job.case, &candidate)?;
                failure_stage = FrontierFailureStage::Judge;
                let judged = self.grade(
                    &job.judge,
                    &JudgeInput {
                        candidate: candidate.clone(),
                        expect: job.case.expect.clone(),
                        rubric_path: job.artifact.root.join("evals/rubric.md"),
                        checks,
                    },
                )?;
                Ok(TrialRecord {
                    key: job.key.clone(),
                    model: candidate.model,
                    harness: candidate.harness,
                    artifact_path: candidate.artifact_path,
                    transcript_path: candidate.transcript_path,
                    candidate_usage: candidate.usage,
                    judge_model: judged.model,
                    judge_usage: judged.usage,
                    verdict: judged.verdict,
                })
            })();
            outcomes.push(FrontierTrialOutcome {
                model: job.model,
                key: job.key,
                infrastructure_attempt: job.infrastructure_attempt,
                failure_stage: result.as_ref().err().map(|_| failure_stage),
                result,
            });
        }
        Ok(outcomes)
    }

    /// Loads and validates one frozen frontier plan and its reviewed suite.
    ///
    /// The input is a repository-relative plan path. The output is the plan and suite.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe path, malformed data, stale capabilities, or source drift.
    fn load_frontier_plan(
        &self,
        path: &Path,
    ) -> Result<(FrontierPlan, FrontierSuite), SkillEvalError>;

    /// Creates the next cumulative frontier run identity.
    ///
    /// The method has no input. The output is a fresh run identity.
    ///
    /// # Errors
    ///
    /// Returns an error when a unique safe identity cannot be created.
    fn next_frontier_run_id(&mut self) -> Result<FrontierRunId, SkillEvalError>;

    /// Creates one durable frontier snapshot before execution.
    ///
    /// The input is a validated pending state. The method produces no value.
    ///
    /// # Errors
    ///
    /// Returns an error for an existing destination, invalid state, or failed atomic write.
    fn create_frontier(&mut self, state: &FrontierRunState) -> Result<(), SkillEvalError>;

    /// Loads one durable frontier snapshot.
    ///
    /// The input is an exact run identity. The output is the validated saved state.
    ///
    /// # Errors
    ///
    /// Returns an error when the run is absent, malformed, inconsistent, or unsafe.
    fn load_frontier(&self, run_id: &FrontierRunId) -> Result<FrontierRunState, SkillEvalError>;

    /// Loads every validated durable trial for one frontier run in one store pass.
    ///
    /// The input is an exact run identity. The output is its complete durable trial set.
    ///
    /// # Errors
    ///
    /// Returns an error when any stored trial is malformed, duplicated, unsafe, or inconsistent.
    fn load_frontier_trials(
        &self,
        _run_id: &FrontierRunId,
    ) -> Result<Vec<TrialRecord>, SkillEvalError> {
        Err(SkillEvalError::InvalidConfiguration(
            "frontier runtime does not support bulk trial loading".to_owned(),
        ))
    }

    /// Loads one frontier snapshot and all of its durable trials.
    ///
    /// The input is an exact run identity. The output is one consistent state and trial set.
    ///
    /// # Errors
    ///
    /// Returns an error when the state or any stored trial is malformed or inconsistent.
    fn load_frontier_with_trials(
        &self,
        run_id: &FrontierRunId,
    ) -> Result<(FrontierRunState, Vec<TrialRecord>), SkillEvalError> {
        Ok((
            self.load_frontier(run_id)?,
            self.load_frontier_trials(run_id)?,
        ))
    }

    /// Atomically replaces one durable frontier snapshot.
    ///
    /// The input is a validated successor state. The method produces no value.
    ///
    /// # Errors
    ///
    /// Returns an error for stale authority, invalid transition, evidence loss, or failed write.
    fn save_frontier(&mut self, state: &FrontierRunState) -> Result<(), SkillEvalError>;

    /// Recovers completed candidate and judge evidence left before durable trial append.
    ///
    /// The inputs identify the exact scheduled trial and frozen execution identities. The output
    /// is the recovered completed trial, or none when no complete evidence exists.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, conflicting, unsafe, or partial evidence.
    fn recover_frontier_trial(
        &mut self,
        _state: &FrontierRunState,
        _key: &TrialKey,
        _artifact: &ArtifactDefinition,
        _case: &CaseDefinition,
        _model: &ModelIdentity,
        _harness: &HarnessIdentity,
    ) -> Result<Option<TrialRecord>, SkillEvalError> {
        Ok(None)
    }

    /// Appends one completed frontier trial before its parent aggregate advances.
    ///
    /// The inputs are the frontier run identity and exact trial. The method produces no value.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate, conflicting, unsafe, incomplete, or unwritable evidence.
    fn save_frontier_trial(
        &mut self,
        run_id: &FrontierRunId,
        trial: &TrialRecord,
    ) -> Result<(), SkillEvalError>;

    /// Loads one exact trial or infrastructure record for inspection.
    ///
    /// The input identifies a run, route, artifact, case, and attempt. The output is that record.
    ///
    /// # Errors
    ///
    /// Returns an error when the selector is unsafe, ambiguous, absent, or inconsistent.
    fn inspect_frontier(
        &self,
        selector: &crate::model::FrontierTrialSelector,
    ) -> Result<FrontierInspection, SkillEvalError>;

    /// Loads the accepted baseline ledger from one repository-relative path.
    ///
    /// The input is the ledger path. The output is the validated hash-chained ledger.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe path, malformed entry, broken chain, or evidence drift.
    fn load_frontier_baselines(
        &self,
        path: &Path,
    ) -> Result<FrontierBaselineLedger, SkillEvalError>;

    /// Atomically accepts one run and appends its baseline entry.
    ///
    /// The inputs are accepted state, ledger path, and successor ledger. The method produces no value.
    ///
    /// # Errors
    ///
    /// Returns an error for stale authority, rewritten history, invalid evidence, or failed transaction.
    fn accept_frontier_baseline(
        &mut self,
        state: &FrontierRunState,
        path: &Path,
        ledger: &FrontierBaselineLedger,
    ) -> Result<(), SkillEvalError>;

    /// Publishes one accepted frontier's active routes to the owned routing map.
    ///
    /// The input is terminal accepted state. The output names routes and whether bytes changed.
    ///
    /// # Errors
    ///
    /// Returns an error for unresolved, rejected, stale, drifted, unsafe, or unwritable evidence.
    fn apply_frontier_routes(
        &mut self,
        state: &FrontierRunState,
    ) -> Result<FrontierApplyReport, SkillEvalError>;
}

pub(crate) trait TierWriter {
    fn write(
        &mut self,
        artifact: &ArtifactDefinition,
        assignments: &[TierAssignment],
    ) -> Result<(), SkillEvalError>;
}

pub(crate) trait PoolRuntime:
    QualificationRuntime + PoolRunIdSource + PoolPlanSource + PoolStore
{
}

pub(crate) trait T1ScreenRuntime: QualificationRuntime + T1ScreenStore {
    fn capability_snapshot_bytes(&self, path: &Path) -> Result<Vec<u8>, SkillEvalError>;

    fn candidate_environment_manifest(
        &self,
    ) -> Result<Vec<CandidateEnvironmentEntry>, SkillEvalError>;

    fn judge_cost_upper_bound(
        &self,
        model: &ModelIdentity,
        input: &JudgeInput,
    ) -> Result<u64, SkillEvalError>;

    fn conservative_next_judge_cost_upper_bound(
        &self,
        model: &ModelIdentity,
    ) -> Result<u64, SkillEvalError>;
}

pub(crate) trait QualificationRuntime:
    ArtifactSource
    + ModelResolver
    + HarnessResolver
    + RunIdSource
    + CandidateRunner
    + Verifier
    + Judge
    + RunStore
    + Clock
    + TierWriter
{
}
