use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::model::{
    PoolChildRun, PoolChildStatus, PoolRunId, PoolRunState, PoolRunStatus, PoolStage, RankedPool,
    SkillEvalError, Tier,
};
use crate::ports::PoolStore;

const POOLS_DIRECTORY: &str = "pools";
const SNAPSHOT_NAME: &str = "state.json";
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Persists validated model-pool snapshots below one runs root.
///
/// The input is a runs root. The output is a store restricted to its `pools` directory.
///
/// # Errors
///
/// Store construction and operations fail for invalid state, unsafe paths, malformed data, or
/// file input/output errors.
pub(crate) struct FilePoolStore {
    pools_root: PathBuf,
    #[cfg(test)]
    failure: Option<FailurePoint>,
}

impl FilePoolStore {
    /// Creates a pool store below `runs_root`.
    ///
    /// The input is a runs-root path. The output is a canonical, path-restricted store.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be created, resolved, or opened as a directory.
    pub(crate) fn new(runs_root: impl AsRef<Path>) -> Result<Self, SkillEvalError> {
        let runs_root = runs_root.as_ref();
        fs::create_dir_all(runs_root).map_err(|error| io_error(runs_root, error))?;
        let runs_root = fs::canonicalize(runs_root).map_err(|error| io_error(runs_root, error))?;
        if !runs_root.is_dir() {
            return Err(invalid(format!(
                "runs root {} is not a directory",
                runs_root.display()
            )));
        }
        let pools_root = runs_root.join(POOLS_DIRECTORY);
        fs::create_dir_all(&pools_root).map_err(|error| io_error(&pools_root, error))?;
        let pools_root =
            fs::canonicalize(&pools_root).map_err(|error| io_error(&pools_root, error))?;
        if !pools_root.is_dir() || !pools_root.starts_with(&runs_root) {
            return Err(invalid(format!(
                "pool root {} escapes the configured runs root",
                pools_root.display()
            )));
        }
        Ok(Self {
            pools_root,
            #[cfg(test)]
            failure: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_failure(
        runs_root: impl AsRef<Path>,
        failure: FailurePoint,
    ) -> Result<Self, SkillEvalError> {
        let mut store = Self::new(runs_root)?;
        store.failure = Some(failure);
        Ok(store)
    }

    fn run_directory(&self, run_id: &PoolRunId) -> Result<PathBuf, SkillEvalError> {
        validate_identifier(&run_id.0, "pool run")?;
        Ok(self.pools_root.join(&run_id.0))
    }

    fn existing_run_directory(&self, run_id: &PoolRunId) -> Result<PathBuf, SkillEvalError> {
        let expected = self.run_directory(run_id)?;
        let canonical = fs::canonicalize(&expected).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SkillEvalError::NotFound(format!("pool run {:?} does not exist", run_id.0))
            } else {
                io_error(&expected, error)
            }
        })?;
        if canonical != expected || !canonical.is_dir() {
            return Err(invalid(format!(
                "pool run {:?} escapes the configured pool root",
                run_id.0
            )));
        }
        Ok(canonical)
    }

