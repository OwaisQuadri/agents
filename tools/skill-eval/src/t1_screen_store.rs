use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use crate::model::{
    CandidateEnvironmentEntry, ModelIdentity, PoolStage, RunId, SkillEvalError,
    T1ScreenAttemptEvidence, T1ScreenCampaignId, T1ScreenCampaignState, T1ScreenChildRun,
    T1ScreenChildStatus, T1ScreenEligibleRow, T1ScreenExcludedRow, T1ScreenModelOutcome,
    T1ScreenModelState, T1ScreenPauseReason, T1ScreenRunId, T1ScreenRunState, T1ScreenRunStatus,
    Tier, TrialUsage,
};
use crate::ports::{RunIdSource, T1ScreenStore};
use crate::t1_screen_campaign_store::FileT1ScreenCampaignStore;

const SCREENING_ROOT: [&str; 3] = [".map", "skill-eval", "t1-screening"];
const SNAPSHOT_NAME: &str = "state.json";
const LOCK_NAME: &str = ".state.lock";
const THINKING_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
const EXAM_CASE_COUNT: u64 = 5;
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Persists validated T1 screening snapshots below a repository root.
///
/// The input is a repository root. The output is a store restricted to
/// `.map/skill-eval/t1-screening`. Construction and operations return errors for unsafe paths,
/// invalid snapshots, or file input/output failures.
pub(crate) struct FileT1ScreenStore {
    screening_root: PathBuf,
    campaign_store: FileT1ScreenCampaignStore,
    #[cfg(test)]
    failure: Option<T1ScreenFailurePoint>,
}

impl FileT1ScreenStore {
    /// Creates a T1 screening store below `repository_root`.
    ///
    /// The input is a repository-root path. The output is a canonical, path-restricted store.
    /// It returns an error when the root cannot be created, resolved, or opened as a directory.
    pub(crate) fn new(repository_root: impl AsRef<Path>) -> Result<Self, SkillEvalError> {
        let repository_root = canonical_repository_root(repository_root.as_ref())?;
        let screening_root = create_contained_directory(&repository_root, &SCREENING_ROOT)?;
        let campaign_store = FileT1ScreenCampaignStore::new(&repository_root)?;
        Ok(Self {
            screening_root,
            campaign_store,
            #[cfg(test)]
            failure: None,
        })
    }

    /// Opens an existing T1 screening store without creating a path.
    ///
    /// The input is a repository root with an existing screening directory. The output is a
    /// read-capable path-restricted store. It returns an error for missing, unsafe, or non-directory
    /// roots and performs no write.
    pub(crate) fn open(repository_root: impl AsRef<Path>) -> Result<Self, SkillEvalError> {
        let repository_root = canonical_repository_root(repository_root.as_ref())?;
        let screening_root = existing_contained_directory(&repository_root, &SCREENING_ROOT)?;
        let campaign_store = FileT1ScreenCampaignStore::open(&repository_root)?;
        Ok(Self {
            screening_root,
            campaign_store,
            #[cfg(test)]
            failure: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_failure(
        repository_root: impl AsRef<Path>,
        failure: T1ScreenFailurePoint,
    ) -> Result<Self, SkillEvalError> {
        let mut store = Self::new(repository_root)?;
        store.failure = Some(failure);
        Ok(store)
    }

    /// Creates a new screening run from a fully preallocated initial snapshot.
    ///
    /// The input is an initial run state. The output is one durable snapshot. It returns an error
    /// for invalid state, a reused identifier, unsafe paths, or file input/output failures.
    pub(crate) fn create(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
        if !state.configuration.is_complete_thinking_coverage {
            return Err(invalid(
                "new T1 screening state must use complete thinking coverage",
            ));
        }
        validate_t1_screen_state(state)?;
        if state.status != T1ScreenRunStatus::Pending
            || state
                .child_runs
                .iter()
                .any(|child| child.status != T1ScreenChildStatus::Pending)
            || state
                .models
                .iter()
                .any(|model| !model.attempts.is_empty() || model.outcome.is_some())
            || state.candidate_usage != zero_usage()
            || state.judge_usage != zero_usage()
            || state.spent_judge_millionths_of_dollar != 0
            || state.pause.is_some()
            || !state.cap_extensions.is_empty()
            || !state.route_failures.is_empty()
        {
            return Err(invalid(
                "new T1 screening state must be entirely pending and unspent",
            ));
        }
        let directory = self.run_directory(&state.configuration.run_id)?;
        fs::create_dir(&directory).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                invalid(format!(
                    "T1 screening run {:?} already exists",
                    state.configuration.run_id.0
                ))
            } else {
                io_error(&directory, error)
            }
        })?;
        let snapshot = directory.join(SNAPSHOT_NAME);
        let screening_root = self.screening_root.clone();
        let result = self
            .replace_snapshot(&directory, &snapshot, state, None)
            .and_then(|()| self.sync_directory(&screening_root));
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&directory);
            let _ = File::open(&self.screening_root).and_then(|file| file.sync_all());
            return Err(error);
        }
        Ok(())
    }

    /// Loads and validates one screening snapshot.
    ///
    /// The input is a safe run identifier. The output is its stored state. It returns an error for
    /// missing runs, malformed or unknown data, unsafe paths, or invalid state.
    pub(crate) fn load(&self, run_id: &T1ScreenRunId) -> Result<T1ScreenRunState, SkillEvalError> {
        self.read_snapshot(run_id).map(|(state, _)| state)
    }

    /// Atomically replaces one screening snapshot after monotonic transition validation.
    ///
    /// The input is the next complete run state. The output is a durable replacement preserving
    /// prior bytes on failure. It returns an error for drift, rollback, unsafe state, or file
    /// input/output failures.
    pub(crate) fn save(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
        validate_t1_screen_state(state)?;
        let directory = self.existing_run_directory(&state.configuration.run_id)?;
        let _lock = RunLock::acquire(&directory)?;
        let (stored, prior_bytes) = self.read_snapshot(&state.configuration.run_id)?;
        validate_t1_screen_transition(&stored, state)?;
        let snapshot = directory.join(SNAPSHOT_NAME);
        self.replace_snapshot(&directory, &snapshot, state, Some(&prior_bytes))
    }

    fn run_directory(&self, run_id: &T1ScreenRunId) -> Result<PathBuf, SkillEvalError> {
        validate_identifier(&run_id.0, "T1 screening run")?;
        Ok(self.screening_root.join(&run_id.0))
    }

    fn existing_run_directory(&self, run_id: &T1ScreenRunId) -> Result<PathBuf, SkillEvalError> {
        let expected = self.run_directory(run_id)?;
        let canonical = fs::canonicalize(&expected).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SkillEvalError::NotFound(format!("T1 screening run {:?} does not exist", run_id.0))
            } else {
                io_error(&expected, error)
            }
        })?;
        if canonical != expected || !canonical.is_dir() {
            return Err(invalid(format!(
                "T1 screening run {:?} escapes the configured screening root",
                run_id.0
            )));
        }
        Ok(canonical)
    }

    fn read_snapshot(
        &self,
        run_id: &T1ScreenRunId,
    ) -> Result<(T1ScreenRunState, Vec<u8>), SkillEvalError> {
        let path = self.existing_run_directory(run_id)?.join(SNAPSHOT_NAME);
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SkillEvalError::NotFound(format!("T1 screening run {:?} has no snapshot", run_id.0))
            } else {
                io_error(&path, error)
            }
        })?;
        if !metadata.file_type().is_file() {
            return Err(invalid(format!(
                "T1 screening snapshot {} is not a regular file",
                path.display()
            )));
        }
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            invalid(format!(
                "T1 screening snapshot {} is malformed: {error}",
                path.display()
            ))
        })?;
        if let Some(object) = value.as_object_mut() {
            object
                .entry("cap_extensions")
                .or_insert_with(|| serde_json::Value::Array(Vec::new()));
            object
                .entry("route_failures")
                .or_insert_with(|| serde_json::Value::Array(Vec::new()));
            if let Some(configuration) = object
                .get_mut("configuration")
                .and_then(serde_json::Value::as_object_mut)
            {
                configuration
                    .entry("is_complete_thinking_coverage")
                    .or_insert(serde_json::Value::Bool(false));
                if let Some(environment) = configuration
                    .get_mut("candidate_environment")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    environment
                        .entry("manifest")
                        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
                }
                if let Some(policy) = configuration
                    .get_mut("policy")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    policy
                        .entry("candidate_timeout_seconds")
                        .or_insert(serde_json::Value::Null);
                }
            }
        }
        let state: T1ScreenRunState = serde_json::from_value(value.clone()).map_err(|error| {
            invalid(format!(
                "T1 screening snapshot {} is malformed: {error}",
                path.display()
            ))
        })?;
        let normalized = serde_json::to_value(&state).map_err(|error| {
            invalid(format!(
                "T1 screening snapshot {} cannot be validated: {error}",
                path.display()
            ))
        })?;
        if normalized != value {
            return Err(invalid(format!(
                "T1 screening snapshot {} contains unknown data",
                path.display()
            )));
        }
        if state.configuration.run_id != *run_id {
            return Err(invalid(
                "T1 screening snapshot identity differs from its path",
            ));
        }
        validate_t1_screen_state(&state)?;
        Ok((state, bytes))
    }

    fn write_temporary(
        &mut self,
        directory: &Path,
        state: &T1ScreenRunState,
    ) -> Result<PathBuf, SkillEvalError> {
        let bytes = serde_json::to_vec_pretty(state).map_err(|error| {
            invalid(format!(
                "T1 screening snapshot serialization failed: {error}"
            ))
        })?;
        let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            ".{SNAPSHOT_NAME}.{}.{sequence}.tmp",
            std::process::id()
        ));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|error| io_error(&path, error))?;
        let result = (|| {
            self.fail(T1ScreenFailurePoint::Write, &path)?;
            file.write_all(&bytes)
                .and_then(|()| file.write_all(b"\n"))
                .map_err(|error| io_error(&path, error))?;
            self.fail(T1ScreenFailurePoint::FileSync, &path)?;
            file.sync_all().map_err(|error| io_error(&path, error))
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(path)
    }

    fn replace_snapshot(
        &mut self,
        directory: &Path,
        snapshot: &Path,
        state: &T1ScreenRunState,
        prior_bytes: Option<&[u8]>,
    ) -> Result<(), SkillEvalError> {
        let temporary = self.write_temporary(directory, state)?;
        let mut is_replaced = false;
        let result = (|| {
            self.fail(T1ScreenFailurePoint::Rename, snapshot)?;
            fs::rename(&temporary, snapshot).map_err(|error| io_error(snapshot, error))?;
            is_replaced = true;
            self.sync_directory(directory)
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            if is_replaced && let Some(bytes) = prior_bytes {
                restore_snapshot(directory, snapshot, bytes)?;
            }
            return Err(error);
        }
        Ok(())
    }

    fn sync_directory(&mut self, directory: &Path) -> Result<(), SkillEvalError> {
        self.fail(T1ScreenFailurePoint::DirectorySync, directory)?;
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|error| io_error(directory, error))
    }

    #[cfg(test)]
    fn fail(&mut self, point: T1ScreenFailurePoint, path: &Path) -> Result<(), SkillEvalError> {
        if self.failure == Some(point) {
            self.failure = None;
            return Err(SkillEvalError::Io {
                path: path.to_path_buf(),
                message: format!("injected {point:?} failure"),
            });
        }
        Ok(())
    }

    #[cfg(not(test))]
    fn fail(&mut self, _point: T1ScreenFailurePoint, _path: &Path) -> Result<(), SkillEvalError> {
        Ok(())
    }
}

