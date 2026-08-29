use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{
    Decision, FrontierBaseline, FrontierBaselineLedger, FrontierCaseGroup, FrontierCaseKey,
    FrontierCaseReviewDecision, FrontierCellStatus, FrontierEvidenceIdentity, FrontierInspection,
    FrontierPlan, FrontierRunId, FrontierRunState, FrontierRunStatus, FrontierSuite,
    FrontierSuiteConstructionPlan, FrontierSuiteConstructionPolicy, FrontierSuiteInventory,
    FrontierSuiteProposal, FrontierSuiteProposalStatus, FrontierSuitePublication,
    FrontierSuiteReviewSet, FrontierTrialSelector, SkillEvalError, Tier, Timestamp, TrialRecord,
    TrialUsage,
};

#[cfg(test)]
#[path = "frontier_source.rs"]
mod test_frontier_source;
#[cfg(not(test))]
use crate::frontier_source;
#[cfg(test)]
use test_frontier_source as frontier_source;

const FRONTIER_ROOT: [&str; 3] = [".map", "skill-eval", "frontier"];
const RUN_EVIDENCE_ROOT: [&str; 3] = [".map", "skill-eval", "runs"];
const SNAPSHOT_NAME: &str = "state.json";
const RUN_LOCK_NAME: &str = ".writer.lock";
const TRIALS_DIRECTORY: &str = "trials";
const TRANSACTION_NAME: &str = ".baseline-transaction.json";
const LEDGER_VERSION: u64 = 1;
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct FileFrontierStore {
    repository_root: PathBuf,
    frontier_root: PathBuf,
    #[cfg(test)]
    failure: Option<FrontierFailurePoint>,
}

impl FileFrontierStore {
    pub(crate) fn new(repository_root: impl AsRef<Path>) -> Result<Self, SkillEvalError> {
        let repository_root = canonical_repository_root(repository_root.as_ref())?;
        let frontier_root = create_contained_directory(&repository_root, &FRONTIER_ROOT)?;
        let mut store = Self {
            repository_root,
            frontier_root,
            #[cfg(test)]
            failure: None,
        };
        store.recover_transaction()?;
        Ok(store)
    }

    #[cfg(test)]
    pub(crate) fn with_failure(
        repository_root: impl AsRef<Path>,
        failure: FrontierFailurePoint,
    ) -> Result<Self, SkillEvalError> {
        let mut store = Self::new(repository_root)?;
        store.failure = Some(failure);
        Ok(store)
    }

