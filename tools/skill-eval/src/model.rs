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
    #[serde(default)]
    pub(crate) qualification_routes: BTreeMap<Tier, Vec<ModelIdentity>>,
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

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct ModelIdentity {
    pub(crate) tier: Tier,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) thinking: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct PoolRunId(pub(crate) String);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PoolStage {
    Calibration,
    Qualification,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PoolEntrant {
    pub(crate) model: ModelIdentity,
    pub(crate) thinking_levels: Vec<String>,
    #[serde(default)]
    pub(crate) retained_lower_thinking_level: Option<String>,
    #[serde(default)]
    pub(crate) candidate_timeout_seconds: Option<u32>,
    pub(crate) catalog_observed_at: Timestamp,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PoolPolicy {
    pub(crate) calibration_repeats_per_case: u16,
    pub(crate) qualification_repeats_per_case: u16,
    pub(crate) promotion_count: u8,
    pub(crate) minimum_score: u8,
    pub(crate) calibration_minimum_reliability_basis_points: u16,
    pub(crate) qualification_minimum_reliability_basis_points: u16,
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
    pub(crate) artifacts: Vec<ArtifactDefinition>,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ThinkingDecision {
    pub(crate) selected: Option<ModelIdentity>,
    #[serde(default)]
    pub(crate) retained_lower: Option<ModelIdentity>,
    pub(crate) next_thinking_index: Option<u8>,
    pub(crate) is_complete: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct RankedPool {
    pub(crate) tier: Tier,
    pub(crate) calibration: Vec<PoolEntrantEvidence>,
    pub(crate) thinking_selections: Vec<ModelIdentity>,
    #[serde(default)]
    pub(crate) retained_lower_routes: Vec<ModelIdentity>,
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
    Skipped,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PoolChildRun {
    pub(crate) tier: Tier,
    pub(crate) entrant_index: u8,
    pub(crate) thinking_index: u8,
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

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct FrontierCaseKey {
    pub(crate) artifact_path: PathBuf,
    pub(crate) artifact_revision: String,
    pub(crate) case: CaseId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierCaseInventoryEntry {
    pub(crate) key: FrontierCaseKey,
    pub(crate) drive: CaseDrive,
    pub(crate) is_holdout: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierSuiteInventory {
    pub(crate) version: u64,
    pub(crate) generated_at: Timestamp,
    pub(crate) cases: Vec<FrontierCaseInventoryEntry>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrontierCaseRejectionReason {
    ResponseOnly,
    RoutingOnly,
    MissingInput,
    ProseOnly,
    Blocked,
    SourceDrift,
    DuplicateLowerTier,
    AtOrBelowTier,
    Unreviewed,
    ReviewerDisagreement,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub(crate) enum FrontierCaseReviewDecision {
    Eligible {
        relative_difficulty_basis_points: u16,
        group: FrontierCaseGroup,
        is_confirmation: bool,
        evidence: Vec<String>,
    },
    Rejected {
        reason: FrontierCaseRejectionReason,
        evidence: Vec<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierCaseReviewRecord {
    pub(crate) key: FrontierCaseKey,
    pub(crate) reviewer: String,
    pub(crate) reviewed_at: Timestamp,
    pub(crate) decision: FrontierCaseReviewDecision,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierSuiteReviewSet {
    pub(crate) version: u64,
    pub(crate) inventory_sha256: String,
    pub(crate) records: Vec<FrontierCaseReviewRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierSuiteConstructionPolicy {
    pub(crate) required_tiers: Vec<Tier>,
    pub(crate) minimum_unique_cases_per_tier: u16,
    pub(crate) minimum_reviewers_per_case: u8,
    pub(crate) group_weights_basis_points: BTreeMap<FrontierCaseGroup, u16>,
    pub(crate) is_unanimous_eligibility_required: bool,
    pub(crate) is_cross_tier_reuse_allowed: bool,
    pub(crate) is_calibration_anchor_counted_toward_minimum: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierSuiteConstructionPlan {
    pub(crate) version: u64,
    pub(crate) artifact_roots: Vec<PathBuf>,
    pub(crate) policy: FrontierSuiteConstructionPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrontierSuiteProposalStatus {
    Ready,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierTierCapacity {
    pub(crate) required_unique_cases: u16,
    pub(crate) accepted_unique_cases: u16,
    pub(crate) shortfall: u16,
    pub(crate) duplicate_cases: u16,
    pub(crate) rejected_cases: u16,
    pub(crate) is_complete: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierSuiteProposal {
    pub(crate) version: u64,
    pub(crate) inventory_sha256: String,
    pub(crate) review_set_sha256: String,
    pub(crate) policy: FrontierSuiteConstructionPolicy,
    pub(crate) proposed_tiers: BTreeMap<Tier, FrontierTierSuite>,
    pub(crate) calibration_anchors: Vec<FrontierCaseKey>,
    pub(crate) holdout_cases: Vec<FrontierCaseKey>,
    pub(crate) tier_capacity: BTreeMap<Tier, FrontierTierCapacity>,
    pub(crate) status: FrontierSuiteProposalStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierSuitePublication {
    pub(crate) proposal_sha256: String,
    pub(crate) suite_path: PathBuf,
    pub(crate) suite_sha256: String,
    pub(crate) published_at: Timestamp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrontierCaseGroup {
    Normal,
    Edge,
    Adversarial,
    Critical,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierCaseReference {
    pub(crate) artifact_path: PathBuf,
    pub(crate) artifact_revision: String,
    pub(crate) case: CaseId,
    pub(crate) group: FrontierCaseGroup,
    pub(crate) is_confirmation: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierTierSuite {
    pub(crate) group_weights_basis_points: BTreeMap<FrontierCaseGroup, u16>,
    pub(crate) cases: Vec<FrontierCaseReference>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierSuite {
    pub(crate) version: u64,
    pub(crate) tiers: BTreeMap<Tier, FrontierTierSuite>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierSuiteIdentity {
    pub(crate) path: PathBuf,
    pub(crate) sha256: String,
    pub(crate) version: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierEntrant {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) entry_tier: Tier,
    pub(crate) thinking_levels: Vec<String>,
    pub(crate) catalog_observed_at: Timestamp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrontierConfidenceMethod {
    StratifiedBootstrap,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct FrontierPolicy {
    pub(crate) screening_trials_per_case: u8,
    pub(crate) confirmation_trials_per_case: u8,
    pub(crate) maximum_trials_per_case: u8,
    pub(crate) minimum_trial_score: u8,
    pub(crate) minimum_weighted_pass_basis_points: u16,
    pub(crate) minimum_lower_bound_basis_points: u16,
    pub(crate) confidence_level_basis_points: u16,
    pub(crate) confidence_method: FrontierConfidenceMethod,
    pub(crate) confidence_resamples: u32,
    pub(crate) maximum_infrastructure_attempts: u8,
    pub(crate) maximum_catalog_age_seconds: u32,
    pub(crate) active_pool_size: u8,
    pub(crate) maximum_trial_cost_millionths_of_dollar: u64,
    pub(crate) spending_limit_millionths_of_dollar: u64,
    pub(crate) is_provider_limit_enforced: bool,
    pub(crate) is_first_party_only: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct FrontierPlan {
    pub(crate) version: u64,
    pub(crate) suite: FrontierSuiteIdentity,
    pub(crate) capabilities: T1ScreenSnapshotIdentity,
    pub(crate) entrants: Vec<FrontierEntrant>,
    pub(crate) judge: ModelIdentity,
    pub(crate) policy: FrontierPolicy,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct FrontierRunId(pub(crate) String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierPreviewReport {
    pub(crate) plan_sha256: String,
    pub(crate) tier_case_counts: BTreeMap<Tier, u16>,
    pub(crate) route_count: u32,
    pub(crate) candidate_calls: T1ScreenCallRange,
    pub(crate) judge_calls: T1ScreenCallRange,
    pub(crate) maximum_spending_millionths_of_dollar: u64,
    pub(crate) is_owner_approval_required: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct FrontierRunConfiguration {
    pub(crate) run_id: FrontierRunId,
    pub(crate) created_at: Timestamp,
    pub(crate) plan_path: PathBuf,
    pub(crate) plan_sha256: String,
    pub(crate) plan: FrontierPlan,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrontierCellStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Indeterminate,
    Skipped,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierScore {
    pub(crate) weighted_pass_basis_points: u16,
    pub(crate) lower_bound_basis_points: u16,
    pub(crate) critical_passed_trials: u32,
    pub(crate) critical_expected_trials: u32,
    pub(crate) is_group_coverage_complete: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct FrontierCellEvidence {
    pub(crate) model: ModelIdentity,
    pub(crate) status: FrontierCellStatus,
    pub(crate) completed_trials: u32,
    pub(crate) expected_trials: u32,
    pub(crate) failed_trials: u32,
    pub(crate) score: Option<FrontierScore>,
    pub(crate) total_usage: TrialUsage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierModelProgress {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) entry_tier: Tier,
    pub(crate) selected_routes: Vec<ModelIdentity>,
    pub(crate) next_tier: Option<Tier>,
    pub(crate) next_thinking_index: Option<u8>,
    pub(crate) is_exhausted: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierInfrastructureEvent {
    pub(crate) model: ModelIdentity,
    pub(crate) artifact: ArtifactName,
    pub(crate) case: CaseId,
    pub(crate) attempt: u16,
    pub(crate) infrastructure_attempt: u8,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(crate) charged_millionths_of_dollar: u64,
    pub(crate) message: String,
    pub(crate) occurred_at: Timestamp,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrontierRunStatus {
    Pending,
    Running,
    Paused,
    AwaitingDecision,
    Accepted,
    Rejected,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct FrontierScheduledTrial {
    pub(crate) model: ModelIdentity,
    pub(crate) key: TrialKey,
    pub(crate) infrastructure_attempt: u8,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum FrontierScheduleAction {
    Dispatch {
        trials: Vec<FrontierScheduledTrial>,
        reserved_cost_per_trial_millionths_of_dollar: u64,
    },
    Pause {
        reason: PoolPauseReason,
    },
    Complete,
    Terminal {
        status: FrontierRunStatus,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierDecisionRecord {
    pub(crate) decision: Decision,
    pub(crate) reason: String,
    pub(crate) decided_at: Timestamp,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct FrontierRunState {
    pub(crate) configuration: FrontierRunConfiguration,
    pub(crate) status: FrontierRunStatus,
    pub(crate) models: Vec<FrontierModelProgress>,
    pub(crate) cells: Vec<FrontierCellEvidence>,
    pub(crate) infrastructure_events: Vec<FrontierInfrastructureEvent>,
    pub(crate) pause: Option<PoolPauseReason>,
    pub(crate) decision: Option<FrontierDecisionRecord>,
    pub(crate) spent_millionths_of_dollar: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierEvidenceIdentity {
    pub(crate) path: PathBuf,
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierPoolMembership {
    pub(crate) model: ModelIdentity,
    pub(crate) rank: u16,
    pub(crate) is_active: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct FrontierCapabilityEvidence {
    pub(crate) model: ModelIdentity,
    pub(crate) tag: String,
    pub(crate) capability_revision: String,
    pub(crate) score: ConfidenceInterval,
    pub(crate) evidence: FrontierEvidenceIdentity,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct FrontierBaseline {
    pub(crate) accepted_at: Timestamp,
    pub(crate) run_id: FrontierRunId,
    pub(crate) run_evidence: FrontierEvidenceIdentity,
    pub(crate) previous_entry_sha256: Option<String>,
    pub(crate) pools: BTreeMap<Tier, Vec<FrontierPoolMembership>>,
    pub(crate) capabilities: Vec<FrontierCapabilityEvidence>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct FrontierBaselineLedger {
    pub(crate) version: u64,
    pub(crate) baselines: Vec<FrontierBaseline>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrontierBaselineChange {
    Better,
    Worse,
    Unchanged,
    New,
    NotCompared,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct FrontierModelReport {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) supported_thinking_levels: Vec<String>,
    pub(crate) cells: Vec<FrontierCellEvidence>,
    pub(crate) highest_passing_tier: Option<Tier>,
    pub(crate) selected_routes: Vec<ModelIdentity>,
    pub(crate) pool_memberships: BTreeMap<Tier, FrontierPoolMembership>,
    pub(crate) baseline_change: FrontierBaselineChange,
    pub(crate) total_usage: TrialUsage,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct FrontierReport {
    pub(crate) run_id: FrontierRunId,
    pub(crate) status: FrontierRunStatus,
    pub(crate) models: Vec<FrontierModelReport>,
    pub(crate) pause: Option<PoolPauseReason>,
    pub(crate) decision: Option<FrontierDecisionRecord>,
    pub(crate) spent_millionths_of_dollar: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierApplyReport {
    pub(crate) run_id: FrontierRunId,
    pub(crate) active_routes: BTreeMap<Tier, Vec<ModelIdentity>>,
    pub(crate) is_changed: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum FrontierInspection {
    Trial { trial: TrialRecord },
    Infrastructure { event: FrontierInfrastructureEvent },
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
    #[serde(default)]
    pub(crate) route_index: u16,
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FrontierTrialJob {
    pub(crate) run_id: RunId,
    pub(crate) model: ModelIdentity,
    pub(crate) judge: ModelIdentity,
    pub(crate) key: TrialKey,
    pub(crate) artifact: ArtifactDefinition,
    pub(crate) case: CaseDefinition,
    pub(crate) harness: HarnessIdentity,
    pub(crate) infrastructure_attempt: u8,
    pub(crate) reserved_cost_millionths_of_dollar: u64,
}

#[derive(Debug, PartialEq)]
pub(crate) struct FrontierTrialOutcome {
    pub(crate) model: ModelIdentity,
    pub(crate) key: TrialKey,
    pub(crate) infrastructure_attempt: u8,
    pub(crate) result: Result<TrialRecord, SkillEvalError>,
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
    PoolCompleted,
    Paused,
    NeedsReview,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct RunState {
    pub(crate) run_id: RunId,
    pub(crate) mode: RunMode,
    pub(crate) purpose: QualificationPurpose,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum T1ScreenFormat {
    Text,
    Json,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct T1ScreenSnapshotIdentity {
    pub(crate) path: PathBuf,
    pub(crate) sha256: String,
    pub(crate) version: u64,
    pub(crate) observed_at_unix_seconds: u64,
    pub(crate) pi_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct T1ScreenEligibleRow {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) supported_pi_thinking_levels: Vec<String>,
    pub(crate) is_preview: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum T1ScreenExclusionReason {
    MissingList,
    MissingRpc,
    MovingAlias,
    NotExactEvidence,
    MovingRouterOrControl,
    MissingPrice,
    NonzeroInputPrice,
    NonzeroOutputPrice,
    MissingTextInput,
    MissingThinkingLevels,
    MalformedThinkingLevels,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct T1ScreenExcludedRow {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) is_preview: bool,
    pub(crate) reasons: Vec<T1ScreenExclusionReason>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct T1ScreenCallRange {
    pub(crate) minimum: u64,
    pub(crate) maximum: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct T1ScreenRunId(pub(crate) String);

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct T1ScreenCampaignId(pub(crate) String);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum T1ScreenRunStatus {
    Pending,
    Running,
    Paused,
    AwaitingOwner,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum T1ScreenChildStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Skipped,
    Exhausted,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub(crate) enum T1ScreenPauseReason {
    Quota {
        model: ModelIdentity,
        reset_at: Option<Timestamp>,
    },
    Infrastructure {
        message: String,
    },
    JudgeCap {
        spent_millionths_of_dollar: u64,
        owner_approved_millionths_of_dollar: u64,
        provider_enforced_millionths_of_dollar: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CandidateEnvironmentEntry {
    pub(crate) key: String,
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenCandidateEnvironment {
    pub(crate) harnesses: Vec<HarnessIdentity>,
    #[serde(default)]
    pub(crate) manifest: Vec<CandidateEnvironmentEntry>,
    pub(crate) digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenPolicy {
    pub(crate) minimum_score: u8,
    pub(crate) calibration_minimum_reliability_basis_points: u16,
    pub(crate) maximum_catastrophic_trials: u32,
    pub(crate) repeats_per_case: u16,
    #[serde(default)]
    pub(crate) candidate_timeout_seconds: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenCandidatePrice {
    pub(crate) input_per_million_tokens: u64,
    pub(crate) output_per_million_tokens: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenRunConfiguration {
    pub(crate) run_id: T1ScreenRunId,
    pub(crate) campaign_id: T1ScreenCampaignId,
    pub(crate) created_at: Timestamp,
    pub(crate) capability_snapshot: T1ScreenSnapshotIdentity,
    pub(crate) classification_sha256: String,
    pub(crate) eligible: Vec<T1ScreenEligibleRow>,
    pub(crate) excluded: Vec<T1ScreenExcludedRow>,
    pub(crate) exam: ArtifactDefinition,
    pub(crate) judge: ModelIdentity,
    pub(crate) candidate_environment: T1ScreenCandidateEnvironment,
    pub(crate) policy: T1ScreenPolicy,
    #[serde(default)]
    pub(crate) is_complete_thinking_coverage: bool,
    pub(crate) candidate_calls: T1ScreenCallRange,
    pub(crate) judge_calls: T1ScreenCallRange,
    pub(crate) candidate_price: T1ScreenCandidatePrice,
    pub(crate) owner_approved_judge_cap_millionths_of_dollar: u64,
    pub(crate) provider_enforced_judge_cap_millionths_of_dollar: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenChildRun {
    pub(crate) model: ModelIdentity,
    pub(crate) run_id: RunId,
    pub(crate) model_index: u64,
    pub(crate) thinking_index: u64,
    pub(crate) status: T1ScreenChildStatus,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenAttemptEvidence {
    pub(crate) child_run_id: RunId,
    pub(crate) evidence: PoolEntrantEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenRouteFailure {
    pub(crate) timestamp: Timestamp,
    pub(crate) child_run_id: RunId,
    pub(crate) model: ModelIdentity,
    pub(crate) paused_message_sha256: String,
    pub(crate) owner_reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub(crate) enum T1ScreenModelOutcome {
    Selected {
        model: ModelIdentity,
    },
    Exhausted,
    InfrastructureFailed {
        model: ModelIdentity,
        child_run_id: RunId,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenModelState {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) attempts: Vec<T1ScreenAttemptEvidence>,
    pub(crate) outcome: Option<T1ScreenModelOutcome>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenCapExtension {
    pub(crate) timestamp: Timestamp,
    pub(crate) previous_owner_cap_millionths_of_dollar: u64,
    pub(crate) new_owner_cap_millionths_of_dollar: u64,
    pub(crate) previous_provider_cap_millionths_of_dollar: u64,
    pub(crate) new_provider_cap_millionths_of_dollar: u64,
    pub(crate) owner_reason: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenRunState {
    pub(crate) configuration: T1ScreenRunConfiguration,
    #[serde(default)]
    pub(crate) cap_extensions: Vec<T1ScreenCapExtension>,
    #[serde(default)]
    pub(crate) route_failures: Vec<T1ScreenRouteFailure>,
    pub(crate) status: T1ScreenRunStatus,
    pub(crate) child_runs: Vec<T1ScreenChildRun>,
    pub(crate) models: Vec<T1ScreenModelState>,
    pub(crate) candidate_usage: TrialUsage,
    pub(crate) judge_usage: TrialUsage,
    pub(crate) spent_judge_millionths_of_dollar: u64,
    pub(crate) pause: Option<T1ScreenPauseReason>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum T1ScreenCampaignStatus {
    Open,
    Paused,
    Exhausted,
    AwaitingOwner,
    Closed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenCampaignRunEntry {
    pub(crate) run_id: T1ScreenRunId,
    pub(crate) canonical_state_path: PathBuf,
    pub(crate) state_file_sha256: String,
    pub(crate) created_at: Timestamp,
    pub(crate) observed_status: T1ScreenRunStatus,
    pub(crate) judge_spend_millionths_of_dollar: u64,
    pub(crate) candidate_cost_millionths_of_dollar: u64,
    pub(crate) is_resumable: bool,
    pub(crate) superseded_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenCampaignCapExtension {
    pub(crate) timestamp: Timestamp,
    pub(crate) previous_approved_total_millionths_of_dollar: u64,
    pub(crate) new_approved_total_millionths_of_dollar: u64,
    pub(crate) owner_reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenCampaignRunRetirement {
    pub(crate) timestamp: Timestamp,
    pub(crate) run_id: T1ScreenRunId,
    pub(crate) owner_reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenCampaignState {
    pub(crate) campaign_id: T1ScreenCampaignId,
    pub(crate) created_at: Timestamp,
    pub(crate) approved_judge_total_millionths_of_dollar: u64,
    #[serde(default)]
    pub(crate) cap_extensions: Vec<T1ScreenCampaignCapExtension>,
    #[serde(default)]
    pub(crate) retirements: Vec<T1ScreenCampaignRunRetirement>,
    pub(crate) aggregate_judge_spent_millionths_of_dollar: u64,
    pub(crate) runs: Vec<T1ScreenCampaignRunEntry>,
    pub(crate) active_run_id: Option<T1ScreenRunId>,
    pub(crate) owner_reason: String,
    pub(crate) status: T1ScreenCampaignStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenCampaignCreateRequest {
    pub(crate) campaign_id: T1ScreenCampaignId,
    pub(crate) judge_cap_millionths_of_dollar: u64,
    pub(crate) owner_reason: String,
    pub(crate) run_ids: Vec<T1ScreenRunId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenCampaignCapExtensionRequest {
    pub(crate) campaign_id: T1ScreenCampaignId,
    pub(crate) new_approved_total_millionths_of_dollar: u64,
    pub(crate) owner_reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenCampaignRunRetirementRequest {
    pub(crate) campaign_id: T1ScreenCampaignId,
    pub(crate) run_id: T1ScreenRunId,
    pub(crate) owner_reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct T1ScreenPreviewReport {
    pub(crate) snapshot: T1ScreenSnapshotIdentity,
    pub(crate) total_rows: u64,
    pub(crate) eligible_count: u64,
    pub(crate) excluded_count: u64,
    pub(crate) eligible: Vec<T1ScreenEligibleRow>,
    pub(crate) excluded: Vec<T1ScreenExcludedRow>,
    pub(crate) exam_case_count: u64,
    pub(crate) candidate_calls: T1ScreenCallRange,
    pub(crate) judge_calls: T1ScreenCallRange,
    pub(crate) projected_candidate_money_cost_usd: u8,
    pub(crate) is_judge_money_projected_from_candidate_price: bool,
    pub(crate) is_owner_approved_judge_cap_required_before_execution: bool,
    pub(crate) judge_money_note: String,
}

/// Defines the validated inputs for one T1 screening start.
///
/// The fields supply repository-relative capability and exam paths, positive judge caps, and the
/// optional run-identifier destination. The command parser produces this value. Invalid paths or
/// caps cause parsing or start validation to return an error.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct T1ScreenStartRequest {
    pub(crate) campaign_id: T1ScreenCampaignId,
    pub(crate) capabilities: PathBuf,
    pub(crate) exam: PathBuf,
    pub(crate) owner_approved_judge_cap_millionths_of_dollar: u64,
    pub(crate) provider_enforced_judge_cap_millionths_of_dollar: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenCapExtensionRequest {
    pub(crate) run_id: T1ScreenRunId,
    pub(crate) new_owner_cap_millionths_of_dollar: u64,
    pub(crate) new_provider_cap_millionths_of_dollar: u64,
    pub(crate) owner_reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct T1ScreenRouteFailureRequest {
    pub(crate) run_id: T1ScreenRunId,
    pub(crate) child_run_id: RunId,
    pub(crate) owner_reason: String,
}

/// Reports one fixed exam case for one exact T1 thinking attempt.
///
/// The case identifier is input. The optional candidate checkpoint and completed trial are the
/// output read from the child event log. This data-only value has no error behavior.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct T1ScreenCaseReport {
    pub(crate) case: CaseId,
    pub(crate) candidate: Option<CandidateArtifact>,
    pub(crate) trial: Option<TrialRecord>,
}

/// Reports one ordered thinking attempt and all five case slots.
///
/// The child identity and stored events are inputs. The output preserves status, aggregate
/// evidence, and per-case checkpoints. Report construction returns an error for inconsistent
/// child evidence.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct T1ScreenAttemptReport {
    pub(crate) child_run_id: RunId,
    pub(crate) model: ModelIdentity,
    pub(crate) status: T1ScreenChildStatus,
    pub(crate) evidence: Option<PoolEntrantEvidence>,
    pub(crate) cases: Vec<T1ScreenCaseReport>,
}

/// Reports all ordered attempts and the terminal outcome for one eligible base model.
///
/// The frozen model state and child logs are inputs. The output retains each attempt in canonical
/// thinking order. Report construction returns an error when identities or evidence disagree.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct T1ScreenModelReport {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) attempts: Vec<T1ScreenAttemptReport>,
    pub(crate) outcome: Option<T1ScreenModelOutcome>,
}

/// Supplies every value used by the deterministic T1 route comparator.
///
/// Passing aggregate evidence is the input. The output contains candidate-only cost, latency, and
/// failure counts. Report construction returns an error when candidate cost is nonzero.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct T1ScreenRankingInputs {
    pub(crate) candidate_cost_millionths_of_dollar: u64,
    pub(crate) candidate_latency_milliseconds: u64,
    pub(crate) candidate_failed_trials: u32,
    pub(crate) candidate_completed_trials: u32,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) thinking: String,
}

/// Reports one ranked exact route and its candidate-only comparator inputs.
///
/// A passing selected route is the input. The output supplies its one-based rank and frozen exact
/// identity. Report construction returns an error for malformed or nonzero-cost evidence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct T1ScreenRankedRoute {
    pub(crate) rank: u64,
    pub(crate) model: ModelIdentity,
    pub(crate) ranking_inputs: T1ScreenRankingInputs,
}

/// Reports the complete terminal T1 ranking or an explicit three-route shortage.
///
/// All terminal passing selections are inputs. The output has exactly three recommendations when
/// possible and every remaining route as an ordered alternate. Ranking returns an error for
/// invalid evidence or arithmetic overflow.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct T1ScreenRankingReport {
    pub(crate) passing_route_count: u64,
    pub(crate) recommendation_shortage_count: u8,
    pub(crate) recommendations: Vec<T1ScreenRankedRoute>,
    pub(crate) alternates: Vec<T1ScreenRankedRoute>,
}

/// Reports complete read-only T1 screening inventory, state, evidence, and owner gate data.
///
/// A stored parent snapshot and child event logs are inputs. The output preserves all
/// classifications, attempts, case evidence, usage, caps, and optional terminal ranking. Report
/// construction returns an error for corrupt, incomplete, or inconsistent stored evidence.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct T1ScreenReport {
    pub(crate) run_id: T1ScreenRunId,
    pub(crate) campaign_id: T1ScreenCampaignId,
    pub(crate) campaign_approved_judge_total_millionths_of_dollar: u64,
    pub(crate) campaign_aggregate_judge_spent_millionths_of_dollar: u64,
    pub(crate) campaign_remaining_judge_millionths_of_dollar: u64,
    pub(crate) campaign_runs: Vec<T1ScreenCampaignRunEntry>,
    pub(crate) campaign_active_run_id: Option<T1ScreenRunId>,
    pub(crate) campaign_status: T1ScreenCampaignStatus,
    pub(crate) created_at: Timestamp,
    pub(crate) status: T1ScreenRunStatus,
    pub(crate) snapshot: T1ScreenSnapshotIdentity,
    pub(crate) total_inventory_count: u64,
    pub(crate) eligible_count: u64,
    pub(crate) excluded_count: u64,
    pub(crate) eligible: Vec<T1ScreenEligibleRow>,
    pub(crate) excluded: Vec<T1ScreenExcludedRow>,
    pub(crate) exam: ArtifactDefinition,
    pub(crate) judge: ModelIdentity,
    pub(crate) candidate_environment: T1ScreenCandidateEnvironment,
    pub(crate) candidate_environment_manifest_digest: String,
    pub(crate) candidate_environment_manifest_entry_count: u64,
    pub(crate) policy: T1ScreenPolicy,
    pub(crate) candidate_calls: T1ScreenCallRange,
    pub(crate) judge_calls: T1ScreenCallRange,
    pub(crate) owner_approved_judge_cap_millionths_of_dollar: u64,
    pub(crate) provider_enforced_judge_cap_millionths_of_dollar: u64,
    pub(crate) effective_owner_approved_judge_cap_millionths_of_dollar: u64,
    pub(crate) effective_provider_enforced_judge_cap_millionths_of_dollar: u64,
    pub(crate) cap_extensions: Vec<T1ScreenCapExtension>,
    pub(crate) route_failures: Vec<T1ScreenRouteFailure>,
    pub(crate) spent_judge_millionths_of_dollar: u64,
    pub(crate) candidate_usage: TrialUsage,
    pub(crate) judge_usage: TrialUsage,
    pub(crate) active_child_run_id: Option<RunId>,
    pub(crate) pause: Option<T1ScreenPauseReason>,
    pub(crate) child_runs: Vec<T1ScreenChildRun>,
    pub(crate) models: Vec<T1ScreenModelReport>,
    pub(crate) ranking: Option<T1ScreenRankingReport>,
    pub(crate) is_owner_approval_required: bool,
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
    ModelCapabilities {
        output: PathBuf,
    },
    T1ScreenPreview {
        capabilities: PathBuf,
        format: T1ScreenFormat,
    },
    T1ScreenCampaignCreate {
        request: T1ScreenCampaignCreateRequest,
        format: T1ScreenFormat,
    },
    T1ScreenCampaignExtendCap {
        request: T1ScreenCampaignCapExtensionRequest,
        format: T1ScreenFormat,
    },
    T1ScreenCampaignRetireRun {
        request: T1ScreenCampaignRunRetirementRequest,
        format: T1ScreenFormat,
    },
    T1ScreenStart {
        request: T1ScreenStartRequest,
        format: T1ScreenFormat,
    },
    T1ScreenResume {
        run_id: T1ScreenRunId,
        format: T1ScreenFormat,
    },
    T1ScreenExtendCap {
        request: T1ScreenCapExtensionRequest,
        format: T1ScreenFormat,
    },
    T1ScreenFailRoute {
        request: T1ScreenRouteFailureRequest,
        format: T1ScreenFormat,
    },
    T1ScreenReport {
        run_id: T1ScreenRunId,
        format: T1ScreenFormat,
    },
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
    PoolReplacement {
        run_id: PoolRunId,
        entrant_index: u8,
    },
    FrontierSuiteInventory {
        plan_path: PathBuf,
        output: PathBuf,
    },
    FrontierSuitePropose {
        plan_path: PathBuf,
        inventory_path: PathBuf,
        review_set_path: PathBuf,
        output: PathBuf,
    },
    FrontierSuiteCheck {
        proposal_path: PathBuf,
    },
    FrontierSuiteApply {
        proposal_path: PathBuf,
        output: PathBuf,
    },
    FrontierPreview {
        plan_path: PathBuf,
    },
    FrontierStart {
        plan_path: PathBuf,
    },
    FrontierResume {
        run_id: FrontierRunId,
    },
    FrontierReport {
        run_id: FrontierRunId,
        baseline_path: Option<PathBuf>,
    },
    FrontierInspect {
        selector: FrontierTrialSelector,
    },
    FrontierDecide {
        request: FrontierDecisionRequest,
    },
    FrontierApply {
        run_id: FrontierRunId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierTrialSelector {
    pub(crate) run_id: FrontierRunId,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) tier: Tier,
    pub(crate) thinking: String,
    pub(crate) artifact: ArtifactName,
    pub(crate) case: CaseId,
    pub(crate) attempt: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FrontierDecisionRequest {
    pub(crate) run_id: FrontierRunId,
    pub(crate) decision: Decision,
    pub(crate) reason: String,
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
    pub(crate) route_index: u16,
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
    PoolChildCompleted {
        at: Timestamp,
        artifact: ArtifactName,
        tier: Tier,
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