impl T1ScreenStore for FileT1ScreenStore {
    fn create_t1_screen(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
        self.create(state)
    }

    fn load_t1_screen(&self, run_id: &T1ScreenRunId) -> Result<T1ScreenRunState, SkillEvalError> {
        self.load(run_id)
    }

    fn save_t1_screen(&mut self, state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
        self.save(state)
    }

    fn load_t1_screen_campaign(
        &self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        self.campaign_store.load(campaign_id)
    }

    fn reconcile_t1_screen_campaign(
        &mut self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        self.campaign_store.reconcile(campaign_id)
    }

    fn pause_t1_screen_campaign_for_budget(
        &mut self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        self.campaign_store.pause_for_budget(campaign_id)
    }

    fn register_t1_screen_campaign_run(
        &mut self,
        state: &T1ScreenRunState,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        self.campaign_store.register_active_run(state)
    }

    fn reconcile_t1_screen_campaign_run(
        &mut self,
        state: &T1ScreenRunState,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        self.campaign_store.reconcile_active_run(state)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum T1ScreenFailurePoint {
    Write,
    FileSync,
    Rename,
    DirectorySync,
}

struct RunLock {
    path: PathBuf,
}

impl RunLock {
    fn acquire(directory: &Path) -> Result<Self, SkillEvalError> {
        let path = directory.join(LOCK_NAME);
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .and_then(|mut file| {
                writeln!(file, "{}", std::process::id())?;
                file.sync_all()
            })
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    invalid("T1 screening run has a concurrent writer")
                } else {
                    io_error(&path, error)
                }
            })?;
        Ok(Self { path })
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(directory) = self.path.parent() {
            let _ = File::open(directory).and_then(|file| file.sync_all());
        }
    }
}

/// Preallocates one stable pending child for every eligible model and thinking level.
///
/// The inputs are the ordered T114 eligible rows and an injected identifier source. The output is
/// the complete child list in model and canonical thinking order. It returns an error for invalid
/// classification order, unsafe or duplicate identifiers, overflow, or source failure.
pub(crate) fn preallocate_t1_screen_children(
    eligible: &[T1ScreenEligibleRow],
    run_ids: &mut dyn RunIdSource,
) -> Result<Vec<T1ScreenChildRun>, SkillEvalError> {
    validate_eligible(eligible)?;
    let mut children = Vec::new();
    let mut identifiers = BTreeSet::new();
    for (model_index, row) in eligible.iter().enumerate() {
        let model_index = u64::try_from(model_index)
            .map_err(|_| invalid("T1 screening model count exceeds the supported range"))?;
        for (thinking_index, thinking) in row.supported_pi_thinking_levels.iter().enumerate() {
            let thinking_index = u64::try_from(thinking_index)
                .map_err(|_| invalid("T1 screening thinking count exceeds the supported range"))?;
            let run_id = run_ids.next()?;
            validate_identifier(&run_id.0, "T1 screening child")?;
            if !identifiers.insert(run_id.0.clone()) {
                return Err(invalid(
                    "T1 screening child identifiers contain a duplicate or collision",
                ));
            }
            children.push(T1ScreenChildRun {
                model: ModelIdentity {
                    tier: Tier::T1,
                    provider: row.provider.clone(),
                    model: row.model.clone(),
                    thinking: thinking.clone(),
                },
                run_id,
                model_index,
                thinking_index,
                status: T1ScreenChildStatus::Pending,
            });
        }
    }
    Ok(children)
}