    pub(crate) fn create_frontier(
        &mut self,
        state: &FrontierRunState,
    ) -> Result<(), SkillEvalError> {
        self.recover_transaction()?;
        validate_state(&self.repository_root, state, true)?;
        let directory = self.run_directory(&state.configuration.run_id)?;
        fs::create_dir(&directory).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                invalid("frontier run already exists")
            } else {
                io_error(&directory, error)
            }
        })?;
        let trials = directory.join(TRIALS_DIRECTORY);
        let result = (|| {
            fs::create_dir(&trials).map_err(|error| io_error(&trials, error))?;
            replace_bytes(
                &directory.join(SNAPSHOT_NAME),
                &json_bytes(state, "frontier snapshot")?,
            )?;
            sync_directory(&directory)?;
            sync_directory(&self.frontier_root)
        })();
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&directory);
            let _ = sync_directory(&self.frontier_root);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn lock_frontier_run(&self, run_id: &FrontierRunId) -> Result<File, SkillEvalError> {
        let path = self.existing_run_directory(run_id)?.join(RUN_LOCK_NAME);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(invalid("frontier run lock path is unsafe"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(&path, error)),
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| io_error(&path, error))?;
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => {
                invalid("another frontier writer owns the run lock")
            }
            std::fs::TryLockError::Error(error) => io_error(&path, error),
        })?;
        Ok(file)
    }

    pub(crate) fn load_frontier(
        &self,
        run_id: &FrontierRunId,
    ) -> Result<FrontierRunState, SkillEvalError> {
        if self.transaction_path().exists() {
            return Err(invalid("frontier baseline transaction requires recovery"));
        }
        self.read_state(run_id).map(|(state, _)| state)
    }

    pub(crate) fn save_frontier(&mut self, state: &FrontierRunState) -> Result<(), SkillEvalError> {
        self.recover_transaction()?;
        validate_state(&self.repository_root, state, false)?;
        if state.status == FrontierRunStatus::Accepted {
            return Err(invalid(
                "accepted frontier state requires the baseline transaction",
            ));
        }
        let (stored, stored_bytes) = self.read_state(&state.configuration.run_id)?;
        if stored == *state {
            return Ok(());
        }
        validate_transition(&stored, state, false)?;
        let path = self.state_path(&state.configuration.run_id)?;
        replace_bytes_recoverable(
            &path,
            &json_bytes(state, "frontier snapshot")?,
            &stored_bytes,
        )
    }

    pub(crate) fn save_frontier_trial(
        &mut self,
        run_id: &FrontierRunId,
        trial: &TrialRecord,
    ) -> Result<(), SkillEvalError> {
        self.recover_transaction()?;
        let (state, _) = self.read_state(run_id)?;
        validate_trial(&self.repository_root, &state, trial)?;
        let directory = self.existing_run_directory(run_id)?.join(TRIALS_DIRECTORY);
        let path = directory.join(format!("{}.json", trial_identity_digest(trial)?));
        let bytes = json_bytes(trial, "frontier trial")?;
        match fs::read(&path) {
            Ok(stored) if stored == bytes => return Ok(()),
            Ok(_) => return Err(invalid("frontier trial identity has conflicting evidence")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(&path, error)),
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    invalid("frontier trial was written concurrently")
                } else {
                    io_error(&path, error)
                }
            })?;
        if let Err(error) = file
            .write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| io_error(&path, error))
            .and_then(|()| sync_directory(&directory))
        {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn inspect_frontier(
        &self,
        selector: &FrontierTrialSelector,
    ) -> Result<FrontierInspection, SkillEvalError> {
        validate_selector(selector)?;
        let (state, _) = self.read_state(&selector.run_id)?;
        let mut matched_trial = None;
        let trials = self
            .existing_run_directory(&selector.run_id)?
            .join(TRIALS_DIRECTORY);
        for entry in fs::read_dir(&trials).map_err(|error| io_error(&trials, error))? {
            let entry = entry.map_err(|error| io_error(&trials, error))?;
            if !entry
                .file_type()
                .map_err(|error| io_error(&entry.path(), error))?
                .is_file()
            {
                return Err(invalid("frontier trial storage contains a non-file entry"));
            }
            let trial: TrialRecord = read_strict_json(&entry.path(), "frontier trial")?;
            validate_trial(&self.repository_root, &state, &trial)?;
            if trial_matches(&trial, selector) {
                if matched_trial.is_some() {
                    return Err(invalid("frontier inspection selector is ambiguous"));
                }
                matched_trial = Some(trial);
            }
        }
        if let Some(trial) = matched_trial {
            return Ok(FrontierInspection::Trial { trial });
        }
        state
            .infrastructure_events
            .iter()
            .filter(|event| {
                event.model.provider == selector.provider
                    && event.model.model == selector.model
                    && event.model.tier == selector.tier
                    && event.model.thinking == selector.thinking
                    && event.artifact == selector.artifact
                    && event.case == selector.case
                    && event.attempt == selector.attempt
            })
            .max_by_key(|event| event.infrastructure_attempt)
            .cloned()
            .map(|event| FrontierInspection::Infrastructure { event })
            .ok_or_else(|| {
                SkillEvalError::NotFound(
                    "frontier inspection selector has no exact record".to_owned(),
                )
            })
    }

    pub(crate) fn load_frontier_suite_construction_plan(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteConstructionPlan, SkillEvalError> {
        let plan = self.load_suite_evidence(path, "frontier suite construction plan")?;
        validate_suite_plan(&plan)?;
        Ok(plan)
    }

    pub(crate) fn load_frontier_suite_inventory(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteInventory, SkillEvalError> {
        let inventory = self.load_suite_evidence(path, "frontier suite inventory")?;
        validate_suite_inventory(&inventory)?;
        Ok(inventory)
    }

    pub(crate) fn load_frontier_suite_review_set(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteReviewSet, SkillEvalError> {
        let reviews = self.load_suite_evidence(path, "frontier suite review set")?;
        validate_suite_reviews(&reviews)?;
        Ok(reviews)
    }

    pub(crate) fn load_frontier_suite_proposal(
        &self,
        path: &Path,
    ) -> Result<FrontierSuiteProposal, SkillEvalError> {
        let proposal = self.load_suite_evidence(path, "frontier suite proposal")?;
        validate_suite_proposal(&proposal)?;
        Ok(proposal)
    }

    pub(crate) fn save_frontier_suite_inventory(
        &mut self,
        path: &Path,
        inventory: &FrontierSuiteInventory,
    ) -> Result<(), SkillEvalError> {
        validate_suite_inventory(inventory)?;
        let bytes = json_bytes(inventory, "frontier suite inventory")?;
        self.save_immutable_suite_evidence(
            path,
            &bytes,
            "frontier suite inventory",
            FrontierFailurePoint::Inventory,
            FrontierFailurePoint::InventoryAfterLink,
        )
    }

    pub(crate) fn save_frontier_suite_proposal(
        &mut self,
        path: &Path,
        proposal: &FrontierSuiteProposal,
    ) -> Result<(), SkillEvalError> {
        validate_suite_proposal(proposal)?;
        let bytes = json_bytes(proposal, "frontier suite proposal")?;
        self.save_immutable_suite_evidence(
            path,
            &bytes,
            "frontier suite proposal",
            FrontierFailurePoint::Proposal,
            FrontierFailurePoint::ProposalAfterLink,
        )
    }

    pub(crate) fn apply_frontier_suite_proposal(
        &mut self,
        proposal: &FrontierSuiteProposal,
        output: &Path,
        published_at: &Timestamp,
    ) -> Result<FrontierSuitePublication, SkillEvalError> {
        if published_at.0.trim().is_empty() {
            return Err(invalid("frontier suite publication time is empty"));
        }
        let suite = frontier_source::frontier_suite_from_ready_proposal(proposal)?;
        let proposal_bytes = json_bytes(proposal, "frontier suite proposal")?;
        let suite_bytes = json_bytes(&suite, "frontier suite")?;
        let destination = safe_suite_destination(&self.repository_root, output)?;
        self.replace_suite_bytes(&destination, &suite_bytes)?;
        Ok(FrontierSuitePublication {
            proposal_sha256: hex_digest(&proposal_bytes),
            suite_path: output.to_path_buf(),
            suite_sha256: hex_digest(&suite_bytes),
            published_at: published_at.clone(),
        })
    }

    pub(crate) fn load_frontier_baselines(
        &self,
        path: &Path,
    ) -> Result<FrontierBaselineLedger, SkillEvalError> {
        if self.transaction_path().exists() {
            return Err(invalid("frontier baseline transaction requires recovery"));
        }
        let candidate = safe_repository_path(&self.repository_root, path, false)?;
        let ledger = match fs::symlink_metadata(&candidate) {
            Ok(_) => {
                let path = safe_repository_path(&self.repository_root, path, true)?;
                let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
                strict_json_bytes(&bytes, &path, "frontier baseline ledger")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => FrontierBaselineLedger {
                version: LEDGER_VERSION,
                baselines: Vec::new(),
            },
            Err(error) => return Err(io_error(&candidate, error)),
        };
        validate_ledger(&self.repository_root, &ledger)?;
        Ok(ledger)
    }

    pub(crate) fn accept_frontier_baseline(
        &mut self,
        state: &FrontierRunState,
        path: &Path,
        ledger: &FrontierBaselineLedger,
    ) -> Result<(), SkillEvalError> {
        self.recover_transaction()?;
        validate_state(&self.repository_root, state, false)?;
        if state.status != FrontierRunStatus::Accepted
            || !matches!(
                state.decision.as_ref(),
                Some(decision) if decision.decision == Decision::Accepted
                    && !decision.reason.trim().is_empty()
            )
        {
            return Err(invalid(
                "frontier baseline acceptance requires an accepted decision",
            ));
        }
        let ledger_path = safe_repository_path(&self.repository_root, path, false)?;
        let (stored, stored_bytes) = self.read_state(&state.configuration.run_id)?;
        let old_ledger_bytes = match fs::read(&ledger_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => json_bytes(
                &FrontierBaselineLedger {
                    version: LEDGER_VERSION,
                    baselines: Vec::new(),
                },
                "frontier baseline ledger",
            )?,
            Err(error) => return Err(io_error(&ledger_path, error)),
        };
        let old_ledger: FrontierBaselineLedger = serde_json::from_slice(&old_ledger_bytes)
            .map_err(|error| invalid(format!("frontier baseline ledger is malformed: {error}")))?;
        validate_ledger(&self.repository_root, &old_ledger)?;
        let state_bytes = json_bytes(state, "frontier snapshot")?;
        let ledger_bytes = json_bytes(ledger, "frontier baseline ledger")?;
        if stored == *state {
            if old_ledger_bytes == ledger_bytes {
                return Ok(());
            }
            return Err(invalid(
                "accepted frontier replay conflicts with the baseline ledger",
            ));
        }
        validate_transition(&stored, state, true)?;
        validate_ledger_successor(&old_ledger, ledger, state, &state_bytes)?;
        validate_ledger_with_pending(
            &self.repository_root,
            ledger,
            Some((
                self.relative_state_path(&state.configuration.run_id),
                hex_digest(&state_bytes),
            )),
        )?;
        let transaction = BaselineTransaction {
            state_path: self.relative_state_path(&state.configuration.run_id),
            ledger_path: path.to_path_buf(),
            old_state_sha256: hex_digest(&stored_bytes),
            old_ledger_sha256: hex_digest(&old_ledger_bytes),
            state_bytes,
            ledger_bytes,
        };
        self.write_transaction(&transaction)?;
        self.fail(FrontierFailurePoint::Journal)?;
        self.finish_transaction(&transaction)
    }

    fn load_suite_evidence<T>(&self, path: &Path, kind: &str) -> Result<T, SkillEvalError>
    where
        T: for<'de> Deserialize<'de> + Serialize,
    {
        let path = safe_repository_path(&self.repository_root, path, true)?;
        read_strict_json(&path, kind)
    }

    fn save_immutable_suite_evidence(
        &mut self,
        path: &Path,
        bytes: &[u8],
        kind: &str,
        failure: FrontierFailurePoint,
        after_link_failure: FrontierFailurePoint,
    ) -> Result<(), SkillEvalError> {
        let destination = safe_suite_destination(&self.repository_root, path)?;
        match fs::read(&destination) {
            Ok(stored) if stored == bytes => return Ok(()),
            Ok(_) => {
                return Err(invalid(format!(
                    "{kind} destination has conflicting evidence"
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(&destination, error)),
        }
        let temporary = write_temporary_bytes(&destination, bytes)?;
        if let Err(error) = self.fail(failure).and_then(|()| {
            fs::hard_link(&temporary, &destination).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    invalid(format!("{kind} destination was written concurrently"))
                } else {
                    io_error(&destination, error)
                }
            })
        }) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        let directory = destination.parent().expect("safe path has a parent");
        let result = self
            .fail(after_link_failure)
            .and_then(|()| sync_directory(directory))
            .and_then(|()| fs::remove_file(&temporary).map_err(|error| io_error(&temporary, error)))
            .and_then(|()| sync_directory(directory));
        if let Err(error) = result {
            return Err(rollback_result(
                "frontier immutable authority",
                error,
                rollback_immutable_authority(&destination, &temporary, directory),
            ));
        }
        Ok(())
    }

    fn replace_suite_bytes(
        &mut self,
        destination: &Path,
        bytes: &[u8],
    ) -> Result<(), SkillEvalError> {
        let prior = match fs::read(destination) {
            Ok(stored) if stored == bytes => return Ok(()),
            Ok(stored) => Some(stored),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(io_error(destination, error)),
        };
        let temporary = write_temporary_bytes(destination, bytes)?;
        if let Err(error) = self.fail(FrontierFailurePoint::Suite).and_then(|()| {
            fs::rename(&temporary, destination).map_err(|error| io_error(destination, error))
        }) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        let directory = destination.parent().expect("safe path has a parent");
        if let Err(error) = self
            .fail(FrontierFailurePoint::SuiteAfterRename)
            .and_then(|()| sync_directory(directory))
        {
            let rollback = rollback_suite_authority(destination, prior.as_deref(), directory);
            let _ = fs::remove_file(&temporary);
            return Err(rollback_result("frontier suite authority", error, rollback));
        }
        Ok(())
    }

    fn read_state(
        &self,
        run_id: &FrontierRunId,
    ) -> Result<(FrontierRunState, Vec<u8>), SkillEvalError> {
        let path = self.state_path(run_id)?;
        let bytes = fs::read(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SkillEvalError::NotFound("frontier run has no snapshot".to_owned())
            } else {
                io_error(&path, error)
            }
        })?;
        let state: FrontierRunState = strict_json_bytes(&bytes, &path, "frontier snapshot")?;
        if state.configuration.run_id != *run_id {
            return Err(invalid("frontier snapshot identity differs from its path"));
        }
        validate_state(&self.repository_root, &state, false)?;
        self.validate_stored_trials(&state)?;
        Ok((state, bytes))
    }

    fn validate_stored_trials(&self, state: &FrontierRunState) -> Result<(), SkillEvalError> {
        let trials = self
            .existing_run_directory(&state.configuration.run_id)?
            .join(TRIALS_DIRECTORY);
        let mut stored = Vec::new();
        for entry in fs::read_dir(&trials).map_err(|error| io_error(&trials, error))? {
            let entry = entry.map_err(|error| io_error(&trials, error))?;
            if !entry
                .file_type()
                .map_err(|error| io_error(&entry.path(), error))?
                .is_file()
            {
                return Err(invalid("frontier trial storage contains a non-file entry"));
            }
            let trial: TrialRecord = read_strict_json(&entry.path(), "frontier trial")?;
            validate_trial(&self.repository_root, state, &trial)?;
            if entry.file_name()
                != std::ffi::OsString::from(format!("{}.json", trial_identity_digest(&trial)?))
            {
                return Err(invalid(
                    "frontier trial path differs from its exact identity",
                ));
            }
            stored.push(trial);
        }
        for cell in &state.cells {
            let count = stored
                .iter()
                .filter(|trial| trial.model == cell.model)
                .count();
            if usize::try_from(cell.completed_trials).map_or(true, |completed| completed > count) {
                return Err(invalid("frontier cell advances beyond its durable trials"));
            }
        }
        Ok(())
    }

    fn run_directory(&self, run_id: &FrontierRunId) -> Result<PathBuf, SkillEvalError> {
        validate_identifier(&run_id.0, "frontier run")?;
        Ok(self.frontier_root.join(&run_id.0))
    }

    fn existing_run_directory(&self, run_id: &FrontierRunId) -> Result<PathBuf, SkillEvalError> {
        let expected = self.run_directory(run_id)?;
        let canonical = fs::canonicalize(&expected).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SkillEvalError::NotFound("frontier run does not exist".to_owned())
            } else {
                io_error(&expected, error)
            }
        })?;
        if canonical != expected || !canonical.is_dir() {
            return Err(invalid("frontier run escapes its storage root"));
        }
        Ok(canonical)
    }

    fn state_path(&self, run_id: &FrontierRunId) -> Result<PathBuf, SkillEvalError> {
        Ok(self.existing_run_directory(run_id)?.join(SNAPSHOT_NAME))
    }

    fn relative_state_path(&self, run_id: &FrontierRunId) -> PathBuf {
        FRONTIER_ROOT
            .iter()
            .fold(PathBuf::new(), |path, component| path.join(component))
            .join(&run_id.0)
            .join(SNAPSHOT_NAME)
    }

    fn transaction_path(&self) -> PathBuf {
        self.frontier_root.join(TRANSACTION_NAME)
    }

    fn write_transaction(
        &mut self,
        transaction: &BaselineTransaction,
    ) -> Result<(), SkillEvalError> {
        let path = self.transaction_path();
        let bytes = json_bytes(transaction, "frontier baseline transaction")?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    invalid("another frontier baseline transaction is active")
                } else {
                    io_error(&path, error)
                }
            })?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| io_error(&path, error))?;
        sync_directory(&self.frontier_root)
    }

    fn recover_transaction(&mut self) -> Result<(), SkillEvalError> {
        let path = self.transaction_path();
        let transaction = match fs::read(&path) {
            Ok(bytes) => strict_json_bytes(&bytes, &path, "frontier baseline transaction")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error(&path, error)),
        };
        self.finish_transaction(&transaction)
    }

    fn finish_transaction(
        &mut self,
        transaction: &BaselineTransaction,
    ) -> Result<(), SkillEvalError> {
        let state_path =
            safe_repository_path(&self.repository_root, &transaction.state_path, true)?;
        let ledger_path =
            safe_repository_path(&self.repository_root, &transaction.ledger_path, false)?;
        if state_path.parent().and_then(Path::parent) != Some(&self.frontier_root) {
            return Err(invalid("frontier transaction state path is not owned"));
        }
        replace_transaction_side(
            &state_path,
            &transaction.old_state_sha256,
            &transaction.state_bytes,
        )?;
        self.fail(FrontierFailurePoint::State)?;
        replace_transaction_side(
            &ledger_path,
            &transaction.old_ledger_sha256,
            &transaction.ledger_bytes,
        )?;
        self.fail(FrontierFailurePoint::Ledger)?;
        fs::remove_file(self.transaction_path())
            .map_err(|error| io_error(&self.transaction_path(), error))?;
        sync_directory(&self.frontier_root)
    }

    #[cfg(test)]
    fn fail(&mut self, point: FrontierFailurePoint) -> Result<(), SkillEvalError> {
        if self.failure == Some(point) {
            self.failure = None;
            return Err(invalid(format!("injected {point:?} failure")));
        }
        Ok(())
    }

    #[cfg(not(test))]
    fn fail(&mut self, _point: FrontierFailurePoint) -> Result<(), SkillEvalError> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrontierFailurePoint {
    Journal,
    State,
    Ledger,
    Inventory,
    InventoryAfterLink,
    Proposal,
    ProposalAfterLink,
    Suite,
    SuiteAfterRename,
}