    fn existing_snapshot_path(&self, run_id: &PoolRunId) -> Result<PathBuf, SkillEvalError> {
        let path = self.existing_run_directory(run_id)?.join(SNAPSHOT_NAME);
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SkillEvalError::NotFound(format!("pool run {:?} has no snapshot", run_id.0))
            } else {
                io_error(&path, error)
            }
        })?;
        if !metadata.file_type().is_file() {
            return Err(invalid(format!(
                "pool snapshot {} is not a regular file",
                path.display()
            )));
        }
        Ok(path)
    }

    fn read_snapshot(&self, run_id: &PoolRunId) -> Result<(PoolRunState, Vec<u8>), SkillEvalError> {
        let path = self.existing_snapshot_path(run_id)?;
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            invalid(format!(
                "pool snapshot {} is malformed: {error}",
                path.display()
            ))
        })?;
        let state: PoolRunState = serde_json::from_value(value.clone()).map_err(|error| {
            invalid(format!(
                "pool snapshot {} is malformed: {error}",
                path.display()
            ))
        })?;
        let normalized = serde_json::to_value(&state).map_err(|error| {
            invalid(format!(
                "pool snapshot {} cannot be validated: {error}",
                path.display()
            ))
        })?;
        if normalized != value {
            return Err(invalid(format!(
                "pool snapshot {} contains unknown data",
                path.display()
            )));
        }
        if state.configuration.run_id != *run_id {
            return Err(invalid("pool snapshot identity differs from its path"));
        }
        validate_state(&state, false)?;
        Ok((state, bytes))
    }

    fn write_temporary(
        &mut self,
        directory: &Path,
        state: &PoolRunState,
    ) -> Result<PathBuf, SkillEvalError> {
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|error| invalid(format!("pool snapshot serialization failed: {error}")))?;
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
            self.fail(FailurePoint::Write, &path)?;
            file.write_all(&bytes)
                .and_then(|()| file.write_all(b"\n"))
                .map_err(|error| io_error(&path, error))?;
            self.fail(FailurePoint::FileSync, &path)?;
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
        state: &PoolRunState,
        prior_bytes: Option<&[u8]>,
    ) -> Result<(), SkillEvalError> {
        let temporary = self.write_temporary(directory, state)?;
        let mut is_replaced = false;
        let result = (|| {
            self.fail(FailurePoint::Rename, snapshot)?;
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
        self.fail(FailurePoint::DirectorySync, directory)?;
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|error| io_error(directory, error))
    }

    #[cfg(test)]
    fn fail(&mut self, point: FailurePoint, path: &Path) -> Result<(), SkillEvalError> {
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
    fn fail(&mut self, _point: FailurePoint, _path: &Path) -> Result<(), SkillEvalError> {
        Ok(())
    }
}

impl PoolStore for FilePoolStore {
    fn create_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
        validate_state(state, true)?;
        let directory = self.run_directory(&state.configuration.run_id)?;
        fs::create_dir(&directory).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                invalid(format!(
                    "pool run {:?} already exists",
                    state.configuration.run_id.0
                ))
            } else {
                io_error(&directory, error)
            }
        })?;
        let snapshot = directory.join(SNAPSHOT_NAME);
        let pools_root = self.pools_root.clone();
        let result = self
            .replace_snapshot(&directory, &snapshot, state, None)
            .and_then(|()| self.sync_directory(&pools_root));
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&directory);
            let _ = File::open(&self.pools_root).and_then(|file| file.sync_all());
            return Err(error);
        }
        Ok(())
    }

    fn load_pool(&self, run_id: &PoolRunId) -> Result<PoolRunState, SkillEvalError> {
        self.read_snapshot(run_id).map(|(state, _)| state)
    }

    fn save_pool(&mut self, state: &PoolRunState) -> Result<(), SkillEvalError> {
        validate_state(state, false)?;
        let (stored, prior_bytes) = self.read_snapshot(&state.configuration.run_id)?;
        validate_transition(&stored, state)?;
        let directory = self.existing_run_directory(&state.configuration.run_id)?;
        let snapshot = directory.join(SNAPSHOT_NAME);
        self.replace_snapshot(&directory, &snapshot, state, Some(&prior_bytes))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailurePoint {
    Write,
    FileSync,
    Rename,
    DirectorySync,
}

// TODO(AGNT-0032.T88): Reject empty, duplicate, or incomplete frozen exam definitions.
fn validate_state(state: &PoolRunState, is_initial: bool) -> Result<(), SkillEvalError> {
    validate_identifier(&state.configuration.run_id.0, "pool run")?;
    if state.selected_tiers.is_empty() {
        return Err(invalid("pool state must select at least one tier"));
    }
    let selected = state
        .selected_tiers
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if selected.len() != state.selected_tiers.len() {
        return Err(invalid("pool state contains duplicate selected tiers"));
    }
    if selected
        .iter()
        .any(|tier| !state.configuration.entrants.contains_key(tier))
    {
        return Err(invalid(
            "pool state selects a tier absent from its configuration",
        ));
    }

    let mut slots = BTreeSet::new();
    let mut run_ids = BTreeSet::new();
    for child in &state.child_runs {
        validate_identifier(&child.run_id.0, "child run")?;
        if !selected.contains(&child.tier) {
            return Err(invalid(
                "pool state contains a child for an unselected tier",
            ));
        }
        let entrant_count = state.configuration.entrants[&child.tier].len();
        if usize::from(child.entrant_index) >= entrant_count {
            return Err(invalid("pool state contains an out-of-range child entrant"));
        }
        if !slots.insert((child.tier, child.entrant_index, stage_number(child.stage))) {
            return Err(invalid("pool state contains duplicate child slots"));
        }
        if !run_ids.insert(child.run_id.0.as_str()) {
            return Err(invalid(
                "pool state contains duplicate child run identifiers",
            ));
        }
    }
    for tier in &state.selected_tiers {
        for entrant_index in 0..state.configuration.entrants[tier].len() {
            let entrant_index = u8::try_from(entrant_index)
                .map_err(|_| invalid("pool state has too many entrants for child slots"))?;
            for stage in [PoolStage::Calibration, PoolStage::Qualification] {
                if !slots.contains(&(*tier, entrant_index, stage_number(stage))) {
                    return Err(invalid("pool state has an unallocated child run"));
                }
            }
        }
    }

    validate_pools(state)?;
    if matches!(state.status, PoolRunStatus::Paused) != state.pause.is_some() {
        return Err(invalid("pool pause reason does not match aggregate status"));
    }
    if is_initial
        && (state.status != PoolRunStatus::Pending
            || state
                .child_runs
                .iter()
                .any(|child| child.status != PoolChildStatus::Pending)
            || !state.pools.is_empty()
            || state.pause.is_some()
            || state.spent_millionths_of_dollar != 0)
    {
        return Err(invalid(
            "new pool state must be entirely pending and unspent",
        ));
    }
    Ok(())
}

fn validate_pools(state: &PoolRunState) -> Result<(), SkillEvalError> {
    let selected = state
        .selected_tiers
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut tiers = BTreeSet::new();
    for pool in &state.pools {
        if !selected.contains(&pool.tier) || !tiers.insert(pool.tier) {
            return Err(invalid(
                "pool state contains a duplicate or unselected ranked pool",
            ));
        }
        let configured = state.configuration.entrants[&pool.tier]
            .iter()
            .map(|entrant| &entrant.model)
            .collect::<Vec<_>>();
        if pool.calibration.iter().any(|evidence| {
            evidence.stage != PoolStage::Calibration
                || evidence.requested_model != evidence.effective_model
                || !configured.contains(&&evidence.requested_model)
        }) || pool.qualification.iter().any(|evidence| {
            evidence.stage != PoolStage::Qualification
                || evidence.requested_model != evidence.effective_model
                || !pool.promoted.contains(&evidence.requested_model)
        }) || pool
            .promoted
            .iter()
            .chain(&pool.ranked)
            .any(|model| !configured.contains(&model))
        {
            return Err(invalid(
                "ranked pool contains drifting or unconfigured model identity",
            ));
        }
        if has_duplicate_models(&pool.promoted)
            || has_duplicate_models(&pool.ranked)
            || pool
                .ranked
                .iter()
                .any(|model| !pool.promoted.contains(model))
        {
            return Err(invalid(
                "ranked pool contains duplicate or unpromoted identity",
            ));
        }
        let promotion_count = usize::from(state.configuration.policy.promotion_count);
        if pool.promoted.len() > promotion_count
            || pool.ranked.len() > promotion_count
            || (pool.is_complete && pool.ranked.len() != promotion_count)
        {
            return Err(invalid(
                "ranked pool has an invalid promotion or completion count",
            ));
        }
    }
    Ok(())
}

fn has_duplicate_models(models: &[crate::model::ModelIdentity]) -> bool {
    models
        .iter()
        .enumerate()
        .any(|(index, model)| models[..index].contains(model))
}

fn validate_transition(stored: &PoolRunState, next: &PoolRunState) -> Result<(), SkillEvalError> {
    if stored.configuration != next.configuration {
        return Err(invalid("pool configuration changed after creation"));
    }
    if stored.selected_tiers != next.selected_tiers {
        return Err(invalid("pool selected tiers changed after creation"));
    }
    if stored.child_runs.len() != next.child_runs.len() {
        return Err(invalid("pool child identities changed after creation"));
    }

    let mut changed_statuses = 0_usize;
    for (old, new) in stored.child_runs.iter().zip(&next.child_runs) {
        if child_identity(old) != child_identity(new) {
            return Err(invalid("pool child identities changed after creation"));
        }
        if old.status != new.status {
            if !is_legal_child_transition(old.status, new.status) {
                return Err(invalid(
                    "pool child status moved backward or skipped a state",
                ));
            }
            changed_statuses += 1;
        }
    }
    if stored.status != next.status {
        if !is_legal_pool_transition(stored.status, next.status) {
            return Err(invalid("pool aggregate status transition is illegal"));
        }
        changed_statuses += 1;
    }
    if changed_statuses > 1 {
        return Err(invalid("pool snapshot changes more than one status"));
    }
    if stored.status == next.status && stored.pause != next.pause {
        return Err(invalid(
            "pool pause reason changed without a status transition",
        ));
    }
    if next.spent_millionths_of_dollar < stored.spent_millionths_of_dollar {
        return Err(invalid("pool spending cannot decrease"));
    }
    validate_pool_progress(&stored.pools, &next.pools)?;
    Ok(())
}

fn validate_pool_progress(
    stored: &[RankedPool],
    next: &[RankedPool],
) -> Result<(), SkillEvalError> {
    if next.len() < stored.len() {
        return Err(invalid("ranked pool evidence cannot be removed"));
    }
    for (old, new) in stored.iter().zip(next) {
        if old.tier != new.tier
            || !new.calibration.starts_with(&old.calibration)
            || !new.qualification.starts_with(&old.qualification)
            || (!old.promoted.is_empty() && new.promoted != old.promoted)
            || (!old.ranked.is_empty() && new.ranked != old.ranked)
            || (old.is_complete && !new.is_complete)
        {
            return Err(invalid(
                "ranked pool evidence moved backward or changed identity",
            ));
        }
    }
    Ok(())
}

fn child_identity(child: &PoolChildRun) -> (Tier, u8, u8, &str) {
    (
        child.tier,
        child.entrant_index,
        stage_number(child.stage),
        &child.run_id.0,
    )
}

fn stage_number(stage: PoolStage) -> u8 {
    match stage {
        PoolStage::Calibration => 0,
        PoolStage::Qualification => 1,
    }
}

fn is_legal_child_transition(old: PoolChildStatus, next: PoolChildStatus) -> bool {
    matches!(
        (old, next),
        (PoolChildStatus::Pending, PoolChildStatus::Running)
            | (PoolChildStatus::Running, PoolChildStatus::Paused)
            | (PoolChildStatus::Running, PoolChildStatus::Completed)
            | (PoolChildStatus::Running, PoolChildStatus::Failed)
            | (PoolChildStatus::Paused, PoolChildStatus::Running)
    )
}

fn is_legal_pool_transition(old: PoolRunStatus, next: PoolRunStatus) -> bool {
    matches!(
        (old, next),
        (PoolRunStatus::Pending, PoolRunStatus::Running)
            | (PoolRunStatus::Running, PoolRunStatus::Paused)
            | (PoolRunStatus::Running, PoolRunStatus::AwaitingDecision)
            | (PoolRunStatus::Running, PoolRunStatus::Failed)
            | (PoolRunStatus::Paused, PoolRunStatus::Running)
            | (PoolRunStatus::Paused, PoolRunStatus::Failed)
            | (PoolRunStatus::AwaitingDecision, PoolRunStatus::Completed)
            | (PoolRunStatus::AwaitingDecision, PoolRunStatus::Failed)
    )
}

fn validate_identifier(identifier: &str, kind: &str) -> Result<(), SkillEvalError> {
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