/// Computes the exact digest of ordered eligible and excluded T114 lists.
///
/// The inputs are both complete ordered lists. The output is a lowercase SHA-256 digest. It
/// returns an error when serialization fails.
pub(crate) fn t1_screen_classification_digest(
    eligible: &[T1ScreenEligibleRow],
    excluded: &[T1ScreenExcludedRow],
) -> Result<String, SkillEvalError> {
    let bytes = serde_json::to_vec(&(eligible, excluded)).map_err(|error| {
        invalid(format!(
            "T1 screening classification serialization failed: {error}"
        ))
    })?;
    Ok(hex_digest(&bytes))
}

pub(crate) fn candidate_environment_manifest_digest(
    manifest: &[CandidateEnvironmentEntry],
) -> Result<String, SkillEvalError> {
    let bytes = serde_json::to_vec(manifest).map_err(|error| {
        invalid(format!(
            "T1 candidate environment manifest serialization failed: {error}"
        ))
    })?;
    Ok(hex_digest(&bytes))
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Validates a complete T1 screening snapshot without external calls or mutable state.
///
/// The input is a state. The output is unit on success. It returns an error for malformed frozen
/// configuration, child allocation, evidence, usage, spend, or status.
pub(crate) fn validate_t1_screen_state(state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
    validate_configuration(state)?;
    t1_screen_effective_caps(state)?;
    validate_children(state)?;
    if state
        .child_runs
        .iter()
        .filter(|child| child.status == T1ScreenChildStatus::Running)
        .count()
        > 1
    {
        return Err(invalid("T1 screening state has more than one active child"));
    }
    validate_models(state)?;
    validate_route_failures(state)?;
    validate_pause(state)?;
    validate_usage_and_spend(state)?;
    Ok(())
}

/// Validates a saved T1 screening state transition without external calls or mutation.
///
/// The inputs are prior and next snapshots. The output is unit on success. It returns an error for
/// frozen drift, identity changes, rollback, non-append evidence, or decreasing usage and spend.
pub(crate) fn validate_t1_screen_transition(
    stored: &T1ScreenRunState,
    next: &T1ScreenRunState,
) -> Result<(), SkillEvalError> {
    if stored.cap_extensions != next.cap_extensions {
        return validate_cap_extension_transition(stored, next);
    }
    if stored.route_failures != next.route_failures {
        return validate_route_failure_transition(stored, next);
    }
    if stored.configuration != next.configuration {
        return Err(invalid(
            "T1 screening frozen configuration changed after creation",
        ));
    }
    if stored.child_runs.len() != next.child_runs.len() {
        return Err(invalid(
            "T1 screening child identities changed after creation",
        ));
    }
    for (old, new) in stored.child_runs.iter().zip(&next.child_runs) {
        if child_identity(old) != child_identity(new) {
            return Err(invalid(
                "T1 screening child identities or order changed after creation",
            ));
        }
        if old.status != new.status && !is_legal_child_transition(old.status, new.status) {
            return Err(invalid("T1 screening child status transition is illegal"));
        }
    }
    if stored.status != next.status && !is_legal_run_transition(stored.status, next.status) {
        return Err(invalid("T1 screening run status transition is illegal"));
    }
    if stored.status == next.status && stored.pause != next.pause {
        return Err(invalid(
            "T1 screening pause changed without a status transition",
        ));
    }
    if stored.models.len() != next.models.len() {
        return Err(invalid("T1 screening model state identity changed"));
    }
    for (old, new) in stored.models.iter().zip(&next.models) {
        if old.provider != new.provider
            || old.model != new.model
            || !new.attempts.starts_with(&old.attempts)
            || old
                .outcome
                .as_ref()
                .is_some_and(|outcome| new.outcome.as_ref() != Some(outcome))
        {
            return Err(invalid(
                "T1 screening evidence is not append-only or changed identity",
            ));
        }
    }
    if !usage_is_nondecreasing(&stored.candidate_usage, &next.candidate_usage)
        || !usage_is_nondecreasing(&stored.judge_usage, &next.judge_usage)
        || next.spent_judge_millionths_of_dollar < stored.spent_judge_millionths_of_dollar
    {
        return Err(invalid("T1 screening usage and spend cannot decrease"));
    }
    Ok(())
}

fn validate_configuration(state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
    let configuration = &state.configuration;
    validate_identifier(&configuration.run_id.0, "T1 screening run")?;
    validate_identifier(&configuration.campaign_id.0, "T1 screening campaign")?;
    validate_t1_timestamp(&configuration.created_at)?;
    validate_canonical_absolute_path(
        &configuration.capability_snapshot.path,
        "T1 capability snapshot",
    )?;
    validate_digest(
        &configuration.capability_snapshot.sha256,
        "T1 capability snapshot",
    )?;
    if configuration.capability_snapshot.version == 0
        || configuration.capability_snapshot.observed_at_unix_seconds == 0
        || configuration
            .capability_snapshot
            .pi_version
            .trim()
            .is_empty()
    {
        return Err(invalid("T1 capability snapshot identity is incomplete"));
    }
    validate_eligible(&configuration.eligible)?;
    validate_excluded(&configuration.excluded)?;
    validate_classification_identity(configuration)?;
    if configuration.classification_sha256
        != t1_screen_classification_digest(&configuration.eligible, &configuration.excluded)?
    {
        return Err(invalid("T1 screening classification digest differs"));
    }
    validate_exam(state)?;
    validate_model_identity(&configuration.judge, "T1 screening judge")?;
    let environment = &configuration.candidate_environment;
    validate_candidate_environment_manifest(environment)?;
    if environment.harnesses.len() != 5 {
        return Err(invalid(
            "T1 candidate environment must contain five ordered harness identities",
        ));
    }
    let first_harness = &environment.harnesses[0];
    if first_harness.runner_version.trim().is_empty()
        || environment.harnesses.iter().any(|harness| {
            harness.runner_version != first_harness.runner_version
                || harness.pi_version != configuration.capability_snapshot.pi_version
                || harness.pi_version != first_harness.pi_version
                || harness.artifact_revision != configuration.exam.revision
                || harness.artifact_revision != first_harness.artifact_revision
                || harness.tool_policy_digest.trim().is_empty()
        })
    {
        return Err(invalid("T1 candidate environment identity is incomplete"));
    }
    if configuration.policy.minimum_score != 8
        || configuration
            .policy
            .calibration_minimum_reliability_basis_points
            != 8_000
        || configuration.policy.maximum_catastrophic_trials != 0
        || configuration.policy.repeats_per_case != 1
        || configuration.policy.candidate_timeout_seconds == Some(0)
    {
        return Err(invalid("T1 screening thresholds are not fixed"));
    }
    if configuration.candidate_price.input_per_million_tokens != 0
        || configuration.candidate_price.output_per_million_tokens != 0
    {
        return Err(invalid("T1 screening candidate price must be zero"));
    }
    if configuration.owner_approved_judge_cap_millionths_of_dollar == 0
        || configuration.provider_enforced_judge_cap_millionths_of_dollar == 0
    {
        return Err(invalid("T1 screening judge caps must be positive"));
    }
    if configuration.provider_enforced_judge_cap_millionths_of_dollar
        > configuration.owner_approved_judge_cap_millionths_of_dollar
    {
        return Err(invalid(
            "T1 screening provider judge cap exceeds the owner judge cap",
        ));
    }
    validate_call_projection(state)
}

fn validate_candidate_environment_manifest(
    environment: &crate::model::T1ScreenCandidateEnvironment,
) -> Result<(), SkillEvalError> {
    if environment.manifest.is_empty() {
        return Err(invalid("legacy candidate environment manifest missing"));
    }
    let mut previous = None::<&str>;
    for entry in &environment.manifest {
        if entry.key.trim().is_empty()
            || entry.key.contains('\0')
            || entry.key.chars().any(char::is_control)
        {
            return Err(invalid(
                "T1 candidate environment manifest contains an invalid key",
            ));
        }
        if previous.is_some_and(|prior| prior >= entry.key.as_str()) {
            return Err(invalid(
                "T1 candidate environment manifest entries are duplicate or unsorted",
            ));
        }
        validate_digest(&entry.sha256, "T1 candidate environment manifest entry")?;
        previous = Some(&entry.key);
    }
    validate_digest(&environment.digest, "T1 candidate environment manifest")?;
    if environment.digest != candidate_environment_manifest_digest(&environment.manifest)? {
        return Err(invalid("T1 candidate environment manifest digest differs"));
    }
    Ok(())
}

pub(crate) fn t1_screen_effective_caps(
    state: &T1ScreenRunState,
) -> Result<(u64, u64), SkillEvalError> {
    let mut owner = state
        .configuration
        .owner_approved_judge_cap_millionths_of_dollar;
    let mut provider = state
        .configuration
        .provider_enforced_judge_cap_millionths_of_dollar;
    for extension in &state.cap_extensions {
        validate_t1_timestamp(&extension.timestamp)?;
        if extension.owner_reason.trim().is_empty() {
            return Err(invalid("T1 screening cap extension owner reason is blank"));
        }
        if extension.previous_owner_cap_millionths_of_dollar != owner
            || extension.previous_provider_cap_millionths_of_dollar != provider
        {
            return Err(invalid(
                "T1 screening cap extension does not name the prior effective caps",
            ));
        }
        if extension.new_owner_cap_millionths_of_dollar <= owner
            || extension.new_provider_cap_millionths_of_dollar <= provider
        {
            return Err(invalid(
                "T1 screening cap extension must strictly increase both caps",
            ));
        }
        if extension.new_provider_cap_millionths_of_dollar
            > extension.new_owner_cap_millionths_of_dollar
        {
            return Err(invalid(
                "T1 screening cap extension provider cap exceeds owner cap",
            ));
        }
        owner = extension.new_owner_cap_millionths_of_dollar;
        provider = extension.new_provider_cap_millionths_of_dollar;
    }
    Ok((owner, provider))
}

fn validate_route_failures(state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
    let mut authority_timestamps = BTreeSet::new();
    let mut previous_cap = state.configuration.created_at.0.as_str();
    for extension in &state.cap_extensions {
        validate_t1_timestamp(&extension.timestamp)?;
        if extension.timestamp.0.as_str() <= previous_cap
            || !authority_timestamps.insert(extension.timestamp.0.as_str())
        {
            return Err(invalid(
                "T1 screening cap extension timestamps are not strictly ordered",
            ));
        }
        previous_cap = extension.timestamp.0.as_str();
    }

    let mut previous_failure = state.configuration.created_at.0.as_str();
    for (failure_index, failure) in state.route_failures.iter().enumerate() {
        validate_t1_timestamp(&failure.timestamp)?;
        if failure.timestamp.0.as_str() <= previous_failure
            || !authority_timestamps.insert(failure.timestamp.0.as_str())
        {
            return Err(invalid(
                "T1 screening authority timestamps are not strictly ordered and unique",
            ));
        }
        previous_failure = failure.timestamp.0.as_str();
        validate_identifier(&failure.child_run_id.0, "T1 screening failed child")?;
        validate_model_identity(&failure.model, "T1 screening failed route")?;
        validate_digest(
            &failure.paused_message_sha256,
            "T1 screening paused message",
        )?;
        if failure.owner_reason.trim().is_empty() {
            return Err(invalid("T1 screening route failure owner reason is blank"));
        }
        if state.route_failures[..failure_index]
            .iter()
            .any(|prior| prior.child_run_id == failure.child_run_id)
        {
            return Err(invalid(
                "T1 screening route failure repeats a child identifier",
            ));
        }
        if state.route_failures[..failure_index]
            .iter()
            .any(|prior| prior.model == failure.model)
        {
            return Err(invalid("T1 screening route failure repeats an exact route"));
        }
        let child = state
            .child_runs
            .iter()
            .find(|child| child.run_id == failure.child_run_id)
            .ok_or_else(|| invalid("T1 screening route failure child does not exist"))?;
        if child.model != failure.model || child.status != T1ScreenChildStatus::Failed {
            return Err(invalid(
                "T1 screening route failure differs from its failed child",
            ));
        }
        let model_index = usize::try_from(child.model_index)
            .map_err(|_| invalid("T1 screening route failure model index overflowed"))?;
        let thinking_index = usize::try_from(child.thinking_index)
            .map_err(|_| invalid("T1 screening route failure thinking index overflowed"))?;
        let model = state
            .models
            .get(model_index)
            .ok_or_else(|| invalid("T1 screening route failure model does not exist"))?;
        if model.attempts.len() != thinking_index
            || !matches!(
                &model.outcome,
                Some(T1ScreenModelOutcome::InfrastructureFailed {
                    model,
                    child_run_id,
                }) if model == &failure.model && child_run_id == &failure.child_run_id
            )
            || state.child_runs.iter().any(|sibling| {
                sibling.model_index == child.model_index
                    && match sibling.thinking_index.cmp(&child.thinking_index) {
                        std::cmp::Ordering::Less => {
                            sibling.status != T1ScreenChildStatus::Completed
                        }
                        std::cmp::Ordering::Equal => sibling.status != T1ScreenChildStatus::Failed,
                        std::cmp::Ordering::Greater => {
                            sibling.status != T1ScreenChildStatus::Skipped
                        }
                    }
            })
        {
            return Err(invalid(
                "T1 screening route failure progression or model outcome differs",
            ));
        }
    }

    for model in &state.models {
        if let Some(T1ScreenModelOutcome::InfrastructureFailed {
            model: failed_model,
            child_run_id,
        }) = &model.outcome
            && state
                .route_failures
                .iter()
                .filter(|failure| {
                    failure.child_run_id == *child_run_id && failure.model == *failed_model
                })
                .count()
                != 1
        {
            return Err(invalid(
                "T1 screening infrastructure-failed outcome has no exact authority record",
            ));
        }
    }
    Ok(())
}

fn validate_route_failure_transition(
    stored: &T1ScreenRunState,
    next: &T1ScreenRunState,
) -> Result<(), SkillEvalError> {
    let expected_length = stored
        .route_failures
        .len()
        .checked_add(1)
        .ok_or_else(|| invalid("T1 screening route failure history overflowed"))?;
    if next.route_failures.len() != expected_length
        || !next.route_failures.starts_with(&stored.route_failures)
    {
        return Err(invalid(
            "T1 screening route failure history is not one exact append",
        ));
    }
    let failure = next
        .route_failures
        .last()
        .expect("one route failure was appended");
    if failure.timestamp.0 <= stored.configuration.created_at.0
        || stored
            .cap_extensions
            .iter()
            .any(|extension| extension.timestamp.0 >= failure.timestamp.0)
        || stored
            .route_failures
            .iter()
            .any(|prior| prior.timestamp.0 >= failure.timestamp.0)
    {
        return Err(invalid(
            "T1 screening route failure timestamp is not globally later",
        ));
    }
    let paused_message = match &stored.pause {
        Some(T1ScreenPauseReason::Infrastructure { message }) => message,
        _ => {
            return Err(invalid(
                "T1 screening route failure requires an infrastructure pause",
            ));
        }
    };
    if stored.status != T1ScreenRunStatus::Paused
        || next.status != T1ScreenRunStatus::Running
        || next.pause.is_some()
        || hex_digest(paused_message.as_bytes()) != failure.paused_message_sha256
    {
        return Err(invalid(
            "T1 screening route failure requires one infrastructure pause to resume",
        ));
    }
    let paused = stored
        .child_runs
        .iter()
        .filter(|child| child.status == T1ScreenChildStatus::Paused)
        .collect::<Vec<_>>();
    if paused.len() != 1
        || paused[0].run_id != failure.child_run_id
        || paused[0].model != failure.model
    {
        return Err(invalid(
            "T1 screening route failure does not name the one paused child",
        ));
    }
    let failed_child = paused[0];
    let model_index = usize::try_from(failed_child.model_index)
        .map_err(|_| invalid("T1 screening route failure model index overflowed"))?;
    if stored.models[model_index].outcome.is_some() {
        return Err(invalid(
            "T1 screening route failure model already has an outcome",
        ));
    }

    let mut expected = stored.clone();
    expected.route_failures.clone_from(&next.route_failures);
    expected.status = T1ScreenRunStatus::Running;
    expected.pause = None;
    let child_index = expected
        .child_runs
        .iter()
        .position(|child| child.run_id == failure.child_run_id)
        .expect("paused child is preallocated");
    expected.child_runs[child_index].status = T1ScreenChildStatus::Failed;
    for sibling in expected.child_runs.iter_mut().filter(|sibling| {
        sibling.model_index == failed_child.model_index
            && sibling.thinking_index > failed_child.thinking_index
            && sibling.status == T1ScreenChildStatus::Pending
    }) {
        sibling.status = T1ScreenChildStatus::Skipped;
    }
    expected.models[model_index].outcome = Some(T1ScreenModelOutcome::InfrastructureFailed {
        model: failure.model.clone(),
        child_run_id: failure.child_run_id.clone(),
    });
    if expected != *next {
        return Err(invalid(
            "T1 screening route failure changed state outside the exact route append",
        ));
    }
    Ok(())
}

fn validate_cap_extension_transition(
    stored: &T1ScreenRunState,
    next: &T1ScreenRunState,
) -> Result<(), SkillEvalError> {
    let expected_length = stored
        .cap_extensions
        .len()
        .checked_add(1)
        .ok_or_else(|| invalid("T1 screening cap extension history overflowed"))?;
    if next.cap_extensions.len() != expected_length
        || !next.cap_extensions.starts_with(&stored.cap_extensions)
    {
        return Err(invalid(
            "T1 screening cap extension history is not one exact append",
        ));
    }
    let extension = next
        .cap_extensions
        .last()
        .expect("one cap extension was appended");
    if stored
        .route_failures
        .iter()
        .any(|failure| failure.timestamp.0 >= extension.timestamp.0)
    {
        return Err(invalid(
            "T1 screening cap extension timestamp is not globally later",
        ));
    }
    if stored.status != T1ScreenRunStatus::Paused
        || next.status != T1ScreenRunStatus::Paused
        || !matches!(stored.pause, Some(T1ScreenPauseReason::JudgeCap { .. }))
    {
        return Err(invalid(
            "T1 screening cap extension requires a paused judge-cap run",
        ));
    }
    let mut expected = stored.clone();
    expected.cap_extensions.clone_from(&next.cap_extensions);
    let is_unchanged_pause = next.pause == stored.pause;
    let (effective_owner, effective_provider) = t1_screen_effective_caps(next)?;
    let is_updated_pause = matches!(
        &next.pause,
        Some(T1ScreenPauseReason::JudgeCap {
            spent_millionths_of_dollar,
            owner_approved_millionths_of_dollar,
            provider_enforced_millionths_of_dollar,
        }) if *spent_millionths_of_dollar == stored.spent_judge_millionths_of_dollar
            && *owner_approved_millionths_of_dollar == effective_owner
            && *provider_enforced_millionths_of_dollar == effective_provider
    );
    if !is_unchanged_pause && !is_updated_pause {
        return Err(invalid(
            "T1 screening cap extension changed an invalid pause payload",
        ));
    }
    expected.pause.clone_from(&next.pause);
    if expected != *next {
        return Err(invalid(
            "T1 screening cap extension changed state outside the append",
        ));
    }
    Ok(())
}

pub(crate) fn validate_t1_timestamp(
    timestamp: &crate::model::Timestamp,
) -> Result<(), SkillEvalError> {
    let value = timestamp.0.as_bytes();
    let is_shape_valid = value.len() == 24
        && value[4] == b'-'
        && value[7] == b'-'
        && value[10] == b'T'
        && value[13] == b':'
        && value[16] == b':'
        && matches!(value[19], b'+' | b'-')
        && value.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        });
    if !is_shape_valid {
        return Err(invalid("T1 screening cap extension timestamp is invalid"));
    }
    let number = |start: usize, end: usize| -> u32 {
        std::str::from_utf8(&value[start..end])
            .expect("timestamp shape is ASCII")
            .parse()
            .expect("timestamp shape contains digits")
    };
    let year = number(0, 4);
    let month = number(5, 7);
    let day = number(8, 10);
    let hour = number(11, 13);
    let minute = number(14, 16);
    let second = number(17, 19);
    let offset_hour = number(20, 22);
    let offset_minute = number(22, 24);
    let is_leap_year =
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year => 29,
        2 => 28,
        _ => 0,
    };
    if year == 0
        || day == 0
        || day > days
        || hour > 23
        || minute > 59
        || second > 59
        || offset_hour > 23
        || offset_minute > 59
    {
        return Err(invalid("T1 screening cap extension timestamp is invalid"));
    }
    Ok(())
}