#[derive(Deserialize, Serialize)]
struct BaselineTransaction {
    state_path: PathBuf,
    ledger_path: PathBuf,
    old_state_sha256: String,
    old_ledger_sha256: String,
    state_bytes: Vec<u8>,
    ledger_bytes: Vec<u8>,
}

fn validate_suite_plan(plan: &FrontierSuiteConstructionPlan) -> Result<(), SkillEvalError> {
    if plan.version != 1 || plan.artifact_roots.is_empty() {
        return Err(invalid(
            "frontier suite construction plan version or roots are invalid",
        ));
    }
    let mut roots = BTreeSet::new();
    for root in &plan.artifact_roots {
        validate_suite_relative_path(root, "frontier construction artifact root")?;
        if !roots.insert(root) {
            return Err(invalid(
                "frontier construction artifact roots are duplicate",
            ));
        }
    }
    validate_suite_policy(&plan.policy)
}

fn validate_suite_inventory(inventory: &FrontierSuiteInventory) -> Result<(), SkillEvalError> {
    if inventory.version != 1 || inventory.generated_at.0.trim().is_empty() {
        return Err(invalid(
            "frontier suite inventory version or generation time is invalid",
        ));
    }
    let mut previous = None;
    for entry in &inventory.cases {
        validate_suite_key(&entry.key, "frontier inventory case")?;
        if previous.is_some_and(|key| key >= &entry.key) {
            return Err(invalid(
                "frontier inventory cases are duplicate or unsorted",
            ));
        }
        validate_suite_drive(&entry.drive)?;
        previous = Some(&entry.key);
    }
    Ok(())
}

