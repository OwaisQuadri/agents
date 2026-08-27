use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use crate::model::{
    SkillEvalError, T1ScreenCampaignCapExtension, T1ScreenCampaignCapExtensionRequest,
    T1ScreenCampaignId, T1ScreenCampaignRunEntry, T1ScreenCampaignRunRetirement,
    T1ScreenCampaignRunRetirementRequest, T1ScreenCampaignState, T1ScreenCampaignStatus,
    T1ScreenPauseReason, T1ScreenRunId, T1ScreenRunState, T1ScreenRunStatus, Timestamp,
};
use crate::t1_screen_store::{validate_digest, validate_identifier, validate_t1_timestamp};

pub(crate) const T1_SCREEN_CAMPAIGN_APPROVED_TOTAL: u64 = 20_000_000;
const CAMPAIGN_ROOT: [&str; 3] = [".map", "skill-eval", "t1-screening-campaigns"];
const SCREENING_ROOT: [&str; 3] = [".map", "skill-eval", "t1-screening"];
const SNAPSHOT_NAME: &str = "state.json";
const LOCK_NAME: &str = ".state.lock";
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct FileT1ScreenCampaignStore {
    repository_root: PathBuf,
    campaigns_root: PathBuf,
    screening_root: PathBuf,
    #[cfg(test)]
    failure: Option<T1ScreenCampaignFailurePoint>,
}

impl FileT1ScreenCampaignStore {
    pub(crate) fn new(repository_root: impl AsRef<Path>) -> Result<Self, SkillEvalError> {
        let repository_root = canonical_repository_root(repository_root.as_ref())?;
        let campaigns_root = create_contained_directory(&repository_root, &CAMPAIGN_ROOT)?;
        let screening_root = create_contained_directory(&repository_root, &SCREENING_ROOT)?;
        Ok(Self {
            repository_root,
            campaigns_root,
            screening_root,
            #[cfg(test)]
            failure: None,
        })
    }

    pub(crate) fn open(repository_root: impl AsRef<Path>) -> Result<Self, SkillEvalError> {
        let repository_root = canonical_repository_root(repository_root.as_ref())?;
        let campaigns_root = existing_contained_directory(&repository_root, &CAMPAIGN_ROOT)?;
        let screening_root = existing_contained_directory(&repository_root, &SCREENING_ROOT)?;
        Ok(Self {
            repository_root,
            campaigns_root,
            screening_root,
            #[cfg(test)]
            failure: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_failure(
        repository_root: impl AsRef<Path>,
        failure: T1ScreenCampaignFailurePoint,
    ) -> Result<Self, SkillEvalError> {
        let mut store = Self::new(repository_root)?;
        store.failure = Some(failure);
        Ok(store)
    }

    pub(crate) fn create_from_runs(
        &mut self,
        campaign_id: &T1ScreenCampaignId,
        approved_total: u64,
        owner_reason: &str,
        created_at: Timestamp,
        run_ids: &[T1ScreenRunId],
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        validate_identifier(&campaign_id.0, "T1 screening campaign")?;
        validate_t1_timestamp(&created_at)?;
        if approved_total != T1_SCREEN_CAMPAIGN_APPROVED_TOTAL {
            return Err(invalid(format!(
                "T1 screening campaign judge total must be exactly {T1_SCREEN_CAMPAIGN_APPROVED_TOTAL} millionths"
            )));
        }
        if owner_reason.trim().is_empty() {
            return Err(invalid("T1 screening campaign owner reason is blank"));
        }
        let requested = run_ids
            .iter()
            .map(|run_id| {
                validate_identifier(&run_id.0, "T1 screening run")?;
                Ok(run_id.0.clone())
            })
            .collect::<Result<BTreeSet<_>, SkillEvalError>>()?;
        if requested.len() != run_ids.len() {
            return Err(invalid(
                "T1 screening campaign import contains duplicate run identifiers",
            ));
        }
        let available = self.available_run_ids()?;
        if requested != available {
            return Err(invalid(
                "T1 screening campaign import omitted or added a stored run",
            ));
        }
        let mut runs = run_ids
            .iter()
            .map(|run_id| self.import_run(campaign_id, run_id))
            .collect::<Result<Vec<_>, _>>()?;
        runs.sort_by(|left, right| {
            left.created_at
                .0
                .cmp(&right.created_at.0)
                .then_with(|| left.run_id.cmp(&right.run_id))
        });
        let aggregate = sum_run_spend(&runs)?;
        if aggregate > approved_total {
            return Err(invalid(
                "T1 screening campaign imported spend exceeds the approved total",
            ));
        }
        let state = T1ScreenCampaignState {
            campaign_id: campaign_id.clone(),
            created_at,
            approved_judge_total_millionths_of_dollar: approved_total,
            cap_extensions: Vec::new(),
            retirements: Vec::new(),
            aggregate_judge_spent_millionths_of_dollar: aggregate,
            runs,
            active_run_id: None,
            owner_reason: owner_reason.to_owned(),
            status: if aggregate == approved_total {
                T1ScreenCampaignStatus::Exhausted
            } else {
                T1ScreenCampaignStatus::Open
            },
        };
        self.create(&state)?;
        Ok(state)
    }

    pub(crate) fn create(&mut self, state: &T1ScreenCampaignState) -> Result<(), SkillEvalError> {
        validate_campaign_state(state)?;
        self.validate_store_paths(state)?;
        let directory = self.campaign_directory(&state.campaign_id)?;
        fs::create_dir(&directory).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                invalid(format!(
                    "T1 screening campaign {:?} already exists",
                    state.campaign_id.0
                ))
            } else {
                io_error(&directory, error)
            }
        })?;
        let snapshot = directory.join(SNAPSHOT_NAME);
        let campaigns_root = self.campaigns_root.clone();
        let result = self
            .replace_snapshot(&directory, &snapshot, state, None)
            .and_then(|()| self.sync_directory(&campaigns_root));
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&directory);
            let _ = File::open(&self.campaigns_root).and_then(|file| file.sync_all());
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn load(
        &self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        self.read_snapshot(campaign_id).map(|(state, _)| state)
    }