fn validate_classification_identity(
    configuration: &crate::model::T1ScreenRunConfiguration,
) -> Result<(), SkillEvalError> {
    let mut identities = BTreeSet::new();
    for row in &configuration.eligible {
        if !identities.insert((&row.provider, &row.model)) {
            return Err(invalid(
                "T1 screening classification contains duplicate identities",
            ));
        }
    }
    for row in &configuration.excluded {
        if !identities.insert((&row.provider, &row.model)) {
            return Err(invalid(
                "T1 screening classification contains duplicate identities",
            ));
        }
    }
    Ok(())
}

fn validate_eligible(eligible: &[T1ScreenEligibleRow]) -> Result<(), SkillEvalError> {
    if eligible.is_empty() {
        return Err(invalid(
            "T1 screening classification has no eligible models",
        ));
    }
    let mut previous = None::<(&str, &str)>;
    for row in eligible {
        validate_identity_parts(&row.provider, &row.model, "T1 eligible model")?;
        let key = (row.provider.as_str(), row.model.as_str());
        if previous.is_some_and(|prior| prior >= key) {
            return Err(invalid(
                "T1 eligible models are duplicate or outside provider/model order",
            ));
        }
        previous = Some(key);
        if row.supported_pi_thinking_levels.is_empty() {
            return Err(invalid("T1 eligible model has no thinking levels"));
        }
        let mut prior_level = None;
        for level in &row.supported_pi_thinking_levels {
            let index = THINKING_LEVELS
                .iter()
                .position(|known| level == known)
                .ok_or_else(|| invalid("T1 eligible model has an unknown thinking level"))?;
            if prior_level.is_some_and(|prior| prior >= index) {
                return Err(invalid(
                    "T1 eligible thinking levels are duplicate or outside canonical order",
                ));
            }
            prior_level = Some(index);
        }
    }
    Ok(())
}

