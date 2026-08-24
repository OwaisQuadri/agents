use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Tier {
    T1,
    T2,
    T3,
    T4,
    T5,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArtifactKind {
    Skill,
    Agent,
    Workflow,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunMode {
    Execute,
    DryRun,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TierDestination {
    SkillMinimum,
    SkillTarget,
    Agent,
    WorkflowOrchestrator,
    WorkflowNode { node: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct TierAssignment {
    pub(crate) destination: TierDestination,
    pub(crate) tier: Tier,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct RunId(pub(crate) String);

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct ArtifactName(pub(crate) String);

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct CaseId(pub(crate) String);

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct Timestamp(pub(crate) String);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct RunConfiguration {
    pub(crate) run_id: RunId,
    pub(crate) mode: RunMode,
    pub(crate) artifacts: Vec<ArtifactDefinition>,
    pub(crate) change: Option<ArtifactChange>,
    pub(crate) policy: QualificationPolicy,
    pub(crate) created_at: Timestamp,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QualificationPurpose {
    #[default]
    Artifact,
    ModelPool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct QualificationPolicy {
    #[serde(default)]
    pub(crate) purpose: QualificationPurpose,
    pub(crate) candidate_tiers: Vec<Tier>,
    pub(crate) reference_tier: Tier,
    pub(crate) judge_tier: Tier,
    pub(crate) repeats_per_case: u16,
    pub(crate) minimum_score: u8,
    pub(crate) noninferiority_margin: f64,
    pub(crate) confidence_level: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct ArtifactDefinition {
    pub(crate) name: ArtifactName,
    pub(crate) kind: ArtifactKind,
    pub(crate) root: PathBuf,
    pub(crate) revision: String,
    pub(crate) required_destinations: Vec<TierDestination>,
    pub(crate) current_tiers: Vec<TierAssignment>,
    pub(crate) cases: Vec<CaseDefinition>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ArtifactChange {
    pub(crate) artifact: ArtifactName,
    pub(crate) kind: ArtifactKind,
    pub(crate) incumbent_revision: String,
    pub(crate) candidate_revision: String,
    pub(crate) own_eval: OwnEvalEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OwnEvalEvidence {
    pub(crate) artifact_revision: String,
    pub(crate) path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct CaseDefinition {
    pub(crate) id: CaseId,
    pub(crate) input: String,
    pub(crate) expect: String,
    pub(crate) source: String,
    pub(crate) is_holdout: bool,
    pub(crate) support_files: Vec<PathBuf>,
    pub(crate) execution: ExecutionDefinition,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CaseDiscovery {
    pub(crate) id: CaseId,
    pub(crate) drive: CaseDrive,
    pub(crate) is_holdout: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ArtifactDiscovery {
    pub(crate) artifact: ArtifactName,
    pub(crate) kind: ArtifactKind,
    pub(crate) revision: String,
    pub(crate) cases: Vec<CaseDiscovery>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ExecutionDefinition {
    pub(crate) drive: CaseDrive,
    pub(crate) allowed_tools: Vec<String>,
    pub(crate) timeout_seconds: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CaseDrive {
    Response,
    Fixture {
        source: PathBuf,
        verify_commands: Vec<CommandDefinition>,
    },
    ExistingHarness {
        command: CommandDefinition,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CommandDefinition {
    pub(crate) program: String,
    pub(crate) arguments: Vec<String>,
    pub(crate) working_directory: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ModelIdentity {
    pub(crate) tier: Tier,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) thinking: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct PoolRunId(pub(crate) String);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PoolStage {
    Calibration,
    Qualification,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PoolEntrant {
    pub(crate) model: ModelIdentity,
    pub(crate) catalog_observed_at: Timestamp,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PoolPolicy {
    pub(crate) calibration_repeats_per_case: u16,
    pub(crate) qualification_repeats_per_case: u16,
    pub(crate) promotion_count: u8,
    pub(crate) minimum_score: u8,
    pub(crate) minimum_reliability_basis_points: u16,
    pub(crate) maximum_catalog_age_seconds: u32,
    pub(crate) spending_limit_millionths_of_dollar: u64,
    pub(crate) is_provider_limit_enforced: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PoolPlan {
    pub(crate) entrants: BTreeMap<Tier, Vec<PoolEntrant>>,
    pub(crate) control: ModelIdentity,
    pub(crate) policy: PoolPolicy,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PoolRunConfiguration {
    pub(crate) run_id: PoolRunId,
    pub(crate) created_at: Timestamp,
    pub(crate) entrants: BTreeMap<Tier, Vec<PoolEntrant>>,
    pub(crate) control: ModelIdentity,
    pub(crate) policy: PoolPolicy,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PoolEntrantEvidence {
    pub(crate) stage: PoolStage,
    pub(crate) requested_model: ModelIdentity,
    pub(crate) effective_model: ModelIdentity,
    pub(crate) judge_model: ModelIdentity,
    pub(crate) harnesses: Vec<HarnessIdentity>,
    pub(crate) is_passing: bool,
    pub(crate) completed_trials: u32,
    pub(crate) expected_trials: u32,
    pub(crate) failed_trials: u32,
    pub(crate) catastrophic_trials: u32,
    pub(crate) score: ConfidenceInterval,
    pub(crate) candidate_usage: TrialUsage,
    pub(crate) judge_usage: TrialUsage,
    pub(crate) total_usage: TrialUsage,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct RankedPool {
    pub(crate) tier: Tier,
    pub(crate) calibration: Vec<PoolEntrantEvidence>,
    pub(crate) promoted: Vec<ModelIdentity>,
    pub(crate) qualification: Vec<PoolEntrantEvidence>,
    pub(crate) ranked: Vec<ModelIdentity>,
    pub(crate) is_complete: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PoolChildStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PoolChildRun {
    pub(crate) tier: Tier,
    pub(crate) entrant_index: u8,
    pub(crate) stage: PoolStage,
    pub(crate) run_id: RunId,
    pub(crate) status: PoolChildStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PoolRunStatus {
    Pending,
    Running,
    Paused,
    AwaitingDecision,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PoolRunState {
    pub(crate) configuration: PoolRunConfiguration,
    pub(crate) selected_tiers: Vec<Tier>,
    pub(crate) status: PoolRunStatus,
    pub(crate) child_runs: Vec<PoolChildRun>,
    pub(crate) pools: Vec<RankedPool>,
    pub(crate) pause: Option<PoolPauseReason>,
    pub(crate) spent_millionths_of_dollar: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PoolPauseReason {
    Quota {
        model: ModelIdentity,
        reset_at: Option<Timestamp>,
    },
    SpendingLimit {
        spent_millionths_of_dollar: u64,
        limit_millionths_of_dollar: u64,
    },
    Infrastructure {
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct HarnessIdentity {
    pub(crate) runner_version: String,
    pub(crate) pi_version: String,
    pub(crate) artifact_revision: String,
    pub(crate) tool_policy_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct TrialKey {
    pub(crate) artifact: ArtifactName,
    pub(crate) tier: Tier,
    pub(crate) case: CaseId,
    pub(crate) attempt: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct TrialUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_read_tokens: u64,
    pub(crate) cache_write_tokens: u64,
    pub(crate) turns: u32,
    pub(crate) tool_calls: u32,
    pub(crate) elapsed_milliseconds: u64,
    pub(crate) cost_millionths_of_dollar: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct TrialRecord {
    pub(crate) key: TrialKey,
    pub(crate) model: ModelIdentity,
    pub(crate) harness: HarnessIdentity,
    pub(crate) artifact_path: PathBuf,
    pub(crate) transcript_path: PathBuf,
    pub(crate) candidate_usage: TrialUsage,
    pub(crate) judge_model: ModelIdentity,
    pub(crate) judge_usage: TrialUsage,
    pub(crate) verdict: TrialVerdict,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct TrialVerdict {
    pub(crate) score: u8,
    pub(crate) is_catastrophic: bool,
    pub(crate) failure_mode: Option<String>,
    pub(crate) checks: Vec<CheckResult>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct JudgeResult {
    pub(crate) verdict: TrialVerdict,
    pub(crate) model: ModelIdentity,
    pub(crate) usage: TrialUsage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CheckResult {
    pub(crate) name: String,
    pub(crate) status: CheckStatus,
    pub(crate) detail: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckStatus {
    Passed,
    Failed,
    NotApplicable,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct ConfidenceInterval {
    pub(crate) lower: f64,
    pub(crate) estimate: f64,
    pub(crate) upper: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceRole {
    Reference,
    Candidate,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct TierEvidence {
    pub(crate) role: EvidenceRole,
    pub(crate) tier: Tier,
    pub(crate) model: ModelIdentity,
    pub(crate) harnesses: Vec<HarnessIdentity>,
    pub(crate) status: TierStatus,
    pub(crate) completed_trials: u32,
    pub(crate) expected_trials: u32,
    pub(crate) passed_trials: u32,
    pub(crate) score: ConfidenceInterval,
    pub(crate) candidate_usage: TrialUsage,
    pub(crate) judge_usage: TrialUsage,
    pub(crate) total_usage: TrialUsage,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TierStatus {
    Pending,
    Running,
    Failed,
    Accepted,
    Paused,
    NeedsReview,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct QualificationBoundary {
    pub(crate) failing: Option<TierEvidence>,
    pub(crate) accepted: TierEvidence,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct ArtifactQualificationState {
    pub(crate) status: ArtifactStatus,
    pub(crate) pending_candidates: Vec<CandidateArtifact>,
    pub(crate) tiers: Vec<TierEvidence>,
    pub(crate) boundary: Option<QualificationBoundary>,
    pub(crate) review_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArtifactStatus {
    Pending,
    Running,
    AwaitingDecision,
    Accepted,
    Rejected,
    Paused,
    NeedsReview,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct RunState {
    pub(crate) run_id: RunId,
    pub(crate) mode: RunMode,
    pub(crate) status: RunStatus,
    pub(crate) discoveries: Vec<ArtifactDiscovery>,
    pub(crate) artifacts: BTreeMap<ArtifactName, ArtifactQualificationState>,
    pub(crate) pause: Option<PauseReason>,
    pub(crate) decisions: BTreeMap<ArtifactName, DecisionRecord>,
    pub(crate) publication_gates: BTreeMap<ArtifactName, PublicationGate>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunStatus {
    Running,
    Discovered,
    Paused,
    AwaitingDecision,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PauseReason {
    Quota {
        model: ModelIdentity,
        reset_at: Option<Timestamp>,
    },
    Infrastructure {
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct DecisionRecord {
    pub(crate) artifact: ArtifactName,
    pub(crate) decision: Decision,
    pub(crate) assignments: Vec<TierAssignment>,
    pub(crate) reason: Option<String>,
    pub(crate) decided_at: Timestamp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PublicationStatus {
    AwaitingQualification,
    AwaitingDecision,
    Ready,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PublicationGate {
    pub(crate) change: ArtifactChange,
    pub(crate) status: PublicationStatus,
    pub(crate) assignments: Vec<TierAssignment>,
    pub(crate) reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Decision {
    Accepted,
    Rejected,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutputFormat {
    Text,
    JsonLines,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct CliRequest {
    pub(crate) runs_root: PathBuf,
    pub(crate) output_format: OutputFormat,
    pub(crate) command: CliCommand,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub(crate) enum CliCommand {
    Qualify {
        request: QualifyRequest,
    },
    Report {
        run_id: RunId,
    },
    Inspect {
        selector: TrialSelector,
    },
    Resume {
        run_id: RunId,
    },
    Decide {
        run_id: RunId,
        artifact: ArtifactName,
        decision: Decision,
        assignments: Vec<TierAssignment>,
        reason: Option<String>,
    },
    Apply {
        run_id: RunId,
        artifact: ArtifactName,
    },
    AuditBriefs {
        request: AuditBriefRequest,
    },
    Judge {
        request: PromptJudgeRequest,
    },
    PoolQualify {
        request: PoolQualifyRequest,
    },
    PoolReport {
        run_id: PoolRunId,
    },
    PoolResume {
        run_id: PoolRunId,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PoolQualifyRequest {
    pub(crate) plan_path: PathBuf,
    pub(crate) artifact_roots: Vec<PathBuf>,
    pub(crate) selected_tiers: Vec<Tier>,
    pub(crate) is_dry_run: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct QualifyRequest {
    pub(crate) artifact_roots: Vec<PathBuf>,
    pub(crate) change: Option<ArtifactChange>,
    pub(crate) policy: QualificationPolicy,
    pub(crate) is_dry_run: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AuditBriefRequest {
    pub(crate) artifact_roots: Vec<PathBuf>,
    pub(crate) output_root: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AuditBrief {
    pub(crate) artifact: ArtifactName,
    pub(crate) failure_modes: Vec<FailureCount>,
    pub(crate) reproductions: Vec<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FailureCount {
    pub(crate) failure_mode: String,
    pub(crate) count: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PromptJudgeRequest {
    pub(crate) prompt: String,
    pub(crate) candidate_model: Option<ModelIdentity>,
    pub(crate) timeout_seconds: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PromptJudgeResult {
    pub(crate) model: ModelIdentity,
    pub(crate) response: String,
    pub(crate) usage: TrialUsage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct TrialSelector {
    pub(crate) run_id: RunId,
    pub(crate) artifact: ArtifactName,
    pub(crate) tier: Tier,
    pub(crate) case: CaseId,
    pub(crate) attempt: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CandidateArtifact {
    pub(crate) key: TrialKey,
    pub(crate) model: ModelIdentity,
    pub(crate) harness: HarnessIdentity,
    pub(crate) artifact_path: PathBuf,
    pub(crate) transcript_path: PathBuf,
    pub(crate) usage: TrialUsage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct JudgeInput {
    pub(crate) candidate: CandidateArtifact,
    pub(crate) expect: String,
    pub(crate) rubric_path: PathBuf,
    pub(crate) checks: Vec<CheckResult>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct QualificationReport {
    pub(crate) run_id: RunId,
    pub(crate) mode: RunMode,
    pub(crate) change: Option<ArtifactChange>,
    pub(crate) status: RunStatus,
    pub(crate) discoveries: Vec<ArtifactDiscovery>,
    pub(crate) artifacts: Vec<ArtifactReport>,
    pub(crate) pause: Option<PauseReason>,
    pub(crate) total_usage: TrialUsage,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct ArtifactReport {
    pub(crate) artifact: ArtifactName,
    pub(crate) kind: ArtifactKind,
    pub(crate) required_destinations: Vec<TierDestination>,
    pub(crate) status: ArtifactStatus,
    pub(crate) review_reason: Option<String>,
    pub(crate) pending_candidates: Vec<CandidateArtifact>,
    pub(crate) reference: Option<TierEvidence>,
    pub(crate) tiers: Vec<TierEvidence>,
    pub(crate) boundary: Option<QualificationBoundary>,
    pub(crate) decision: Option<DecisionRecord>,
    pub(crate) publication_gate: Option<PublicationGate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SkillEvalError {
    InvalidArguments(String),
    InvalidConfiguration(String),
    Io {
        path: PathBuf,
        message: String,
    },
    InvalidEvent {
        line: u64,
        message: String,
    },
    Process {
        program: String,
        exit_code: Option<i32>,
        standard_error: String,
    },
    Quota {
        model: ModelIdentity,
        reset_at: Option<Timestamp>,
    },
    JudgeUnavailable {
        candidate: ModelIdentity,
        judge_tier: Tier,
    },
    Verification(String),
    NotFound(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SkillRoutingDecision {
    pub(crate) artifact: ArtifactName,
    pub(crate) target_tier: Tier,
    pub(crate) parent_responsibilities: Vec<ParentResponsibility>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParentResponsibility {
    HumanDecision,
    IrreversibleAction,
    FinalVerification,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum RunEvent {
    RunStarted {
        at: Timestamp,
        configuration: RunConfiguration,
    },
    TrialStarted {
        at: Timestamp,
        key: TrialKey,
        models: Vec<ModelIdentity>,
        harness: HarnessIdentity,
    },
    CandidateExecuted {
        at: Timestamp,
        candidate: CandidateArtifact,
    },
    TrialCompleted {
        at: Timestamp,
        record: TrialRecord,
    },
    TierEvaluated {
        at: Timestamp,
        artifact: ArtifactName,
        evidence: TierEvidence,
    },
    BoundaryFound {
        at: Timestamp,
        artifact: ArtifactName,
        boundary: QualificationBoundary,
    },
    ReviewRequired {
        at: Timestamp,
        artifact: ArtifactName,
        reason: String,
    },
    RunPaused {
        at: Timestamp,
        reason: PauseReason,
    },
    RunResumed {
        at: Timestamp,
    },
    DecisionRecorded {
        at: Timestamp,
        decision: DecisionRecord,
    },
    PublicationGateEvaluated {
        at: Timestamp,
        gate: PublicationGate,
    },
    DiscoveryCompleted {
        at: Timestamp,
        artifacts: Vec<ArtifactDiscovery>,
    },
}