    pub(crate) fn extend_cap(
        &mut self,
        request: &T1ScreenCampaignCapExtensionRequest,
        timestamp: Timestamp,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        validate_identifier(&request.campaign_id.0, "T1 screening campaign")?;
        validate_t1_timestamp(&timestamp)?;
        if request.new_approved_total_millionths_of_dollar == 0 {
            return Err(invalid(
                "T1 screening campaign extension total must be positive",
            ));
        }
        if request.owner_reason.trim().is_empty() {
            return Err(invalid(
                "T1 screening campaign extension owner reason is blank",
            ));
        }
        self.update(&request.campaign_id, |stored, store| {
            if stored.active_run_id.is_some() {
                return Err(invalid(
                    "T1 screening campaign extension requires no active run",
                ));
            }
            if !matches!(
                stored.status,
                T1ScreenCampaignStatus::Paused | T1ScreenCampaignStatus::Exhausted
            ) {
                return Err(invalid(
                    "T1 screening campaign extension requires a paused or exhausted campaign",
                ));
            }
            if request.new_approved_total_millionths_of_dollar
                <= stored.approved_judge_total_millionths_of_dollar
            {
                return Err(invalid(
                    "T1 screening campaign extension total must strictly increase",
                ));
            }
            for entry in &stored.runs {
                let observed = store.import_run(&stored.campaign_id, &entry.run_id)?;
                let observed = apply_persisted_retirement(stored, observed)?;
                if *entry != observed {
                    return Err(invalid(format!(
                        "T1 screening campaign run {:?} state bytes or audit fields changed",
                        entry.run_id.0
                    )));
                }
            }
            let mut next = stored.clone();
            next.cap_extensions.push(T1ScreenCampaignCapExtension {
                timestamp,
                previous_approved_total_millionths_of_dollar: stored
                    .approved_judge_total_millionths_of_dollar,
                new_approved_total_millionths_of_dollar: request
                    .new_approved_total_millionths_of_dollar,
                owner_reason: request.owner_reason.clone(),
            });
            next.approved_judge_total_millionths_of_dollar =
                request.new_approved_total_millionths_of_dollar;
            next.status = T1ScreenCampaignStatus::Open;
            Ok(next)
        })
    }

    pub(crate) fn retire_run(
        &mut self,
        request: &T1ScreenCampaignRunRetirementRequest,
        timestamp: Timestamp,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        validate_identifier(&request.campaign_id.0, "T1 screening campaign")?;
        validate_identifier(&request.run_id.0, "T1 screening run")?;
        validate_t1_timestamp(&timestamp)?;
        if request.owner_reason.trim().is_empty() {
            return Err(invalid(
                "T1 screening campaign retirement owner reason is blank",
            ));
        }
        self.update(&request.campaign_id, |stored, store| {
            if stored.status != T1ScreenCampaignStatus::Paused {
                return Err(invalid(
                    "T1 screening campaign retirement requires a paused campaign",
                ));
            }
            if stored.active_run_id.as_ref() != Some(&request.run_id) {
                return Err(invalid(
                    "T1 screening campaign retirement run is not the exact active run",
                ));
            }
            if stored
                .retirements
                .iter()
                .any(|retirement| retirement.run_id == request.run_id)
            {
                return Err(invalid("T1 screening campaign run is already retired"));
            }
            let index = stored
                .runs
                .iter()
                .position(|entry| entry.run_id == request.run_id)
                .ok_or_else(|| invalid("T1 screening campaign active run is not registered"))?;
            let entry = &stored.runs[index];
            if !entry.is_resumable || entry.superseded_reason.is_some() {
                return Err(invalid(
                    "T1 screening campaign retirement run is not resumable",
                ));
            }
            let observed = store.import_run(&stored.campaign_id, &request.run_id)?;
            if observed != *entry {
                return Err(invalid(format!(
                    "T1 screening campaign run {:?} state bytes or audit fields changed",
                    request.run_id.0
                )));
            }
            if observed.observed_status != T1ScreenRunStatus::Paused {
                return Err(invalid(
                    "T1 screening campaign retirement requires a paused run state",
                ));
            }
            let mut next = stored.clone();
            next.retirements.push(T1ScreenCampaignRunRetirement {
                timestamp,
                run_id: request.run_id.clone(),
                owner_reason: request.owner_reason.clone(),
            });
            next.runs[index].is_resumable = false;
            next.runs[index].superseded_reason = Some(request.owner_reason.clone());
            next.active_run_id = None;
            next.status = T1ScreenCampaignStatus::Open;
            Ok(next)
        })
    }

    pub(crate) fn save(&mut self, state: &T1ScreenCampaignState) -> Result<(), SkillEvalError> {
        validate_campaign_state(state)?;
        self.validate_store_paths(state)?;
        let directory = self.existing_campaign_directory(&state.campaign_id)?;
        let _lock = CampaignLock::acquire(&directory)?;
        let (stored, prior_bytes) = self.read_snapshot(&state.campaign_id)?;
        validate_campaign_transition(&stored, state)?;
        self.replace_snapshot(
            &directory,
            &directory.join(SNAPSHOT_NAME),
            state,
            Some(&prior_bytes),
        )
    }