fn validate_excluded(excluded: &[T1ScreenExcludedRow]) -> Result<(), SkillEvalError> {
    let mut previous = None::<(&str, &str)>;
    for row in excluded {
        validate_identity_parts(&row.provider, &row.model, "T1 excluded model")?;
        let key = (row.provider.as_str(), row.model.as_str());
        if previous.is_some_and(|prior| prior >= key) {
            return Err(invalid(
                "T1 excluded models are duplicate or outside provider/model order",
            ));
        }
        previous = Some(key);
        if row.reasons.is_empty() {
            return Err(invalid("T1 excluded model has no exclusion reason"));
        }
        let unique = row.reasons.iter().collect::<BTreeSet<_>>();
        if unique.len() != row.reasons.len() {
            return Err(invalid("T1 excluded model repeats an exclusion reason"));
        }
    }
    Ok(())
}

fn validate_exam(state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
    let exam = &state.configuration.exam;
    validate_canonical_absolute_path(&exam.root, "T1 fixed exam")?;
    if exam.revision.trim().is_empty()
        || exam.cases.len() != 5
        || exam.cases.iter().any(|case| case.is_holdout)
    {
        return Err(invalid(
            "T1 screening exam must be the fixed five-case loaded artifact",
        ));
    }
    let mut cases = BTreeSet::new();
    for case in &exam.cases {
        if !cases.insert(&case.id) {
            return Err(invalid("T1 screening exam contains duplicate cases"));
        }
    }
    Ok(())
}