fn validate_suite_drive(drive: &crate::model::CaseDrive) -> Result<(), SkillEvalError> {
    match drive {
        crate::model::CaseDrive::Response => Err(invalid(
            "frontier inventory contains unsupported response drive",
        )),
        crate::model::CaseDrive::Fixture {
            source,
            verify_commands,
        } => {
            validate_loaded_suite_path(source, "frontier inventory fixture")?;
            for command in verify_commands {
                validate_suite_command(command)?;
            }
            Ok(())
        }
        crate::model::CaseDrive::ExistingHarness { command } => validate_suite_command(command),
    }
}

fn validate_suite_command(command: &crate::model::CommandDefinition) -> Result<(), SkillEvalError> {
    if command.program.trim().is_empty()
        || command.program.contains('\0')
        || command.arguments.iter().any(|value| value.contains('\0'))
    {
        return Err(invalid("frontier inventory command is invalid"));
    }
    if let Some(directory) = &command.working_directory {
        validate_loaded_suite_path(directory, "frontier inventory working directory")?;
    }
    Ok(())
}

fn validate_loaded_suite_path(path: &Path, kind: &str) -> Result<(), SkillEvalError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        || path.to_string_lossy().chars().any(char::is_control)
    {
        return Err(invalid(format!("{kind} path is unsafe")));
    }
    Ok(())
}

fn validate_suite_reviews(reviews: &FrontierSuiteReviewSet) -> Result<(), SkillEvalError> {
    if reviews.version != 1 {
        return Err(invalid("frontier suite review-set version is invalid"));
    }
    validate_digest(&reviews.inventory_sha256)?;
    let mut previous: Option<(&FrontierCaseKey, &str)> = None;
    for record in &reviews.records {
        validate_suite_key(&record.key, "frontier review case")?;
        if record.reviewer.trim().is_empty() || record.reviewed_at.0.trim().is_empty() {
            return Err(invalid(
                "frontier review has an invalid reviewer or review time",
            ));
        }
        let identity = (&record.key, record.reviewer.as_str());
        if previous.is_some_and(|stored| stored >= identity) {
            return Err(invalid("frontier reviews are duplicate or unsorted"));
        }
        let evidence = match &record.decision {
            FrontierCaseReviewDecision::Eligible {
                relative_difficulty_basis_points,
                evidence,
                ..
            } => {
                if !(1..=10_000).contains(relative_difficulty_basis_points) {
                    return Err(invalid("frontier review difficulty is invalid"));
                }
                evidence
            }
            FrontierCaseReviewDecision::Rejected { evidence, .. } => evidence,
        };
        if evidence.is_empty() || evidence.iter().any(|item| item.trim().is_empty()) {
            return Err(invalid("frontier review evidence is invalid"));
        }
        previous = Some(identity);
    }
    Ok(())
}

fn validate_suite_proposal(proposal: &FrontierSuiteProposal) -> Result<(), SkillEvalError> {
    if proposal.version != 1 {
        return Err(invalid("frontier suite proposal version is invalid"));
    }
    validate_digest(&proposal.inventory_sha256)?;
    validate_digest(&proposal.review_set_sha256)?;
    validate_suite_policy(&proposal.policy)?;
    validate_sorted_suite_keys(
        &proposal.calibration_anchors,
        "frontier calibration anchors",
    )?;
    validate_sorted_suite_keys(&proposal.holdout_cases, "frontier holdout cases")?;
    if proposal.proposed_tiers.len() != proposal.policy.required_tiers.len()
        || proposal.tier_capacity.len() != proposal.policy.required_tiers.len()
    {
        return Err(invalid(
            "frontier proposal does not contain every required tier",
        ));
    }
    let holdouts = proposal.holdout_cases.iter().collect::<BTreeSet<_>>();
    let anchors = proposal.calibration_anchors.iter().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut rejected_cases = None;
    let mut is_ready = true;
    for tier in &proposal.policy.required_tiers {
        let suite = proposal
            .proposed_tiers
            .get(tier)
            .ok_or_else(|| invalid("frontier proposal is missing a required tier"))?;
        validate_suite_weights(&suite.group_weights_basis_points)?;
        if suite.group_weights_basis_points != proposal.policy.group_weights_basis_points {
            return Err(invalid("frontier proposal tier weights drift from policy"));
        }
        let capacity = proposal
            .tier_capacity
            .get(tier)
            .ok_or_else(|| invalid("frontier proposal is missing tier capacity"))?;
        let mut unique = BTreeSet::new();
        let mut groups = BTreeSet::new();
        for reference in &suite.cases {
            let key = FrontierCaseKey {
                artifact_path: reference.artifact_path.clone(),
                artifact_revision: reference.artifact_revision.clone(),
                case: reference.case.clone(),
            };
            validate_suite_key(&key, "frontier proposal case")?;
            unique.insert(key.clone());
            groups.insert(reference.group);
            if !seen.insert(key.clone()) {
                return Err(invalid("frontier proposal reuses a case across tiers"));
            }
            if anchors.contains(&key) {
                return Err(invalid("frontier proposal counts a calibration anchor"));
            }
            if reference.is_confirmation && !holdouts.contains(&key) {
                return Err(invalid(
                    "frontier proposal confirmation is absent from holdouts",
                ));
            }
        }
        let accepted = u16::try_from(unique.len())
            .map_err(|_| invalid("frontier proposal case count overflow"))?;
        let total = u16::try_from(suite.cases.len())
            .map_err(|_| invalid("frontier proposal case count overflow"))?;
        let duplicate = total.saturating_sub(accepted);
        let shortfall = proposal
            .policy
            .minimum_unique_cases_per_tier
            .saturating_sub(accepted);
        let is_complete = shortfall == 0 && groups.len() == 4;
        if capacity.required_unique_cases != proposal.policy.minimum_unique_cases_per_tier
            || capacity.accepted_unique_cases != accepted
            || capacity.shortfall != shortfall
            || capacity.duplicate_cases != duplicate
            || capacity.is_complete != is_complete
            || rejected_cases.is_some_and(|stored| stored != capacity.rejected_cases)
        {
            return Err(invalid(
                "frontier proposal capacity is forged or inconsistent",
            ));
        }
        rejected_cases = Some(capacity.rejected_cases);
        is_ready &= is_complete;
    }
    if (proposal.status == FrontierSuiteProposalStatus::Ready) != is_ready {
        return Err(invalid("frontier proposal status differs from capacity"));
    }
    if proposal.status == FrontierSuiteProposalStatus::Ready {
        frontier_source::frontier_suite_from_ready_proposal(proposal)?;
    }
    Ok(())
}