    pub(crate) fn reconcile(
        &mut self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        self.update(campaign_id, |stored, store| {
            let active = stored.active_run_id.clone();
            let mut next = stored.clone();
            for entry in &mut next.runs {
                let observed = store.import_run(campaign_id, &entry.run_id)?;
                let observed = apply_persisted_retirement(stored, observed)?;
                if active.as_ref() == Some(&entry.run_id) {
                    if !entry.is_resumable || observed.superseded_reason.is_some() {
                        return Err(invalid("T1 screening campaign active run is not resumable"));
                    }
                    if observed.judge_spend_millionths_of_dollar
                        < entry.judge_spend_millionths_of_dollar
                    {
                        return Err(invalid("T1 screening campaign active run spend decreased"));
                    }
                    observed_identity_matches(entry, &observed)?;
                    *entry = observed;
                } else if *entry != observed {
                    return Err(invalid(format!(
                        "T1 screening campaign run {:?} state bytes or audit fields changed",
                        entry.run_id.0
                    )));
                }
            }
            refresh_campaign_aggregate_and_status(&mut next, None)?;
            Ok(next)
        })
    }

    pub(crate) fn pause_for_budget(
        &mut self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        self.update(campaign_id, |stored, _store| {
            let mut next = stored.clone();
            next.status = if next.aggregate_judge_spent_millionths_of_dollar
                == next.approved_judge_total_millionths_of_dollar
            {
                next.active_run_id = None;
                T1ScreenCampaignStatus::Exhausted
            } else {
                T1ScreenCampaignStatus::Paused
            };
            Ok(next)
        })
    }

    pub(crate) fn register_active_run(
        &mut self,
        state: &T1ScreenRunState,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        let campaign_id = &state.configuration.campaign_id;
        self.update(campaign_id, |stored, store| {
            let entry = store.import_run(campaign_id, &state.configuration.run_id)?;
            if !entry.is_resumable {
                return Err(invalid(
                    "T1 screening campaign cannot register a non-resumable run",
                ));
            }
            let mut next = stored.clone();
            if let Some(index) = next
                .runs
                .iter()
                .position(|stored_entry| stored_entry.run_id == entry.run_id)
            {
                if next.active_run_id.as_ref() != Some(&entry.run_id) {
                    return Err(invalid(
                        "T1 screening campaign retry names an inactive registered run",
                    ));
                }
                observed_identity_matches(&next.runs[index], &entry)?;
                if next.runs[index].judge_spend_millionths_of_dollar
                    > entry.judge_spend_millionths_of_dollar
                {
                    return Err(invalid(
                        "T1 screening campaign registered run spend decreased",
                    ));
                }
                next.runs[index] = entry;
            } else {
                if stored.active_run_id.is_some() {
                    return Err(invalid("T1 screening campaign already has an active run"));
                }
                if stored.status != T1ScreenCampaignStatus::Open {
                    return Err(invalid("T1 screening campaign is not open"));
                }
                next.runs.push(entry.clone());
                next.active_run_id = Some(entry.run_id);
            }
            refresh_campaign_aggregate_and_status(&mut next, Some(state))?;
            Ok(next)
        })
    }

    pub(crate) fn reconcile_active_run(
        &mut self,
        state: &T1ScreenRunState,
    ) -> Result<T1ScreenCampaignState, SkillEvalError> {
        let campaign_id = &state.configuration.campaign_id;
        self.update(campaign_id, |stored, store| {
            if stored.active_run_id.as_ref() != Some(&state.configuration.run_id) {
                return Err(invalid(
                    "T1 screening campaign active run differs from the parent state",
                ));
            }
            let entry = store.import_run(campaign_id, &state.configuration.run_id)?;
            let mut next = stored.clone();
            let index = next
                .runs
                .iter()
                .position(|stored_entry| stored_entry.run_id == entry.run_id)
                .ok_or_else(|| invalid("T1 screening campaign active run is not registered"))?;
            observed_identity_matches(&next.runs[index], &entry)?;
            if next.runs[index].judge_spend_millionths_of_dollar
                > entry.judge_spend_millionths_of_dollar
            {
                return Err(invalid("T1 screening campaign active run spend decreased"));
            }
            next.runs[index] = entry;
            refresh_campaign_aggregate_and_status(&mut next, Some(state))?;
            Ok(next)
        })
    }

    fn update<F>(
        &mut self,
        campaign_id: &T1ScreenCampaignId,
        operation: F,
    ) -> Result<T1ScreenCampaignState, SkillEvalError>
    where
        F: FnOnce(&T1ScreenCampaignState, &Self) -> Result<T1ScreenCampaignState, SkillEvalError>,
    {
        let directory = self.existing_campaign_directory(campaign_id)?;
        let _lock = CampaignLock::acquire(&directory)?;
        let (stored, prior_bytes) = self.read_snapshot(campaign_id)?;
        let next = operation(&stored, self)?;
        validate_campaign_state(&next)?;
        validate_campaign_transition(&stored, &next)?;
        if next != stored {
            self.replace_snapshot(
                &directory,
                &directory.join(SNAPSHOT_NAME),
                &next,
                Some(&prior_bytes),
            )?;
        }
        Ok(next)
    }

    fn available_run_ids(&self) -> Result<BTreeSet<String>, SkillEvalError> {
        let mut identifiers = BTreeSet::new();
        for item in fs::read_dir(&self.screening_root)
            .map_err(|error| io_error(&self.screening_root, error))?
        {
            let item = item.map_err(|error| io_error(&self.screening_root, error))?;
            let file_type = item
                .file_type()
                .map_err(|error| io_error(&item.path(), error))?;
            if !file_type.is_dir() {
                continue;
            }
            let identifier = item
                .file_name()
                .into_string()
                .map_err(|_| invalid("T1 screening run directory is not valid UTF-8"))?;
            validate_identifier(&identifier, "T1 screening run")?;
            if item.path().join(SNAPSHOT_NAME).is_file() {
                identifiers.insert(identifier);
            }
        }
        Ok(identifiers)
    }