fn validate_call_projection(state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
    let eligible = u64::try_from(state.configuration.eligible.len())
        .map_err(|_| invalid("T1 eligible count exceeds the supported range"))?;
    let routes = state
        .configuration
        .eligible
        .iter()
        .try_fold(0_u64, |sum, row| {
            let levels = u64::try_from(row.supported_pi_thinking_levels.len())
                .map_err(|_| invalid("T1 thinking count exceeds the supported range"))?;
            sum.checked_add(levels)
                .ok_or_else(|| invalid("T1 thinking count overflowed"))
        })?;
    let adaptive_minimum = eligible
        .checked_mul(EXAM_CASE_COUNT)
        .ok_or_else(|| invalid("T1 minimum call projection overflowed"))?;
    let complete = routes
        .checked_mul(EXAM_CASE_COUNT)
        .ok_or_else(|| invalid("T1 complete call projection overflowed"))?;
    let expected_minimum = if state.configuration.is_complete_thinking_coverage {
        complete
    } else {
        adaptive_minimum
    };
    let candidate = &state.configuration.candidate_calls;
    let judge = &state.configuration.judge_calls;
    if candidate.minimum != expected_minimum || candidate.maximum != complete || judge != candidate
    {
        return Err(invalid("T1 screening call projection differs"));
    }
    Ok(())
}

fn validate_children(state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
    let mut index = 0_usize;
    let mut run_ids = BTreeSet::new();
    for (model_index, eligible) in state.configuration.eligible.iter().enumerate() {
        for (thinking_index, thinking) in eligible.supported_pi_thinking_levels.iter().enumerate() {
            let child = state
                .child_runs
                .get(index)
                .ok_or_else(|| invalid("T1 screening child preallocation is incomplete"))?;
            let expected_model_index = u64::try_from(model_index)
                .map_err(|_| invalid("T1 model index exceeds the supported range"))?;
            let expected_thinking_index = u64::try_from(thinking_index)
                .map_err(|_| invalid("T1 thinking index exceeds the supported range"))?;
            if child.model_index != expected_model_index
                || child.thinking_index != expected_thinking_index
                || child.model.tier != Tier::T1
                || child.model.provider != eligible.provider
                || child.model.model != eligible.model
                || child.model.thinking != *thinking
            {
                return Err(invalid(
                    "T1 screening child identity or order differs from classification",
                ));
            }
            validate_identifier(&child.run_id.0, "T1 screening child")?;
            if !run_ids.insert(&child.run_id.0) {
                return Err(invalid(
                    "T1 screening child identifiers contain a duplicate or collision",
                ));
            }
            index = index
                .checked_add(1)
                .ok_or_else(|| invalid("T1 child index overflowed"))?;
        }
    }
    if index != state.child_runs.len() {
        return Err(invalid(
            "T1 screening child preallocation contains extra children",
        ));
    }
    Ok(())
}

fn validate_models(state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
    if state.models.len() != state.configuration.eligible.len() {
        return Err(invalid(
            "T1 screening model evidence does not cover every eligible model",
        ));
    }
    let mut is_unfinished_seen = false;
    for (model_index, (model, eligible)) in state
        .models
        .iter()
        .zip(&state.configuration.eligible)
        .enumerate()
    {
        if model.provider != eligible.provider || model.model != eligible.model {
            return Err(invalid(
                "T1 screening model evidence order differs from classification",
            ));
        }
        let model_index = u64::try_from(model_index)
            .map_err(|_| invalid("T1 model index exceeds the supported range"))?;
        let children = state
            .child_runs
            .iter()
            .filter(|child| child.model_index == model_index)
            .collect::<Vec<_>>();
        if is_unfinished_seen
            && (model.outcome.is_some()
                || !model.attempts.is_empty()
                || children
                    .iter()
                    .any(|child| child.status != T1ScreenChildStatus::Pending))
        {
            return Err(invalid("T1 screening model progression contains a gap"));
        }
        validate_attempts(state, model_index, model, eligible)?;
        if model.outcome.is_none() {
            is_unfinished_seen = true;
        }
        validate_model_child_progression(model, &children)?;
    }
    Ok(())
}

fn validate_model_child_progression(
    model: &T1ScreenModelState,
    children: &[&T1ScreenChildRun],
) -> Result<(), SkillEvalError> {
    match &model.outcome {
        None => {
            let attempt_count = model.attempts.len();
            if attempt_count == children.len() {
                return Err(invalid(
                    "T1 screening complete thinking evidence has no outcome",
                ));
            }
            for (index, child) in children.iter().enumerate() {
                if index < attempt_count && child.status != T1ScreenChildStatus::Completed
                    || index == attempt_count
                        && !matches!(
                            child.status,
                            T1ScreenChildStatus::Pending
                                | T1ScreenChildStatus::Running
                                | T1ScreenChildStatus::Paused
                        )
                    || index > attempt_count && child.status != T1ScreenChildStatus::Pending
                {
                    return Err(invalid("T1 screening thinking progression contains a gap"));
                }
            }
        }
        Some(T1ScreenModelOutcome::Selected { .. }) => {
            if model.attempts.len() != children.len()
                || children
                    .iter()
                    .any(|child| child.status != T1ScreenChildStatus::Completed)
            {
                return Err(invalid(
                    "T1 screening selected outcome precedes complete scored evidence",
                ));
            }
        }
        Some(T1ScreenModelOutcome::Exhausted) => {
            if model.attempts.len() != children.len()
                || children.iter().enumerate().any(|(index, child)| {
                    let expected = if index + 1 == children.len() {
                        T1ScreenChildStatus::Exhausted
                    } else {
                        T1ScreenChildStatus::Completed
                    };
                    child.status != expected
                })
            {
                return Err(invalid(
                    "T1 screening exhausted outcome precedes complete scored evidence",
                ));
            }
        }
        Some(T1ScreenModelOutcome::InfrastructureFailed {
            model: failed_model,
            child_run_id,
        }) => {
            let failed_index = children
                .iter()
                .position(|child| child.run_id == *child_run_id && child.model == *failed_model)
                .ok_or_else(|| invalid("T1 screening infrastructure failure has no child"))?;
            if children.iter().enumerate().any(|(index, child)| {
                let expected = match index.cmp(&failed_index) {
                    std::cmp::Ordering::Less => T1ScreenChildStatus::Completed,
                    std::cmp::Ordering::Equal => T1ScreenChildStatus::Failed,
                    std::cmp::Ordering::Greater => T1ScreenChildStatus::Skipped,
                };
                child.status != expected
            }) {
                return Err(invalid(
                    "T1 screening infrastructure failure progression differs",
                ));
            }
        }
    }
    Ok(())
}