fn validate_suite_policy(policy: &FrontierSuiteConstructionPolicy) -> Result<(), SkillEvalError> {
    if policy.required_tiers != [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5]
        || policy.minimum_unique_cases_per_tier < 30
        || policy.minimum_reviewers_per_case < 2
        || !policy.is_unanimous_eligibility_required
        || policy.is_cross_tier_reuse_allowed
        || policy.is_calibration_anchor_counted_toward_minimum
    {
        return Err(invalid("frontier suite construction policy is invalid"));
    }
    validate_suite_weights(&policy.group_weights_basis_points)
}

fn validate_suite_weights(
    weights: &std::collections::BTreeMap<FrontierCaseGroup, u16>,
) -> Result<(), SkillEvalError> {
    let required = [
        FrontierCaseGroup::Normal,
        FrontierCaseGroup::Edge,
        FrontierCaseGroup::Adversarial,
        FrontierCaseGroup::Critical,
    ];
    let total = weights
        .values()
        .try_fold(0_u16, |sum, weight| sum.checked_add(*weight));
    if weights.len() != required.len()
        || required
            .iter()
            .any(|group| weights.get(group).is_none_or(|weight| *weight == 0))
        || total != Some(10_000)
    {
        return Err(invalid("frontier suite group weights are invalid"));
    }
    Ok(())
}

fn validate_sorted_suite_keys(keys: &[FrontierCaseKey], kind: &str) -> Result<(), SkillEvalError> {
    if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(invalid(format!("{kind} are duplicate or unsorted")));
    }
    for key in keys {
        validate_suite_key(key, kind)?;
    }
    Ok(())
}

fn validate_suite_key(key: &FrontierCaseKey, kind: &str) -> Result<(), SkillEvalError> {
    validate_suite_relative_path(&key.artifact_path, kind)?;
    if key.artifact_revision.trim().is_empty() || key.case.0.trim().is_empty() {
        return Err(invalid(format!("{kind} identity is incomplete")));
    }
    Ok(())
}

fn validate_suite_relative_path(path: &Path, kind: &str) -> Result<(), SkillEvalError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || path.to_string_lossy().chars().any(char::is_control)
    {
        return Err(invalid(format!("{kind} path is unsafe")));
    }
    Ok(())
}

fn validate_state(
    repository_root: &Path,
    state: &FrontierRunState,
    is_initial: bool,
) -> Result<(), SkillEvalError> {
    validate_identifier(&state.configuration.run_id.0, "frontier run")?;
    validate_plan_evidence_path(
        repository_root,
        &state.configuration.plan_path,
        &state.configuration.plan_sha256,
        &state.configuration.plan,
    )?;
    validate_evidence_path(
        repository_root,
        &FrontierEvidenceIdentity {
            path: state.configuration.plan.suite.path.clone(),
            sha256: state.configuration.plan.suite.sha256.clone(),
        },
    )?;
    validate_evidence_path(
        repository_root,
        &FrontierEvidenceIdentity {
            path: state.configuration.plan.capabilities.path.clone(),
            sha256: state.configuration.plan.capabilities.sha256.clone(),
        },
    )?;
    if state.configuration.plan.version != 1
        || state.configuration.plan.suite.version == 0
        || state.configuration.plan.capabilities.version == 0
    {
        return Err(invalid("frontier frozen evidence version is unsupported"));
    }
    let entrants = &state.configuration.plan.entrants;
    if entrants.is_empty() || entrants.len() != state.models.len() {
        return Err(invalid(
            "frontier model progress does not match its entrants",
        ));
    }
    let mut identities = BTreeSet::new();
    for (entrant, progress) in entrants.iter().zip(&state.models) {
        if entrant.provider != progress.provider
            || entrant.model != progress.model
            || entrant.entry_tier != progress.entry_tier
            || entrant.thinking_levels.is_empty()
            || !identities.insert((&entrant.provider, &entrant.model))
        {
            return Err(invalid("frontier model identity drifted or is duplicated"));
        }
        for route in &progress.selected_routes {
            if route.provider != entrant.provider
                || route.model != entrant.model
                || !entrant.thinking_levels.contains(&route.thinking)
            {
                return Err(invalid("frontier selected route differs from its entrant"));
            }
        }
    }
    let mut cells = BTreeSet::new();
    for cell in &state.cells {
        if cell.completed_trials > cell.expected_trials
            || cell.failed_trials > cell.completed_trials
            || cell.total_usage.cost_millionths_of_dollar > state.spent_millionths_of_dollar
            || !cells.insert((
                &cell.model.provider,
                &cell.model.model,
                cell.model.tier,
                &cell.model.thinking,
            ))
        {
            return Err(invalid("frontier cell evidence is malformed or duplicated"));
        }
        let entrant = entrants
            .iter()
            .find(|entrant| {
                entrant.provider == cell.model.provider && entrant.model == cell.model.model
            })
            .ok_or_else(|| invalid("frontier cell belongs to an unknown entrant"))?;
        if !entrant.thinking_levels.contains(&cell.model.thinking) {
            return Err(invalid("frontier cell thinking identity drifted"));
        }
        let is_terminal = matches!(
            cell.status,
            FrontierCellStatus::Passed
                | FrontierCellStatus::Failed
                | FrontierCellStatus::Indeterminate
                | FrontierCellStatus::Skipped
        );
        if is_terminal && cell.status != FrontierCellStatus::Skipped && cell.completed_trials == 0 {
            return Err(invalid("frontier terminal cell has no evidence"));
        }
    }
    if state.spent_millionths_of_dollar
        > state
            .configuration
            .plan
            .policy
            .spending_limit_millionths_of_dollar
    {
        return Err(invalid("frontier spending exceeds its frozen limit"));
    }
    if is_initial
        && (state.status != FrontierRunStatus::Pending
            || !state.cells.is_empty()
            || !state.infrastructure_events.is_empty()
            || state.pause.is_some()
            || state.decision.is_some()
            || state.spent_millionths_of_dollar != 0
            || state
                .models
                .iter()
                .any(|model| !model.selected_routes.is_empty() || model.is_exhausted))
    {
        return Err(invalid("new frontier state must be pending and unspent"));
    }
    Ok(())
}