    fn import_run(
        &self,
        campaign_id: &T1ScreenCampaignId,
        run_id: &T1ScreenRunId,
    ) -> Result<T1ScreenCampaignRunEntry, SkillEvalError> {
        let expected = self.screening_root.join(&run_id.0).join(SNAPSHOT_NAME);
        let canonical = fs::canonicalize(&expected).map_err(|error| io_error(&expected, error))?;
        let metadata =
            fs::symlink_metadata(&expected).map_err(|error| io_error(&expected, error))?;
        if canonical != expected
            || !canonical.starts_with(&self.screening_root)
            || !metadata.file_type().is_file()
        {
            return Err(invalid(format!(
                "T1 screening run {:?} state path escapes the screening root",
                run_id.0
            )));
        }
        let bytes = fs::read(&canonical).map_err(|error| io_error(&canonical, error))?;
        raw_run_entry(campaign_id, run_id, canonical, &bytes)
    }

    fn campaign_directory(
        &self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<PathBuf, SkillEvalError> {
        validate_identifier(&campaign_id.0, "T1 screening campaign")?;
        Ok(self.campaigns_root.join(&campaign_id.0))
    }

    fn existing_campaign_directory(
        &self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<PathBuf, SkillEvalError> {
        let expected = self.campaign_directory(campaign_id)?;
        let canonical = fs::canonicalize(&expected).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SkillEvalError::NotFound(format!(
                    "T1 screening campaign {:?} does not exist",
                    campaign_id.0
                ))
            } else {
                io_error(&expected, error)
            }
        })?;
        if canonical != expected || !canonical.is_dir() {
            return Err(invalid(format!(
                "T1 screening campaign {:?} escapes the campaign root",
                campaign_id.0
            )));
        }
        Ok(canonical)
    }

    fn read_snapshot(
        &self,
        campaign_id: &T1ScreenCampaignId,
    ) -> Result<(T1ScreenCampaignState, Vec<u8>), SkillEvalError> {
        let path = self
            .existing_campaign_directory(campaign_id)?
            .join(SNAPSHOT_NAME);
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if !metadata.file_type().is_file() {
            return Err(invalid(format!(
                "T1 screening campaign snapshot {} is not a regular file",
                path.display()
            )));
        }
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let state: T1ScreenCampaignState = serde_json::from_slice(&bytes).map_err(|error| {
            invalid(format!(
                "T1 screening campaign snapshot {} is malformed: {error}",
                path.display()
            ))
        })?;
        if state.campaign_id != *campaign_id {
            return Err(invalid(
                "T1 screening campaign snapshot identity differs from its path",
            ));
        }
        validate_campaign_state(&state)?;
        self.validate_store_paths(&state)?;
        Ok((state, bytes))
    }

    fn validate_store_paths(&self, state: &T1ScreenCampaignState) -> Result<(), SkillEvalError> {
        if !self.campaigns_root.starts_with(&self.repository_root)
            || !self.screening_root.starts_with(&self.repository_root)
        {
            return Err(invalid(
                "T1 screening campaign store escaped the repository root",
            ));
        }
        for run in &state.runs {
            let expected = self.screening_root.join(&run.run_id.0).join(SNAPSHOT_NAME);
            if run.canonical_state_path != expected {
                return Err(invalid(
                    "T1 screening campaign run path escapes the screening root",
                ));
            }
        }
        Ok(())
    }

    fn write_temporary(
        &mut self,
        directory: &Path,
        state: &T1ScreenCampaignState,
    ) -> Result<PathBuf, SkillEvalError> {
        let bytes = serde_json::to_vec_pretty(state).map_err(|error| {
            invalid(format!(
                "T1 screening campaign serialization failed: {error}"
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
            self.fail(T1ScreenCampaignFailurePoint::Write, &path)?;
            file.write_all(&bytes)
                .and_then(|()| file.write_all(b"\n"))
                .map_err(|error| io_error(&path, error))?;
            self.fail(T1ScreenCampaignFailurePoint::FileSync, &path)?;
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
        state: &T1ScreenCampaignState,
        prior_bytes: Option<&[u8]>,
    ) -> Result<(), SkillEvalError> {
        let temporary = self.write_temporary(directory, state)?;
        let mut is_replaced = false;
        let result = (|| {
            self.fail(T1ScreenCampaignFailurePoint::Rename, snapshot)?;
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
        self.fail(T1ScreenCampaignFailurePoint::DirectorySync, directory)?;
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|error| io_error(directory, error))
    }

    #[cfg(test)]
    fn fail(
        &mut self,
        point: T1ScreenCampaignFailurePoint,
        path: &Path,
    ) -> Result<(), SkillEvalError> {
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
    fn fail(
        &mut self,
        _point: T1ScreenCampaignFailurePoint,
        _path: &Path,
    ) -> Result<(), SkillEvalError> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum T1ScreenCampaignFailurePoint {
    Write,
    FileSync,
    Rename,
    DirectorySync,
}

struct CampaignLock {
    path: PathBuf,
}

impl CampaignLock {
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
                    invalid("T1 screening campaign has a concurrent writer")
                } else {
                    io_error(&path, error)
                }
            })?;
        Ok(Self { path })
    }
}

impl Drop for CampaignLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(directory) = self.path.parent() {
            let _ = File::open(directory).and_then(|file| file.sync_all());
        }
    }
}