fn validate_attempts(
    state: &T1ScreenRunState,
    model_index: u64,
    model: &T1ScreenModelState,
    eligible: &T1ScreenEligibleRow,
) -> Result<(), SkillEvalError> {
    if model.attempts.len() > eligible.supported_pi_thinking_levels.len() {
        return Err(invalid("T1 screening model has too many attempts"));
    }
    let children = state
        .child_runs
        .iter()
        .filter(|child| child.model_index == model_index)
        .collect::<Vec<_>>();
    for (attempt_index, attempt) in model.attempts.iter().enumerate() {
        let child = children
            .get(attempt_index)
            .ok_or_else(|| invalid("T1 screening attempt has no child"))?;
        validate_attempt(state, attempt, child)?;
    }
    match &model.outcome {
        None => {}
        Some(T1ScreenModelOutcome::Selected { model: selected }) => {
            let first_passing = model
                .attempts
                .iter()
                .find(|attempt| attempt.evidence.is_passing)
                .map(|attempt| &attempt.evidence.requested_model);
            if model.attempts.len() != eligible.supported_pi_thinking_levels.len()
                || first_passing != Some(selected)
            {
                return Err(invalid(
                    "T1 screening selected identity lacks complete first-passing evidence",
                ));
            }
        }
        Some(T1ScreenModelOutcome::Exhausted)
            if model.attempts.len() != eligible.supported_pi_thinking_levels.len()
                || model
                    .attempts
                    .iter()
                    .any(|attempt| attempt.evidence.is_passing) =>
        {
            return Err(invalid(
                "T1 screening exhausted marker lacks all failing attempts",
            ));
        }
        Some(T1ScreenModelOutcome::Exhausted) => {}
        Some(T1ScreenModelOutcome::InfrastructureFailed {
            model: failed_model,
            child_run_id,
        }) => {
            let failed_child = children
                .iter()
                .find(|child| child.run_id == *child_run_id && child.model == *failed_model)
                .ok_or_else(|| {
                    invalid("T1 screening infrastructure failure names no exact child")
                })?;
            if failed_child.model_index != model_index
                || model.attempts.len()
                    != usize::try_from(failed_child.thinking_index).map_err(|_| {
                        invalid("T1 screening infrastructure failure thinking index overflowed")
                    })?
            {
                return Err(invalid(
                    "T1 screening infrastructure failure has invalid prior attempts",
                ));
            }
        }
    }
    Ok(())
}

fn validate_attempt(
    state: &T1ScreenRunState,
    attempt: &T1ScreenAttemptEvidence,
    child: &T1ScreenChildRun,
) -> Result<(), SkillEvalError> {
    let evidence = &attempt.evidence;
    if attempt.child_run_id != child.run_id
        || evidence.stage != PoolStage::Calibration
        || evidence.requested_model != child.model
        || evidence.effective_model != child.model
        || evidence.judge_model != state.configuration.judge
        || evidence.harnesses != state.configuration.candidate_environment.harnesses
        || evidence.expected_trials != 5
        || evidence.completed_trials != evidence.expected_trials
        || evidence.is_passing
            && evidence.catastrophic_trials > state.configuration.policy.maximum_catastrophic_trials
        || !matches!(
            child.status,
            T1ScreenChildStatus::Completed | T1ScreenChildStatus::Exhausted
        )
        || (evidence.is_passing && child.status != T1ScreenChildStatus::Completed)
    {
        return Err(invalid(
            "T1 screening attempt evidence differs from its exact child identity",
        ));
    }
    let total = checked_add_usage(&evidence.candidate_usage, &evidence.judge_usage)?;
    if total != evidence.total_usage {
        return Err(invalid("T1 screening attempt total usage does not add up"));
    }
    Ok(())
}

fn validate_pause(state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
    if matches!(state.status, T1ScreenRunStatus::Paused) != state.pause.is_some() {
        return Err(invalid(
            "T1 screening pause reason does not match aggregate status",
        ));
    }
    match &state.pause {
        None => Ok(()),
        Some(T1ScreenPauseReason::Quota { model, .. }) => {
            if model != &state.configuration.judge
                && !state.child_runs.iter().any(|child| &child.model == model)
            {
                return Err(invalid("T1 screening quota pause names an unknown model"));
            }
            Ok(())
        }
        Some(T1ScreenPauseReason::Infrastructure { message }) => {
            if message.trim().is_empty() {
                Err(invalid("T1 screening infrastructure pause is empty"))
            } else {
                Ok(())
            }
        }
        Some(T1ScreenPauseReason::JudgeCap {
            spent_millionths_of_dollar,
            owner_approved_millionths_of_dollar,
            provider_enforced_millionths_of_dollar,
        }) => {
            let mut historical_caps = vec![(
                state
                    .configuration
                    .owner_approved_judge_cap_millionths_of_dollar,
                state
                    .configuration
                    .provider_enforced_judge_cap_millionths_of_dollar,
            )];
            historical_caps.extend(state.cap_extensions.iter().map(|extension| {
                (
                    extension.new_owner_cap_millionths_of_dollar,
                    extension.new_provider_cap_millionths_of_dollar,
                )
            }));
            let is_known_boundary = historical_caps.iter().any(|(owner, provider)| {
                owner_approved_millionths_of_dollar == owner
                    && provider_enforced_millionths_of_dollar == provider
                    && *spent_millionths_of_dollar <= (*owner).min(*provider)
            });
            if *spent_millionths_of_dollar != state.spent_judge_millionths_of_dollar
                || !is_known_boundary
            {
                return Err(invalid(
                    "T1 screening judge-cap pause differs from a known stop boundary",
                ));
            }
            Ok(())
        }
    }
}

fn validate_usage_and_spend(state: &T1ScreenRunState) -> Result<(), SkillEvalError> {
    if state.candidate_usage.cost_millionths_of_dollar != 0 {
        return Err(invalid("T1 screening candidate usage must cost zero"));
    }
    if state.spent_judge_millionths_of_dollar != state.judge_usage.cost_millionths_of_dollar {
        return Err(invalid(
            "T1 screening judge spend differs from aggregate judge usage",
        ));
    }
    let (effective_owner, effective_provider) = t1_screen_effective_caps(state)?;
    if state.spent_judge_millionths_of_dollar > effective_owner
        || state.spent_judge_millionths_of_dollar > effective_provider
    {
        return Err(invalid("T1 screening judge spend exceeds an effective cap"));
    }
    let mut candidate_evidence = zero_usage();
    let mut judge_evidence = zero_usage();
    for attempt in state.models.iter().flat_map(|model| &model.attempts) {
        candidate_evidence =
            checked_add_usage(&candidate_evidence, &attempt.evidence.candidate_usage)?;
        judge_evidence = checked_add_usage(&judge_evidence, &attempt.evidence.judge_usage)?;
    }
    if !usage_is_nondecreasing(&candidate_evidence, &state.candidate_usage)
        || !usage_is_nondecreasing(&judge_evidence, &state.judge_usage)
    {
        return Err(invalid(
            "T1 screening aggregate usage is below stored attempt evidence",
        ));
    }
    Ok(())
}