fn validate_transition(
    stored: &FrontierRunState,
    next: &FrontierRunState,
    is_acceptance: bool,
) -> Result<(), SkillEvalError> {
    if stored.configuration != next.configuration {
        return Err(invalid("frontier frozen authority changed after creation"));
    }
    if stored.status != next.status
        && !matches!(
            (stored.status, next.status),
            (FrontierRunStatus::Pending, FrontierRunStatus::Running)
                | (FrontierRunStatus::Running, FrontierRunStatus::Paused)
                | (
                    FrontierRunStatus::Running,
                    FrontierRunStatus::AwaitingDecision
                )
                | (FrontierRunStatus::Running, FrontierRunStatus::Failed)
                | (FrontierRunStatus::Paused, FrontierRunStatus::Running)
                | (FrontierRunStatus::Paused, FrontierRunStatus::Failed)
                | (
                    FrontierRunStatus::AwaitingDecision,
                    FrontierRunStatus::Rejected
                )
                | (
                    FrontierRunStatus::AwaitingDecision,
                    FrontierRunStatus::Accepted
                )
        )
    {
        return Err(invalid("frontier run status transition is illegal"));
    }
    if next.status == FrontierRunStatus::Accepted && !is_acceptance {
        return Err(invalid(
            "frontier acceptance must include its baseline ledger",
        ));
    }
    if stored.models.len() != next.models.len() {
        return Err(invalid("frontier model identities changed"));
    }
    for (old, new) in stored.models.iter().zip(&next.models) {
        if old.provider != new.provider
            || old.model != new.model
            || old.entry_tier != new.entry_tier
            || !new.selected_routes.starts_with(&old.selected_routes)
            || (old.is_exhausted && !new.is_exhausted)
        {
            return Err(invalid("frontier model progress regressed or drifted"));
        }
    }
    if next.cells.len() < stored.cells.len() {
        return Err(invalid("frontier cell evidence was removed"));
    }
    for (old, new) in stored.cells.iter().zip(&next.cells) {
        if old.model != new.model
            || new.completed_trials < old.completed_trials
            || new.expected_trials != old.expected_trials
            || new.failed_trials < old.failed_trials
            || !usage_is_nondecreasing(&old.total_usage, &new.total_usage)
            || (is_terminal_cell(old.status) && old != new)
        {
            return Err(invalid(
                "frontier cell evidence regressed or changed identity",
            ));
        }
    }
    if !next
        .infrastructure_events
        .starts_with(&stored.infrastructure_events)
        || next.spent_millionths_of_dollar < stored.spent_millionths_of_dollar
        || stored
            .decision
            .as_ref()
            .is_some_and(|decision| next.decision.as_ref() != Some(decision))
    {
        return Err(invalid(
            "frontier evidence, decision, or spending regressed",
        ));
    }
    Ok(())
}

fn validate_trial(
    repository_root: &Path,
    state: &FrontierRunState,
    trial: &TrialRecord,
) -> Result<(), SkillEvalError> {
    let entrant = state
        .configuration
        .plan
        .entrants
        .iter()
        .find(|entrant| {
            entrant.provider == trial.model.provider && entrant.model == trial.model.model
        })
        .ok_or_else(|| invalid("frontier trial model is not scheduled"))?;
    if trial.model.tier < entrant.entry_tier
        || !entrant.thinking_levels.contains(&trial.model.thinking)
        || trial.judge_model != state.configuration.plan.judge
        || trial.key.tier != trial.model.tier
        || trial.key.attempt == 0
        || trial.key.attempt > u16::from(state.configuration.plan.policy.maximum_trials_per_case)
        || trial.verdict.score > 10
        || trial.harness.runner_version.trim().is_empty()
        || trial.harness.pi_version.trim().is_empty()
        || trial.harness.artifact_revision.trim().is_empty()
        || trial.harness.tool_policy_digest.trim().is_empty()
    {
        return Err(invalid("frontier trial identity or evidence is incomplete"));
    }
    let suite_path =
        safe_repository_path(repository_root, &state.configuration.plan.suite.path, true)?;
    let suite: FrontierSuite = read_strict_json(&suite_path, "frontier suite")?;
    let tier = suite
        .tiers
        .get(&trial.key.tier)
        .ok_or_else(|| invalid("frontier trial tier is absent from the suite"))?;
    let reference = tier
        .cases
        .iter()
        .find(|reference| {
            reference.case == trial.key.case
                && artifact_name(&reference.artifact_path) == Some(trial.key.artifact.0.as_str())
        })
        .ok_or_else(|| invalid("frontier trial case is not scheduled"))?;
    let maximum_attempt = if reference.is_confirmation {
        state.configuration.plan.policy.maximum_trials_per_case
    } else {
        state.configuration.plan.policy.screening_trials_per_case
    };
    if trial.key.attempt > u16::from(maximum_attempt)
        || trial.harness.artifact_revision != reference.artifact_revision
    {
        return Err(invalid(
            "frontier trial attempt or source revision is out of schedule",
        ));
    }
    let run_root = RUN_EVIDENCE_ROOT
        .iter()
        .fold(repository_root.to_path_buf(), |path, component| {
            path.join(component)
        })
        .join(&state.configuration.run_id.0);
    validate_trial_evidence_path(repository_root, &run_root, &trial.artifact_path, false)?;
    validate_trial_evidence_path(repository_root, &run_root, &trial.transcript_path, true)
}

fn validate_ledger(
    repository_root: &Path,
    ledger: &FrontierBaselineLedger,
) -> Result<(), SkillEvalError> {
    validate_ledger_with_pending(repository_root, ledger, None)
}

fn validate_ledger_with_pending(
    repository_root: &Path,
    ledger: &FrontierBaselineLedger,
    pending: Option<(PathBuf, String)>,
) -> Result<(), SkillEvalError> {
    if ledger.version != LEDGER_VERSION {
        return Err(invalid("frontier baseline ledger version is unsupported"));
    }
    let mut previous = None;
    let mut runs = BTreeSet::new();
    for baseline in &ledger.baselines {
        if baseline.previous_entry_sha256 != previous
            || !runs.insert(&baseline.run_id)
            || baseline.accepted_at.0.trim().is_empty()
        {
            return Err(invalid(
                "frontier baseline ledger chain or identity is invalid",
            ));
        }
        let is_pending = pending.as_ref().is_some_and(|(path, sha256)| {
            baseline.run_evidence.path == *path && baseline.run_evidence.sha256 == *sha256
        });
        if !is_pending {
            validate_evidence_path(repository_root, &baseline.run_evidence)?;
            validate_baseline_run(repository_root, baseline)?;
        }
        for capability in &baseline.capabilities {
            validate_evidence_path(repository_root, &capability.evidence)?;
        }
        previous = Some(baseline_digest(baseline)?);
    }
    Ok(())
}