fn raw_run_entry(
    campaign_id: &T1ScreenCampaignId,
    expected_run_id: &T1ScreenRunId,
    canonical_state_path: PathBuf,
    bytes: &[u8],
) -> Result<T1ScreenCampaignRunEntry, SkillEvalError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| {
        invalid(format!(
            "T1 screening import snapshot is malformed: {error}"
        ))
    })?;
    let root = value
        .as_object()
        .ok_or_else(|| invalid("T1 screening import snapshot root is not an object"))?;
    let configuration = root
        .get("configuration")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| invalid("T1 screening import configuration is missing"))?;
    let run_id = required_string(configuration, "run_id", "run identifier")?;
    validate_identifier(run_id, "T1 screening run")?;
    if run_id != expected_run_id.0 {
        return Err(invalid(
            "T1 screening import run identity differs from its path",
        ));
    }
    let created_at =
        Timestamp(required_string(configuration, "created_at", "creation time")?.to_owned());
    validate_t1_timestamp(&created_at)?;
    let observed_status: T1ScreenRunStatus = serde_json::from_value(
        root.get("status")
            .cloned()
            .ok_or_else(|| invalid("T1 screening import status is missing"))?,
    )
    .map_err(|error| invalid(format!("T1 screening import status is invalid: {error}")))?;
    let judge_spend_millionths_of_dollar =
        required_u64(root, "spent_judge_millionths_of_dollar", "judge spend")?;
    let candidate_usage = root
        .get("candidate_usage")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| invalid("T1 screening import candidate usage is missing"))?;
    let candidate_cost_millionths_of_dollar = required_u64(
        candidate_usage,
        "cost_millionths_of_dollar",
        "candidate cost",
    )?;
    if candidate_cost_millionths_of_dollar != 0 {
        return Err(invalid(
            "T1 screening campaign import candidate cost must be exactly zero",
        ));
    }
    let manifest = configuration
        .get("candidate_environment")
        .and_then(serde_json::Value::as_object)
        .and_then(|environment| environment.get("manifest"))
        .and_then(serde_json::Value::as_array);
    let stored_campaign = configuration
        .get("campaign_id")
        .and_then(serde_json::Value::as_str);
    let (is_resumable, superseded_reason) = if manifest.is_none_or(Vec::is_empty) {
        (
            false,
            Some("legacy candidate environment schema cannot resume".to_owned()),
        )
    } else if stored_campaign != Some(campaign_id.0.as_str()) {
        (
            false,
            Some("state has no matching campaign identity".to_owned()),
        )
    } else {
        (true, None)
    };
    Ok(T1ScreenCampaignRunEntry {
        run_id: expected_run_id.clone(),
        canonical_state_path,
        state_file_sha256: hex_digest(bytes),
        created_at,
        observed_status,
        judge_spend_millionths_of_dollar,
        candidate_cost_millionths_of_dollar,
        is_resumable,
        superseded_reason,
    })
}

