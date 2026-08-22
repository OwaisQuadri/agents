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

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct RunId(pub(crate) String);

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct SkillName(pub(crate) String);

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct CaseId(pub(crate) String);

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct Timestamp(pub(crate) String);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct RunConfiguration {
    pub(crate) run_id: RunId,
    pub(crate) skills: Vec<SkillDefinition>,
    pub(crate) policy: QualificationPolicy,
    pub(crate) created_at: Timestamp,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct QualificationPolicy {
    pub(crate) candidate_tiers: Vec<Tier>,
    pub(crate) reference_tier: Tier,
    pub(crate) judge_tier: Tier,
    pub(crate) repeats_per_case: u16,
    pub(crate) minimum_score: u8,
    pub(crate) noninferiority_margin: f64,
    pub(crate) confidence_level: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct SkillDefinition {
    pub(crate) name: SkillName,
    pub(crate) root: PathBuf,
    pub(crate) current_minimum_tier: Option<Tier>,
    pub(crate) target_tier: Option<Tier>,
    pub(crate) cases: Vec<CaseDefinition>,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct HarnessIdentity {
    pub(crate) runner_version: String,
    pub(crate) pi_version: String,
    pub(crate) skill_revision: String,
    pub(crate) tool_policy_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct TrialKey {
    pub(crate) skill: SkillName,
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
    pub(crate) usage: TrialUsage,
    pub(crate) verdict: TrialVerdict,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct TrialVerdict {
    pub(crate) score: u8,
    pub(crate) is_catastrophic: bool,
    pub(crate) failure_mode: Option<String>,
    pub(crate) checks: Vec<CheckResult>,
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct TierEvidence {
    pub(crate) tier: Tier,
    pub(crate) status: TierStatus,
    pub(crate) completed_trials: u32,
    pub(crate) expected_trials: u32,
    pub(crate) passed_trials: u32,
    pub(crate) score: ConfidenceInterval,
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
pub(crate) struct SkillQualificationState {
    pub(crate) status: SkillStatus,
    pub(crate) tiers: Vec<TierEvidence>,
    pub(crate) boundary: Option<QualificationBoundary>,
    pub(crate) review_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SkillStatus {
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
    pub(crate) status: RunStatus,
    pub(crate) skills: BTreeMap<SkillName, SkillQualificationState>,
    pub(crate) pause: Option<PauseReason>,
    pub(crate) decisions: BTreeMap<SkillName, DecisionRecord>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunStatus {
    Running,
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
    pub(crate) skill: SkillName,
    pub(crate) decision: Decision,
    pub(crate) reason: Option<String>,
    pub(crate) decided_at: Timestamp,
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
        skill: SkillName,
        decision: Decision,
        reason: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct QualifyRequest {
    pub(crate) skill_roots: Vec<PathBuf>,
    pub(crate) policy: QualificationPolicy,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct TrialSelector {
    pub(crate) run_id: RunId,
    pub(crate) skill: SkillName,
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
    pub(crate) status: RunStatus,
    pub(crate) skills: Vec<SkillReport>,
    pub(crate) total_usage: TrialUsage,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct SkillReport {
    pub(crate) skill: SkillName,
    pub(crate) status: SkillStatus,
    pub(crate) boundary: Option<QualificationBoundary>,
    pub(crate) decision: Option<DecisionRecord>,
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
    pub(crate) skill: SkillName,
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
        model: ModelIdentity,
        harness: HarnessIdentity,
    },
    TrialCompleted {
        at: Timestamp,
        record: TrialRecord,
    },
    TierEvaluated {
        at: Timestamp,
        skill: SkillName,
        evidence: TierEvidence,
    },
    BoundaryFound {
        at: Timestamp,
        skill: SkillName,
        boundary: QualificationBoundary,
    },
    ReviewRequired {
        at: Timestamp,
        skill: SkillName,
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
}