fn validate_baseline_run(
    repository_root: &Path,
    baseline: &FrontierBaseline,
) -> Result<(), SkillEvalError> {
    let expected = FRONTIER_ROOT
        .iter()
        .fold(PathBuf::new(), |path, component| path.join(component))
        .join(&baseline.run_id.0)
        .join(SNAPSHOT_NAME);
    if baseline.run_evidence.path != expected {
        return Err(invalid(
            "frontier baseline run evidence path differs from its identity",
        ));
    }
    let path = safe_repository_path(repository_root, &baseline.run_evidence.path, true)?;
    let state: FrontierRunState = read_strict_json(&path, "frontier accepted snapshot")?;
    if state.configuration.run_id != baseline.run_id
        || state.status != FrontierRunStatus::Accepted
        || !matches!(
            state.decision.as_ref(),
            Some(decision) if decision.decision == Decision::Accepted
        )
    {
        return Err(invalid(
            "frontier baseline references non-accepted evidence",
        ));
    }
    validate_state(repository_root, &state, false)
}

fn validate_ledger_successor(
    stored: &FrontierBaselineLedger,
    next: &FrontierBaselineLedger,
    state: &FrontierRunState,
    state_bytes: &[u8],
) -> Result<(), SkillEvalError> {
    if next.version != LEDGER_VERSION
        || next.baselines.len() != stored.baselines.len() + 1
        || !next.baselines.starts_with(&stored.baselines)
    {
        return Err(invalid("frontier baseline ledger is not one exact suffix"));
    }
    let suffix = next.baselines.last().expect("one suffix exists");
    let previous = stored.baselines.last().map(baseline_digest).transpose()?;
    let expected_path = FRONTIER_ROOT
        .iter()
        .fold(PathBuf::new(), |path, component| path.join(component))
        .join(&state.configuration.run_id.0)
        .join(SNAPSHOT_NAME);
    if suffix.run_id != state.configuration.run_id
        || suffix.previous_entry_sha256 != previous
        || suffix.run_evidence.path != expected_path
        || suffix.run_evidence.sha256 != hex_digest(state_bytes)
    {
        return Err(invalid(
            "frontier baseline suffix has stale or conflicting authority",
        ));
    }
    Ok(())
}

fn validate_plan_evidence_path(
    repository_root: &Path,
    relative_path: &Path,
    semantic_sha256: &str,
    expected: &FrontierPlan,
) -> Result<(), SkillEvalError> {
    validate_digest(semantic_sha256)?;
    let path = safe_repository_path(repository_root, relative_path, true)?;
    let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
    if hex_digest(&bytes) == semantic_sha256 {
        return Ok(());
    }
    let parsed: FrontierPlan = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(format!("frontier plan is malformed: {error}")))?;
    if &parsed != expected {
        return Err(invalid("frontier plan differs from stored authority"));
    }
    let semantic_bytes = serde_json::to_vec(&parsed)
        .map_err(|error| invalid(format!("frontier plan serialization failed: {error}")))?;
    if hex_digest(&semantic_bytes) != semantic_sha256 {
        return Err(invalid(
            "frontier plan semantic digest differs from stored identity",
        ));
    }
    Ok(())
}

fn validate_evidence_path(
    repository_root: &Path,
    evidence: &FrontierEvidenceIdentity,
) -> Result<(), SkillEvalError> {
    validate_digest(&evidence.sha256)?;
    let path = safe_repository_path(repository_root, &evidence.path, true)?;
    let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
    if hex_digest(&bytes) != evidence.sha256 {
        return Err(invalid(
            "frontier evidence digest differs from stored identity",
        ));
    }
    Ok(())
}

fn validate_trial_evidence_path(
    repository_root: &Path,
    run_root: &Path,
    path: &Path,
    is_regular_file_required: bool,
) -> Result<(), SkillEvalError> {
    let lexical_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository_root.join(path)
    };
    let metadata =
        fs::symlink_metadata(&lexical_path).map_err(|error| io_error(&lexical_path, error))?;
    if metadata.file_type().is_symlink() {
        return Err(invalid("frontier trial evidence path is a symlink"));
    }
    let path = if path.is_absolute() {
        let canonical = fs::canonicalize(path).map_err(|error| io_error(path, error))?;
        if !canonical.starts_with(repository_root) {
            return Err(invalid("frontier trial evidence escapes the repository"));
        }
        canonical
    } else {
        safe_repository_path(repository_root, path, true)?
    };
    if !path.starts_with(run_root)
        || is_regular_file_required && !path.is_file()
        || !is_regular_file_required && !path.is_file() && !path.is_dir()
    {
        return Err(invalid(
            "frontier trial evidence is outside its run or has an invalid kind",
        ));
    }
    Ok(())
}

fn validate_selector(selector: &FrontierTrialSelector) -> Result<(), SkillEvalError> {
    validate_identifier(&selector.run_id.0, "frontier run")?;
    for (value, kind) in [
        (&selector.provider, "frontier provider"),
        (&selector.model, "frontier model"),
        (&selector.thinking, "frontier thinking"),
        (&selector.artifact.0, "frontier artifact"),
        (&selector.case.0, "frontier case"),
    ] {
        if value.trim().is_empty()
            || value.contains(['/', '\\', '\0'])
            || value.chars().any(char::is_control)
            || matches!(value.as_str(), "." | "..")
        {
            return Err(invalid(format!("{kind} is unsafe")));
        }
    }
    if selector.attempt == 0 {
        return Err(invalid("frontier attempt must be positive"));
    }
    Ok(())
}

fn trial_matches(trial: &TrialRecord, selector: &FrontierTrialSelector) -> bool {
    trial.model.provider == selector.provider
        && trial.model.model == selector.model
        && trial.model.tier == selector.tier
        && trial.model.thinking == selector.thinking
        && trial.key.artifact == selector.artifact
        && trial.key.case == selector.case
        && trial.key.attempt == selector.attempt
}

fn trial_identity_digest(trial: &TrialRecord) -> Result<String, SkillEvalError> {
    let bytes = serde_json::to_vec(&(
        &trial.model.provider,
        &trial.model.model,
        trial.model.tier,
        &trial.model.thinking,
        trial.key.route_index,
        &trial.key.artifact,
        &trial.key.case,
        trial.key.attempt,
    ))
    .map_err(|error| {
        invalid(format!(
            "frontier trial identity serialization failed: {error}"
        ))
    })?;
    Ok(hex_digest(&bytes))
}

fn baseline_digest(baseline: &FrontierBaseline) -> Result<String, SkillEvalError> {
    serde_json::to_vec(baseline)
        .map(|bytes| hex_digest(&bytes))
        .map_err(|error| invalid(format!("frontier baseline serialization failed: {error}")))
}

fn artifact_name(path: &Path) -> Option<&str> {
    path.file_name().and_then(|name| name.to_str())
}

fn is_terminal_cell(status: FrontierCellStatus) -> bool {
    matches!(
        status,
        FrontierCellStatus::Passed
            | FrontierCellStatus::Failed
            | FrontierCellStatus::Indeterminate
            | FrontierCellStatus::Skipped
    )
}

fn usage_is_nondecreasing(stored: &TrialUsage, next: &TrialUsage) -> bool {
    next.input_tokens >= stored.input_tokens
        && next.output_tokens >= stored.output_tokens
        && next.cache_read_tokens >= stored.cache_read_tokens
        && next.cache_write_tokens >= stored.cache_write_tokens
        && next.turns >= stored.turns
        && next.tool_calls >= stored.tool_calls
        && next.elapsed_milliseconds >= stored.elapsed_milliseconds
        && next.cost_millionths_of_dollar >= stored.cost_millionths_of_dollar
}