pub(crate) fn validate_campaign_state(state: &T1ScreenCampaignState) -> Result<(), SkillEvalError> {
    validate_identifier(&state.campaign_id.0, "T1 screening campaign")?;
    validate_t1_timestamp(&state.created_at)?;
    let mut approved_total = T1_SCREEN_CAMPAIGN_APPROVED_TOTAL;
    let mut previous_timestamp = state.created_at.0.as_str();
    let mut history_timestamps = BTreeSet::new();
    for extension in &state.cap_extensions {
        validate_t1_timestamp(&extension.timestamp)?;
        if extension.timestamp.0.as_str() <= previous_timestamp {
            return Err(invalid(
                "T1 screening campaign extension timestamps are not strictly increasing",
            ));
        }
        if !history_timestamps.insert(extension.timestamp.0.clone()) {
            return Err(invalid(
                "T1 screening campaign history timestamps must be unique",
            ));
        }
        previous_timestamp = extension.timestamp.0.as_str();
        if extension.owner_reason.trim().is_empty() {
            return Err(invalid(
                "T1 screening campaign extension owner reason is blank",
            ));
        }
        if extension.previous_approved_total_millionths_of_dollar != approved_total {
            return Err(invalid(
                "T1 screening campaign extension does not name the prior approved total",
            ));
        }
        if extension.new_approved_total_millionths_of_dollar <= approved_total {
            return Err(invalid(
                "T1 screening campaign extension total must strictly increase",
            ));
        }
        approved_total = extension.new_approved_total_millionths_of_dollar;
    }
    if state.approved_judge_total_millionths_of_dollar != approved_total {
        return Err(invalid(
            "T1 screening campaign approved total differs from its complete extension chain",
        ));
    }
    if state.owner_reason.trim().is_empty() {
        return Err(invalid("T1 screening campaign owner reason is blank"));
    }
    let mut retirement_run_ids = BTreeSet::new();
    let mut previous_retirement_timestamp = state.created_at.0.as_str();
    for retirement in &state.retirements {
        validate_t1_timestamp(&retirement.timestamp)?;
        if retirement.timestamp.0.as_str() <= previous_retirement_timestamp {
            return Err(invalid(
                "T1 screening campaign retirement timestamps are not strictly increasing",
            ));
        }
        if !history_timestamps.insert(retirement.timestamp.0.clone()) {
            return Err(invalid(
                "T1 screening campaign history timestamps must be unique",
            ));
        }
        previous_retirement_timestamp = retirement.timestamp.0.as_str();
        validate_identifier(&retirement.run_id.0, "T1 screening retired run")?;
        if !retirement_run_ids.insert(retirement.run_id.0.clone()) {
            return Err(invalid(
                "T1 screening campaign contains a duplicate retirement",
            ));
        }
        if retirement.owner_reason.trim().is_empty() {
            return Err(invalid(
                "T1 screening campaign retirement owner reason is blank",
            ));
        }
    }
    let mut identifiers = BTreeSet::new();
    let mut previous = None::<(&str, &str)>;
    for run in &state.runs {
        validate_identifier(&run.run_id.0, "T1 screening campaign run")?;
        validate_digest(&run.state_file_sha256, "T1 screening campaign state file")?;
        validate_t1_timestamp(&run.created_at)?;
        if !run.canonical_state_path.is_absolute()
            || !run.canonical_state_path.starts_with(&state_root_for_path(
                &run.canonical_state_path,
                &run.run_id,
            )?)
        {
            return Err(invalid(
                "T1 screening campaign run state path is not canonical",
            ));
        }
        if run.candidate_cost_millionths_of_dollar != 0 {
            return Err(invalid(
                "T1 screening campaign run candidate cost must be zero",
            ));
        }
        if run.is_resumable == run.superseded_reason.is_some()
            || run
                .superseded_reason
                .as_ref()
                .is_some_and(|reason| reason.trim().is_empty())
        {
            return Err(invalid(
                "T1 screening campaign resumability metadata is inconsistent",
            ));
        }
        if !identifiers.insert(&run.run_id.0) {
            return Err(invalid(
                "T1 screening campaign contains a duplicate run identifier",
            ));
        }
        let order = (run.created_at.0.as_str(), run.run_id.0.as_str());
        if previous.is_some_and(|prior| prior >= order) {
            return Err(invalid(
                "T1 screening campaign runs are outside creation-time and identifier order",
            ));
        }
        previous = Some(order);
    }
    for retirement in &state.retirements {
        let mut matching = state
            .runs
            .iter()
            .filter(|run| run.run_id == retirement.run_id);
        let run = matching.next().ok_or_else(|| {
            invalid("T1 screening campaign retirement does not identify a stored run")
        })?;
        if matching.next().is_some() {
            return Err(invalid(
                "T1 screening campaign retirement identifies multiple stored runs",
            ));
        }
        if run.is_resumable
            || run.superseded_reason.as_deref() != Some(retirement.owner_reason.as_str())
        {
            return Err(invalid(
                "T1 screening campaign retirement metadata differs from its run entry",
            ));
        }
        if state.active_run_id.as_ref() == Some(&retirement.run_id) {
            return Err(invalid(
                "T1 screening campaign retirement names the active run",
            ));
        }
    }
    let aggregate = sum_run_spend(&state.runs)?;
    if aggregate != state.aggregate_judge_spent_millionths_of_dollar {
        return Err(invalid(
            "T1 screening campaign aggregate spend is not the exact run sum",
        ));
    }
    if aggregate > state.approved_judge_total_millionths_of_dollar {
        return Err(invalid(
            "T1 screening campaign aggregate spend exceeds its approved total",
        ));
    }
    if let Some(active) = &state.active_run_id {
        let run = state
            .runs
            .iter()
            .find(|run| &run.run_id == active)
            .ok_or_else(|| invalid("T1 screening campaign active run is not registered"))?;
        if !run.is_resumable {
            return Err(invalid("T1 screening campaign active run is not resumable"));
        }
        if !matches!(
            state.status,
            T1ScreenCampaignStatus::Open | T1ScreenCampaignStatus::Paused
        ) {
            return Err(invalid(
                "T1 screening campaign terminal status has an active run",
            ));
        }
    }
    if state.status == T1ScreenCampaignStatus::Exhausted
        && aggregate != state.approved_judge_total_millionths_of_dollar
    {
        return Err(invalid(
            "T1 screening campaign exhausted status differs from spend",
        ));
    }
    Ok(())
}

pub(crate) fn validate_campaign_transition(
    stored: &T1ScreenCampaignState,
    next: &T1ScreenCampaignState,
) -> Result<(), SkillEvalError> {
    validate_campaign_state(stored)?;
    validate_campaign_state(next)?;
    if stored.campaign_id != next.campaign_id
        || stored.created_at != next.created_at
        || stored.owner_reason != next.owner_reason
    {
        return Err(invalid(
            "T1 screening campaign identity or initial owner reason changed",
        ));
    }
    let is_extension = next.cap_extensions != stored.cap_extensions;
    let is_retirement = next.retirements != stored.retirements;
    if is_extension && is_retirement {
        return Err(invalid(
            "T1 screening campaign cannot append an extension and retirement together",
        ));
    }
    if is_extension {
        validate_campaign_extension_transition(stored, next)?;
    } else if next.approved_judge_total_millionths_of_dollar
        != stored.approved_judge_total_millionths_of_dollar
    {
        return Err(invalid(
            "T1 screening campaign approved total changed without an extension",
        ));
    }
    if is_retirement {
        validate_campaign_retirement_transition(stored, next)?;
    }
    if next.runs.len() < stored.runs.len() || next.runs.len() > stored.runs.len().saturating_add(1)
    {
        return Err(invalid(
            "T1 screening campaign run entries are not one exact append or reconciliation",
        ));
    }
    let retired_run_id = is_retirement
        .then(|| next.retirements.last().map(|retirement| &retirement.run_id))
        .flatten();
    for (old, new) in stored.runs.iter().zip(&next.runs) {
        if retired_run_id == Some(&old.run_id) {
            continue;
        }
        observed_identity_matches(old, new)?;
        if stored.active_run_id.as_ref() != Some(&old.run_id) && old != new {
            return Err(invalid(
                "T1 screening campaign inactive run entry was rewritten",
            ));
        }
        if new.judge_spend_millionths_of_dollar < old.judge_spend_millionths_of_dollar {
            return Err(invalid("T1 screening campaign run spend cannot decrease"));
        }
    }
    if next.runs.len() == stored.runs.len().saturating_add(1)
        && (stored.active_run_id.is_some()
            || next.runs[..stored.runs.len()] != stored.runs
            || next.active_run_id.as_ref() != next.runs.last().map(|run| &run.run_id))
    {
        return Err(invalid(
            "T1 screening campaign append did not register one new active run",
        ));
    }
    if next.aggregate_judge_spent_millionths_of_dollar
        < stored.aggregate_judge_spent_millionths_of_dollar
    {
        return Err(invalid(
            "T1 screening campaign aggregate spend cannot decrease",
        ));
    }
    Ok(())
}