fn checked_add_usage(left: &TrialUsage, right: &TrialUsage) -> Result<TrialUsage, SkillEvalError> {
    Ok(TrialUsage {
        input_tokens: checked_add(left.input_tokens, right.input_tokens, "input tokens")?,
        output_tokens: checked_add(left.output_tokens, right.output_tokens, "output tokens")?,
        cache_read_tokens: checked_add(
            left.cache_read_tokens,
            right.cache_read_tokens,
            "cache-read tokens",
        )?,
        cache_write_tokens: checked_add(
            left.cache_write_tokens,
            right.cache_write_tokens,
            "cache-write tokens",
        )?,
        turns: left
            .turns
            .checked_add(right.turns)
            .ok_or_else(|| invalid("T1 screening turns overflowed"))?,
        tool_calls: left
            .tool_calls
            .checked_add(right.tool_calls)
            .ok_or_else(|| invalid("T1 screening tool calls overflowed"))?,
        elapsed_milliseconds: checked_add(
            left.elapsed_milliseconds,
            right.elapsed_milliseconds,
            "elapsed milliseconds",
        )?,
        cost_millionths_of_dollar: checked_add(
            left.cost_millionths_of_dollar,
            right.cost_millionths_of_dollar,
            "cost",
        )?,
    })
}

fn checked_add(left: u64, right: u64, field: &str) -> Result<u64, SkillEvalError> {
    left.checked_add(right)
        .ok_or_else(|| invalid(format!("T1 screening {field} overflowed")))
}

fn usage_is_nondecreasing(old: &TrialUsage, new: &TrialUsage) -> bool {
    new.input_tokens >= old.input_tokens
        && new.output_tokens >= old.output_tokens
        && new.cache_read_tokens >= old.cache_read_tokens
        && new.cache_write_tokens >= old.cache_write_tokens
        && new.turns >= old.turns
        && new.tool_calls >= old.tool_calls
        && new.elapsed_milliseconds >= old.elapsed_milliseconds
        && new.cost_millionths_of_dollar >= old.cost_millionths_of_dollar
}

fn is_legal_child_transition(old: T1ScreenChildStatus, next: T1ScreenChildStatus) -> bool {
    matches!(
        (old, next),
        (T1ScreenChildStatus::Pending, T1ScreenChildStatus::Running)
            | (T1ScreenChildStatus::Pending, T1ScreenChildStatus::Skipped)
            | (T1ScreenChildStatus::Running, T1ScreenChildStatus::Paused)
            | (T1ScreenChildStatus::Running, T1ScreenChildStatus::Completed)
            | (T1ScreenChildStatus::Running, T1ScreenChildStatus::Exhausted)
            | (T1ScreenChildStatus::Running, T1ScreenChildStatus::Failed)
            | (T1ScreenChildStatus::Paused, T1ScreenChildStatus::Running)
    )
}

fn is_legal_run_transition(old: T1ScreenRunStatus, next: T1ScreenRunStatus) -> bool {
    matches!(
        (old, next),
        (T1ScreenRunStatus::Pending, T1ScreenRunStatus::Running)
            | (T1ScreenRunStatus::Running, T1ScreenRunStatus::Paused)
            | (T1ScreenRunStatus::Running, T1ScreenRunStatus::AwaitingOwner)
            | (T1ScreenRunStatus::Running, T1ScreenRunStatus::Failed)
            | (T1ScreenRunStatus::Paused, T1ScreenRunStatus::Running)
            | (T1ScreenRunStatus::Paused, T1ScreenRunStatus::Failed)
            | (
                T1ScreenRunStatus::AwaitingOwner,
                T1ScreenRunStatus::Completed
            )
            | (T1ScreenRunStatus::AwaitingOwner, T1ScreenRunStatus::Failed)
    )
}

fn child_identity(child: &T1ScreenChildRun) -> (&ModelIdentity, &RunId, u64, u64) {
    (
        &child.model,
        &child.run_id,
        child.model_index,
        child.thinking_index,
    )
}

fn validate_model_identity(model: &ModelIdentity, label: &str) -> Result<(), SkillEvalError> {
    validate_identity_parts(&model.provider, &model.model, label)?;
    if !THINKING_LEVELS.contains(&model.thinking.as_str()) {
        return Err(invalid(format!("{label} has an unknown thinking level")));
    }
    Ok(())
}

fn validate_identity_parts(provider: &str, model: &str, label: &str) -> Result<(), SkillEvalError> {
    if provider.contains('/')
        || !is_identity_segment(provider)
        || !model.split('/').all(is_identity_segment)
    {
        return Err(invalid(format!("{label} identity is malformed")));
    }
    Ok(())
}

fn is_identity_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '-' | '.' | '_' | ':' | '+' | '@')
        })
}

pub(crate) fn validate_identifier(identifier: &str, kind: &str) -> Result<(), SkillEvalError> {
    if identifier.is_empty()
        || !identifier
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(invalid(format!(
            "{kind} identifier {identifier:?} is not a safe path component"
        )));
    }
    Ok(())
}

pub(crate) fn validate_digest(digest: &str, label: &str) -> Result<(), SkillEvalError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(format!("{label} digest is not lowercase SHA-256")));
    }
    Ok(())
}

fn validate_canonical_absolute_path(path: &Path, label: &str) -> Result<(), SkillEvalError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(invalid(format!("{label} path is not canonical")));
    }
    Ok(())
}

fn zero_usage() -> TrialUsage {
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

fn canonical_repository_root(repository_root: &Path) -> Result<PathBuf, SkillEvalError> {
    let canonical =
        fs::canonicalize(repository_root).map_err(|error| io_error(repository_root, error))?;
    if !canonical.is_dir() {
        return Err(invalid(format!(
            "repository root {} is not a directory",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn existing_contained_directory(
    repository_root: &Path,
    components: &[&str],
) -> Result<PathBuf, SkillEvalError> {
    let mut current = repository_root.to_path_buf();
    for component in components {
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|error| io_error(&current, error))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid(format!(
                "T1 screening root component {} is not a real directory",
                current.display()
            )));
        }
    }
    let canonical = fs::canonicalize(&current).map_err(|error| io_error(&current, error))?;
    if canonical != current || !canonical.starts_with(repository_root) {
        return Err(invalid(
            "T1 screening root escapes the configured repository root",
        ));
    }
    Ok(canonical)
}

fn create_contained_directory(
    repository_root: &Path,
    components: &[&str],
) -> Result<PathBuf, SkillEvalError> {
    let mut current = repository_root.to_path_buf();
    for component in components {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(invalid(format!(
                    "T1 screening root component {} is not a real directory",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| io_error(&current, error))?;
            }
            Err(error) => return Err(io_error(&current, error)),
        }
    }
    let canonical = fs::canonicalize(&current).map_err(|error| io_error(&current, error))?;
    if canonical != current || !canonical.starts_with(repository_root) {
        return Err(invalid(
            "T1 screening root escapes the configured repository root",
        ));
    }
    Ok(canonical)
}

fn restore_snapshot(directory: &Path, snapshot: &Path, bytes: &[u8]) -> Result<(), SkillEvalError> {
    let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(
        ".{SNAPSHOT_NAME}.rollback.{}.{sequence}.tmp",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| io_error(&temporary, error))?;
    file.write_all(bytes)
        .map_err(|error| io_error(&temporary, error))?;
    file.sync_all()
        .map_err(|error| io_error(&temporary, error))?;
    fs::rename(&temporary, snapshot).map_err(|error| io_error(snapshot, error))?;
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_error(directory, error))
}

fn invalid(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(message.into())
}

fn io_error(path: &Path, error: std::io::Error) -> SkillEvalError {
    SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}