fn replace_transaction_side(
    path: &Path,
    old_sha256: &str,
    new_bytes: &[u8],
) -> Result<(), SkillEvalError> {
    let current = fs::read(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound
            && hex_digest(
                &json_bytes(
                    &FrontierBaselineLedger {
                        version: LEDGER_VERSION,
                        baselines: Vec::new(),
                    },
                    "empty ledger",
                )
                .unwrap_or_default(),
            ) == old_sha256
        {
            return SkillEvalError::NotFound("transaction destination is absent".to_owned());
        }
        io_error(path, error)
    });
    match current {
        Ok(bytes) if hex_digest(&bytes) == hex_digest(new_bytes) => Ok(()),
        Ok(bytes) if hex_digest(&bytes) == old_sha256 => replace_bytes(path, new_bytes),
        Ok(_) => Err(invalid(
            "frontier transaction authority changed during recovery",
        )),
        Err(SkillEvalError::NotFound(_)) => replace_bytes(path, new_bytes),
        Err(error) => Err(error),
    }
}

fn replace_bytes_recoverable(
    path: &Path,
    bytes: &[u8],
    prior: &[u8],
) -> Result<(), SkillEvalError> {
    if let Err(error) = replace_bytes(path, bytes) {
        let _ = replace_bytes(path, prior);
        return Err(error);
    }
    Ok(())
}

pub(crate) fn rollback_result(
    authority: &str,
    initiating_error: SkillEvalError,
    rollback: Result<(), SkillEvalError>,
) -> SkillEvalError {
    match rollback {
        Ok(()) => initiating_error,
        Err(rollback_error) => invalid(format!(
            "{authority} rollback failed: {rollback_error:?}; initiating error: {initiating_error:?}"
        )),
    }
}

fn rollback_immutable_authority(
    destination: &Path,
    temporary: &Path,
    directory: &Path,
) -> Result<(), SkillEvalError> {
    fs::remove_file(destination).map_err(|error| io_error(destination, error))?;
    sync_directory(directory)?;
    match fs::remove_file(temporary) {
        Ok(()) => sync_directory(directory),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(temporary, error)),
    }
}

fn rollback_suite_authority(
    destination: &Path,
    prior: Option<&[u8]>,
    directory: &Path,
) -> Result<(), SkillEvalError> {
    match prior {
        Some(bytes) => replace_bytes(destination, bytes),
        None => {
            fs::remove_file(destination).map_err(|error| io_error(destination, error))?;
            sync_directory(directory)
        }
    }
}

fn write_temporary_bytes(path: &Path, bytes: &[u8]) -> Result<PathBuf, SkillEvalError> {
    let directory = path
        .parent()
        .ok_or_else(|| invalid("frontier destination has no parent"))?;
    fs::create_dir_all(directory).map_err(|error| io_error(directory, error))?;
    let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("frontier destination filename is invalid"))?;
    let temporary = directory.join(format!(".{name}.{}.{sequence}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| io_error(&temporary, error))?;
    if let Err(error) = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(&temporary, error))
    {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(temporary)
}

fn replace_bytes(path: &Path, bytes: &[u8]) -> Result<(), SkillEvalError> {
    let directory = path
        .parent()
        .ok_or_else(|| invalid("frontier destination has no parent"))?;
    fs::create_dir_all(directory).map_err(|error| io_error(directory, error))?;
    let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("frontier destination filename is invalid"))?;
    let temporary = directory.join(format!(".{name}.{}.{sequence}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| io_error(&temporary, error))?;
    let result = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(&temporary, error))
        .and_then(|()| fs::rename(&temporary, path).map_err(|error| io_error(path, error)))
        .and_then(|()| sync_directory(directory));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn json_bytes<T: Serialize>(value: &T, kind: &str) -> Result<Vec<u8>, SkillEvalError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| invalid(format!("{kind} serialization failed: {error}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn read_strict_json<T>(path: &Path, kind: &str) -> Result<T, SkillEvalError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let bytes = fs::read(path).map_err(|error| io_error(path, error))?;
    strict_json_bytes(&bytes, path, kind)
}

fn strict_json_bytes<T>(bytes: &[u8], path: &Path, kind: &str) -> Result<T, SkillEvalError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("{kind} {} is malformed: {error}", path.display())))?;
    let parsed: T = serde_json::from_value(value.clone())
        .map_err(|error| invalid(format!("{kind} {} is malformed: {error}", path.display())))?;
    let normalized = serde_json::to_value(&parsed)
        .map_err(|error| invalid(format!("{kind} validation failed: {error}")))?;
    if normalized != value {
        return Err(invalid(format!(
            "{kind} {} contains unknown data",
            path.display()
        )));
    }
    Ok(parsed)
}

fn safe_repository_path(
    repository_root: &Path,
    relative: &Path,
    is_existing: bool,
) -> Result<PathBuf, SkillEvalError> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid("frontier repository path is unsafe"));
    }
    let path = repository_root.join(relative);
    if is_existing {
        let canonical = fs::canonicalize(&path).map_err(|error| io_error(&path, error))?;
        if !canonical.starts_with(repository_root) {
            return Err(invalid("frontier repository path escapes its root"));
        }
        Ok(canonical)
    } else {
        let parent = path
            .parent()
            .ok_or_else(|| invalid("frontier repository path has no parent"))?;
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
        let canonical_parent = fs::canonicalize(parent).map_err(|error| io_error(parent, error))?;
        if !canonical_parent.starts_with(repository_root) {
            return Err(invalid("frontier repository path escapes its root"));
        }
        Ok(canonical_parent.join(path.file_name().expect("normal path has a filename")))
    }
}

fn safe_suite_destination(
    repository_root: &Path,
    relative: &Path,
) -> Result<PathBuf, SkillEvalError> {
    let path = safe_repository_path(repository_root, relative, false)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(invalid("frontier suite destination is a symlink"))
        }
        Ok(metadata) if !metadata.is_file() => {
            Err(invalid("frontier suite destination is not a regular file"))
        }
        Ok(_) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(error) => Err(io_error(&path, error)),
    }
}

fn canonical_repository_root(path: &Path) -> Result<PathBuf, SkillEvalError> {
    fs::create_dir_all(path).map_err(|error| io_error(path, error))?;
    let canonical = fs::canonicalize(path).map_err(|error| io_error(path, error))?;
    if !canonical.is_dir() {
        return Err(invalid("frontier repository root is not a directory"));
    }
    Ok(canonical)
}

fn create_contained_directory(
    repository_root: &Path,
    components: &[&str],
) -> Result<PathBuf, SkillEvalError> {
    let path = components
        .iter()
        .fold(repository_root.to_path_buf(), |path, component| {
            path.join(component)
        });
    fs::create_dir_all(&path).map_err(|error| io_error(&path, error))?;
    let canonical = fs::canonicalize(&path).map_err(|error| io_error(&path, error))?;
    if !canonical.starts_with(repository_root) || !canonical.is_dir() {
        return Err(invalid("frontier storage root escapes the repository"));
    }
    Ok(canonical)
}

fn sync_directory(path: &Path) -> Result<(), SkillEvalError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_error(path, error))
}

fn validate_identifier(identifier: &str, kind: &str) -> Result<(), SkillEvalError> {
    if identifier.is_empty()
        || !identifier
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(invalid(format!(
            "{kind} identifier is not a safe path component"
        )));
    }
    Ok(())
}

fn validate_digest(digest: &str) -> Result<(), SkillEvalError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid("frontier evidence digest is malformed"));
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