fn validate_campaign_extension_transition(
    stored: &T1ScreenCampaignState,
    next: &T1ScreenCampaignState,
) -> Result<(), SkillEvalError> {
    let expected_length = stored
        .cap_extensions
        .len()
        .checked_add(1)
        .ok_or_else(|| invalid("T1 screening campaign extension history overflowed"))?;
    if next.cap_extensions.len() != expected_length
        || !next.cap_extensions.starts_with(&stored.cap_extensions)
    {
        return Err(invalid(
            "T1 screening campaign extension history is not one exact append",
        ));
    }
    let extension = next
        .cap_extensions
        .last()
        .expect("one appended extension exists");
    if extension.previous_approved_total_millionths_of_dollar
        != stored.approved_judge_total_millionths_of_dollar
        || extension.new_approved_total_millionths_of_dollar
            != next.approved_judge_total_millionths_of_dollar
        || extension.new_approved_total_millionths_of_dollar
            <= extension.previous_approved_total_millionths_of_dollar
    {
        return Err(invalid(
            "T1 screening campaign extension does not connect the approved totals",
        ));
    }
    if stored
        .retirements
        .iter()
        .any(|retirement| retirement.timestamp.0 >= extension.timestamp.0)
    {
        return Err(invalid(
            "T1 screening campaign extension timestamp is not later than every retirement",
        ));
    }
    if !matches!(
        stored.status,
        T1ScreenCampaignStatus::Paused | T1ScreenCampaignStatus::Exhausted
    ) || stored.active_run_id.is_some()
        || next.status != T1ScreenCampaignStatus::Open
        || next.active_run_id.is_some()
        || next.runs != stored.runs
        || next.retirements != stored.retirements
        || next.aggregate_judge_spent_millionths_of_dollar
            != stored.aggregate_judge_spent_millionths_of_dollar
    {
        return Err(invalid(
            "T1 screening campaign extension changed state outside one authority append",
        ));
    }
    Ok(())
}

fn validate_campaign_retirement_transition(
    stored: &T1ScreenCampaignState,
    next: &T1ScreenCampaignState,
) -> Result<(), SkillEvalError> {
    let expected_length = stored
        .retirements
        .len()
        .checked_add(1)
        .ok_or_else(|| invalid("T1 screening campaign retirement history overflowed"))?;
    if next.retirements.len() != expected_length
        || !next.retirements.starts_with(&stored.retirements)
    {
        return Err(invalid(
            "T1 screening campaign retirement history is not one exact append",
        ));
    }
    let retirement = next
        .retirements
        .last()
        .expect("one appended retirement exists");
    if stored
        .cap_extensions
        .iter()
        .any(|extension| extension.timestamp.0 >= retirement.timestamp.0)
        || stored
            .retirements
            .iter()
            .any(|prior| prior.timestamp.0 >= retirement.timestamp.0)
    {
        return Err(invalid(
            "T1 screening campaign retirement timestamp is not later than campaign history",
        ));
    }
    let index = stored
        .runs
        .iter()
        .position(|run| run.run_id == retirement.run_id)
        .ok_or_else(|| invalid("T1 screening campaign retirement run is not registered"))?;
    let old = &stored.runs[index];
    let mut expected = old.clone();
    expected.is_resumable = false;
    expected.superseded_reason = Some(retirement.owner_reason.clone());
    if stored.status != T1ScreenCampaignStatus::Paused
        || stored.active_run_id.as_ref() != Some(&retirement.run_id)
        || old.observed_status != T1ScreenRunStatus::Paused
        || !old.is_resumable
        || old.superseded_reason.is_some()
        || next.status != T1ScreenCampaignStatus::Open
        || next.active_run_id.is_some()
        || next.cap_extensions != stored.cap_extensions
        || next.approved_judge_total_millionths_of_dollar
            != stored.approved_judge_total_millionths_of_dollar
        || next.aggregate_judge_spent_millionths_of_dollar
            != stored.aggregate_judge_spent_millionths_of_dollar
        || next.runs.len() != stored.runs.len()
        || next.runs[index] != expected
        || next
            .runs
            .iter()
            .enumerate()
            .any(|(position, run)| position != index && run != &stored.runs[position])
    {
        return Err(invalid(
            "T1 screening campaign retirement changed state outside one paused run",
        ));
    }
    Ok(())
}

fn refresh_campaign_aggregate_and_status(
    campaign: &mut T1ScreenCampaignState,
    parent: Option<&T1ScreenRunState>,
) -> Result<(), SkillEvalError> {
    campaign.aggregate_judge_spent_millionths_of_dollar = sum_run_spend(&campaign.runs)?;
    if campaign.aggregate_judge_spent_millionths_of_dollar
        > campaign.approved_judge_total_millionths_of_dollar
    {
        return Err(invalid(
            "T1 screening campaign aggregate spend exceeds the approved total",
        ));
    }
    let Some(parent) = parent else {
        return Ok(());
    };
    match parent.status {
        T1ScreenRunStatus::Pending | T1ScreenRunStatus::Running => {
            campaign.active_run_id = Some(parent.configuration.run_id.clone());
            campaign.status = T1ScreenCampaignStatus::Open;
        }
        T1ScreenRunStatus::Paused => match parent.pause {
            Some(T1ScreenPauseReason::JudgeCap { .. }) => {
                if campaign.aggregate_judge_spent_millionths_of_dollar
                    == campaign.approved_judge_total_millionths_of_dollar
                {
                    campaign.active_run_id = None;
                    campaign.status = T1ScreenCampaignStatus::Exhausted;
                } else {
                    campaign.active_run_id = Some(parent.configuration.run_id.clone());
                    campaign.status = T1ScreenCampaignStatus::Paused;
                }
            }
            Some(T1ScreenPauseReason::Infrastructure { .. })
            | Some(T1ScreenPauseReason::Quota { .. }) => {
                campaign.active_run_id = Some(parent.configuration.run_id.clone());
                campaign.status = T1ScreenCampaignStatus::Paused;
            }
            None => return Err(invalid("T1 screening paused run has no pause reason")),
        },
        T1ScreenRunStatus::AwaitingOwner => {
            campaign.active_run_id = None;
            campaign.status = T1ScreenCampaignStatus::AwaitingOwner;
        }
        T1ScreenRunStatus::Completed => {
            campaign.active_run_id = None;
            campaign.status = T1ScreenCampaignStatus::Closed;
        }
        T1ScreenRunStatus::Failed => {
            campaign.active_run_id = None;
            campaign.status = T1ScreenCampaignStatus::Open;
        }
    }
    Ok(())
}

fn apply_persisted_retirement(
    campaign: &T1ScreenCampaignState,
    mut observed: T1ScreenCampaignRunEntry,
) -> Result<T1ScreenCampaignRunEntry, SkillEvalError> {
    if let Some(retirement) = campaign
        .retirements
        .iter()
        .find(|retirement| retirement.run_id == observed.run_id)
    {
        if !observed.is_resumable || observed.superseded_reason.is_some() {
            return Err(invalid(
                "T1 screening campaign retired run raw resumability changed",
            ));
        }
        observed.is_resumable = false;
        observed.superseded_reason = Some(retirement.owner_reason.clone());
    }
    Ok(observed)
}

fn observed_identity_matches(
    stored: &T1ScreenCampaignRunEntry,
    next: &T1ScreenCampaignRunEntry,
) -> Result<(), SkillEvalError> {
    if stored.run_id != next.run_id
        || stored.canonical_state_path != next.canonical_state_path
        || stored.created_at != next.created_at
        || stored.is_resumable != next.is_resumable
        || stored.superseded_reason != next.superseded_reason
        || stored.candidate_cost_millionths_of_dollar != next.candidate_cost_millionths_of_dollar
    {
        return Err(invalid(
            "T1 screening campaign run identity or immutable audit metadata changed",
        ));
    }
    Ok(())
}

fn state_root_for_path(path: &Path, run_id: &T1ScreenRunId) -> Result<PathBuf, SkillEvalError> {
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("T1 screening campaign run state path has no file name"))?;
    let run = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("T1 screening campaign run state path has no run directory"))?;
    if file != SNAPSHOT_NAME || run != run_id.0 {
        return Err(invalid(
            "T1 screening campaign run state path differs from its run identity",
        ));
    }
    path.parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| invalid("T1 screening campaign run state path is incomplete"))
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
    label: &str,
) -> Result<&'a str, SkillEvalError> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid(format!("T1 screening import {label} is missing or blank")))
}

fn required_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    label: &str,
) -> Result<u64, SkillEvalError> {
    object
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| invalid(format!("T1 screening import {label} is missing or invalid")))
}

fn sum_run_spend(runs: &[T1ScreenCampaignRunEntry]) -> Result<u64, SkillEvalError> {
    runs.iter().try_fold(0_u64, |sum, run| {
        sum.checked_add(run.judge_spend_millionths_of_dollar)
            .ok_or_else(|| invalid("T1 screening campaign aggregate spend overflowed"))
    })
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn restore_snapshot(directory: &Path, snapshot: &Path, bytes: &[u8]) -> Result<(), SkillEvalError> {
    let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let restore = directory.join(format!(
        ".{SNAPSHOT_NAME}.restore.{}.{sequence}.tmp",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&restore)
        .map_err(|error| io_error(&restore, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(&restore, error))?;
    fs::rename(&restore, snapshot).map_err(|error| io_error(snapshot, error))?;
    File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error(directory, error))
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

fn create_contained_directory(
    repository_root: &Path,
    components: &[&str],
) -> Result<PathBuf, SkillEvalError> {
    let mut current = repository_root.to_path_buf();
    for component in components {
        current.push(component);
        match fs::create_dir(&current) {
            Ok(()) => {
                File::open(current.parent().expect("created path has a parent"))
                    .and_then(|directory| directory.sync_all())
                    .map_err(|error| io_error(&current, error))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error(&current, error)),
        }
        let canonical = fs::canonicalize(&current).map_err(|error| io_error(&current, error))?;
        if canonical != current || !canonical.starts_with(repository_root) || !canonical.is_dir() {
            return Err(invalid(format!(
                "campaign store path {} escapes the repository root",
                current.display()
            )));
        }
    }
    Ok(current)
}

fn existing_contained_directory(
    repository_root: &Path,
    components: &[&str],
) -> Result<PathBuf, SkillEvalError> {
    let expected = components
        .iter()
        .fold(repository_root.to_path_buf(), |mut path, component| {
            path.push(component);
            path
        });
    let canonical = fs::canonicalize(&expected).map_err(|error| io_error(&expected, error))?;
    if canonical != expected || !canonical.starts_with(repository_root) || !canonical.is_dir() {
        return Err(invalid(format!(
            "campaign store path {} escapes the repository root",
            expected.display()
        )));
    }
    Ok(canonical)
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
