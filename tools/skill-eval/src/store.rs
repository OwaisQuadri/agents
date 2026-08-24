use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};

use crate::model::{
    CandidateArtifact, HarnessIdentity, ModelIdentity, RunEvent, RunId, SkillEvalError, TrialKey,
    TrialRecord, TrialSelector,
};
use crate::ports::RunStore;

const EVENT_LOG_NAME: &str = "events.jsonl";

pub(crate) struct FileRunStore {
    root: PathBuf,
}

impl FileRunStore {
    pub(crate) fn new(root: impl AsRef<Path>) -> Result<Self, SkillEvalError> {
        let root = root.as_ref();
        fs::create_dir_all(root).map_err(|error| io_error(root, error))?;
        let root = fs::canonicalize(root).map_err(|error| io_error(root, error))?;
        Ok(Self { root })
    }

    fn run_directory(&self, run_id: &RunId) -> Result<PathBuf, SkillEvalError> {
        validate_run_id(run_id)?;
        Ok(self.root.join(&run_id.0))
    }

    fn log_path(&self, run_id: &RunId) -> Result<PathBuf, SkillEvalError> {
        Ok(self.run_directory(run_id)?.join(EVENT_LOG_NAME))
    }

    fn existing_log_path(&self, run_id: &RunId) -> Result<PathBuf, SkillEvalError> {
        let run_directory = self.run_directory(run_id)?;
        let canonical_directory = fs::canonicalize(&run_directory).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SkillEvalError::NotFound(format!("run {:?} does not exist", run_id.0))
            } else {
                io_error(&run_directory, error)
            }
        })?;
        if canonical_directory != run_directory {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "run {:?} escapes the configured run root",
                run_id.0
            )));
        }

        let log_path = canonical_directory.join(EVENT_LOG_NAME);
        let metadata = fs::symlink_metadata(&log_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SkillEvalError::NotFound(format!("run {:?} has no event log", run_id.0))
            } else {
                io_error(&log_path, error)
            }
        })?;
        if !metadata.file_type().is_file() {
            return Err(SkillEvalError::InvalidConfiguration(format!(
                "event log {} is not a regular file",
                log_path.display()
            )));
        }
        Ok(log_path)
    }

    fn read_events(
        &self,
        run_id: &RunId,
        visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
    ) -> Result<SequenceState, SkillEvalError> {
        let run_directory = self.run_directory(run_id)?;
        let log_path = self.existing_log_path(run_id)?;
        let file = File::open(&log_path).map_err(|error| io_error(&log_path, error))?;
        let mut reader = BufReader::new(file);
        let mut state = SequenceState::new(run_id.clone());
        let mut bytes = Vec::new();
        let mut line = 0_u64;

        loop {
            bytes.clear();
            let byte_count = reader
                .read_until(b'\n', &mut bytes)
                .map_err(|error| io_error(&log_path, error))?;
            if byte_count == 0 {
                break;
            }
            line = line
                .checked_add(1)
                .ok_or_else(|| SkillEvalError::InvalidEvent {
                    line: u64::MAX,
                    message: "event log has too many lines".to_string(),
                })?;
            if bytes.last() != Some(&b'\n') {
                return Err(SkillEvalError::InvalidEvent {
                    line,
                    message: "partial event line".to_string(),
                });
            }
            bytes.pop();
            let event = serde_json::from_slice::<RunEvent>(&bytes).map_err(|error| {
                SkillEvalError::InvalidEvent {
                    line,
                    message: error.to_string(),
                }
            })?;
            state.accept(&event, line, &run_directory, &self.root)?;
            visitor(event)?;
        }

        Ok(state)
    }
}

impl RunStore for FileRunStore {
    fn append(&mut self, run_id: &RunId, event: &RunEvent) -> Result<(), SkillEvalError> {
        let run_directory = self.run_directory(run_id)?;
        let log_path = self.log_path(run_id)?;
        let is_existing = log_path
            .try_exists()
            .map_err(|error| io_error(&log_path, error))?;
        let mut state = if is_existing {
            self.read_events(run_id, &mut |_| Ok(()))?
        } else {
            SequenceState::new(run_id.clone())
        };
        let line = state.next_line()?;
        state.accept(event, line, &run_directory, &self.root)?;

        let mut encoded = serde_json::to_vec(event).map_err(|error| {
            SkillEvalError::InvalidConfiguration(format!("event serialization failed: {error}"))
        })?;
        encoded.push(b'\n');

        let mut options = OpenOptions::new();
        options.append(true);
        if !is_existing {
            fs::create_dir(&run_directory).map_err(|error| io_error(&run_directory, error))?;
            options.create_new(true);
        }
        let mut file = options
            .open(&log_path)
            .map_err(|error| io_error(&log_path, error))?;
        file.write_all(&encoded)
            .map_err(|error| io_error(&log_path, error))?;
        file.flush().map_err(|error| io_error(&log_path, error))
    }

    fn replay(
        &self,
        run_id: &RunId,
        visitor: &mut dyn FnMut(RunEvent) -> Result<(), SkillEvalError>,
    ) -> Result<(), SkillEvalError> {
        let state = self.read_events(run_id, visitor)?;
        if !state.is_started {
            return Err(SkillEvalError::InvalidEvent {
                line: 1,
                message: "event log has no run_started event".to_string(),
            });
        }
        Ok(())
    }

    fn find_trial(&self, selector: &TrialSelector) -> Result<TrialRecord, SkillEvalError> {
        let mut found = None;
        self.replay(&selector.run_id, &mut |event| {
            if let RunEvent::TrialCompleted { record, .. } = event
                && is_selected(&record, selector)
            {
                if found.is_some() {
                    return Err(SkillEvalError::InvalidConfiguration(format!(
                        "trial selector for artifact {:?}, case {:?}, and attempt {} is not unique",
                        selector.artifact.0, selector.case.0, selector.attempt
                    )));
                }
                found = Some(record);
            }
            Ok(())
        })?;
        found.ok_or_else(|| {
            SkillEvalError::NotFound(format!(
                "completed trial for artifact {:?}, case {:?}, and attempt {} was not found",
                selector.artifact.0, selector.case.0, selector.attempt
            ))
        })
    }
}

pub(crate) fn inspect_trial(
    selector: &TrialSelector,
    store: &dyn RunStore,
) -> Result<TrialRecord, SkillEvalError> {
    store.find_trial(selector)
}

struct SequenceState {
    run_id: RunId,
    is_started: bool,
    line_count: u64,
    trials: BTreeMap<TrialKey, TrialState>,
}

impl SequenceState {
    fn new(run_id: RunId) -> Self {
        Self {
            run_id,
            is_started: false,
            line_count: 0,
            trials: BTreeMap::new(),
        }
    }

    fn next_line(&self) -> Result<u64, SkillEvalError> {
        self.line_count
            .checked_add(1)
            .ok_or_else(|| SkillEvalError::InvalidEvent {
                line: u64::MAX,
                message: "event log has too many lines".to_string(),
            })
    }

    // TODO(AGNT-0032.T88): Validate pool-child completion purpose, trial coverage, and terminal ordering.
    fn accept(
        &mut self,
        event: &RunEvent,
        line: u64,
        run_directory: &Path,
        runs_root: &Path,
    ) -> Result<(), SkillEvalError> {
        validate_event_identity(event, &self.run_id, run_directory, runs_root, line)?;

        match event {
            RunEvent::RunStarted { .. } if self.is_started => {
                return Err(invalid_sequence(line, "duplicate run_started event"));
            }
            RunEvent::RunStarted { .. } if line != 1 => {
                return Err(invalid_sequence(
                    line,
                    "run_started must be the first event",
                ));
            }
            RunEvent::RunStarted { .. } => self.is_started = true,
            _ if !self.is_started => {
                return Err(invalid_sequence(
                    line,
                    "run_started must be the first event",
                ));
            }
            RunEvent::TrialStarted {
                key,
                models,
                harness,
                ..
            } => {
                if models.is_empty() {
                    return Err(invalid_sequence(
                        line,
                        "trial_started has an empty model route",
                    ));
                }
                if self.trials.contains_key(key) {
                    return Err(invalid_sequence(line, "duplicate trial_started event"));
                }
                self.trials.insert(
                    key.clone(),
                    TrialState::Started {
                        models: models.clone(),
                        harness: harness.clone(),
                    },
                );
            }
            RunEvent::CandidateExecuted { candidate, .. } => {
                match self.trials.get(&candidate.key) {
                    Some(TrialState::Started { models, .. })
                        if !models.contains(&candidate.model) =>
                    {
                        return Err(invalid_sequence(
                            line,
                            "candidate_executed model is not in the permitted route",
                        ));
                    }
                    Some(TrialState::Started { harness, .. }) if *harness != candidate.harness => {
                        return Err(invalid_sequence(
                            line,
                            "candidate_executed harness differs from trial_started",
                        ));
                    }
                    Some(TrialState::Started { .. }) => {}
                    Some(TrialState::CandidateExecuted(_)) => {
                        return Err(invalid_sequence(line, "duplicate candidate_executed event"));
                    }
                    Some(TrialState::Completed) => {
                        return Err(invalid_sequence(
                            line,
                            "candidate_executed follows trial_completed",
                        ));
                    }
                    None => {
                        return Err(invalid_sequence(
                            line,
                            "candidate_executed has no matching trial_started event",
                        ));
                    }
                }
                self.trials.insert(
                    candidate.key.clone(),
                    TrialState::CandidateExecuted(candidate.clone()),
                );
            }
            RunEvent::TrialCompleted { record, .. } => {
                match self.trials.get(&record.key) {
                    Some(TrialState::CandidateExecuted(candidate))
                        if !candidate_matches_record(candidate, record) =>
                    {
                        return Err(invalid_sequence(
                            line,
                            "trial_completed candidate differs from candidate_executed",
                        ));
                    }
                    Some(TrialState::CandidateExecuted(_)) => {}
                    Some(TrialState::Completed) => {
                        return Err(invalid_sequence(line, "duplicate trial_completed event"));
                    }
                    Some(TrialState::Started { .. }) | None => {
                        return Err(invalid_sequence(
                            line,
                            "trial_completed has no matching candidate_executed event",
                        ));
                    }
                }
                self.trials
                    .insert(record.key.clone(), TrialState::Completed);
            }
            _ => {}
        }

        self.line_count = line;
        Ok(())
    }
}

enum TrialState {
    Started {
        models: Vec<ModelIdentity>,
        harness: HarnessIdentity,
    },
    CandidateExecuted(CandidateArtifact),
    Completed,
}

fn validate_run_id(run_id: &RunId) -> Result<(), SkillEvalError> {
    let mut components = Path::new(&run_id.0).components();
    let is_single_normal = matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
        && !run_id.0.is_empty();
    if !is_single_normal {
        return Err(SkillEvalError::InvalidArguments(format!(
            "run identifier {:?} must be one path component",
            run_id.0
        )));
    }
    Ok(())
}

fn validate_event_identity(
    event: &RunEvent,
    run_id: &RunId,
    run_directory: &Path,
    runs_root: &Path,
    line: u64,
) -> Result<(), SkillEvalError> {
    match event {
        RunEvent::RunStarted { configuration, .. } if configuration.run_id != *run_id => Err(
            invalid_sequence(line, "run_started identity differs from the requested run"),
        ),
        RunEvent::CandidateExecuted { candidate, .. } => {
            validate_record_path(&candidate.artifact_path, run_directory, runs_root, line)?;
            validate_record_path(&candidate.transcript_path, run_directory, runs_root, line)
        }
        RunEvent::TrialCompleted { record, .. } => {
            validate_record_path(&record.artifact_path, run_directory, runs_root, line)?;
            validate_record_path(&record.transcript_path, run_directory, runs_root, line)
        }
        _ => Ok(()),
    }
}

fn validate_record_path(
    path: &Path,
    run_directory: &Path,
    runs_root: &Path,
    line: u64,
) -> Result<(), SkillEvalError> {
    let resolved = normalize_record_path(path, run_directory)
        .ok_or_else(|| invalid_sequence(line, "trial record contains an escaping path"))?;
    if !resolved.starts_with(runs_root) || resolved == runs_root {
        return Err(invalid_sequence(
            line,
            "trial record contains an escaping path",
        ));
    }

    let mut ancestor = resolved.as_path();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let canonical = fs::canonicalize(ancestor).map_err(|error| {
                    invalid_sequence(
                        line,
                        &format!("trial record path cannot be resolved: {error}"),
                    )
                })?;
                if !canonical.starts_with(runs_root) {
                    return Err(invalid_sequence(
                        line,
                        "trial record contains an escaping path",
                    ));
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ancestor = ancestor.parent().ok_or_else(|| {
                    invalid_sequence(line, "trial record contains an escaping path")
                })?;
            }
            Err(error) => {
                return Err(invalid_sequence(
                    line,
                    &format!("trial record path cannot be inspected: {error}"),
                ));
            }
        }
    }
    Ok(())
}

fn normalize_record_path(path: &Path, run_directory: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        return None;
    }
    let mut normalized = if path.is_absolute() {
        PathBuf::new()
    } else {
        run_directory.to_path_buf()
    };
    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(component) => normalized.push(component),
            Component::Prefix(_) => return None,
        }
    }
    Some(normalized)
}

fn candidate_matches_record(candidate: &CandidateArtifact, record: &TrialRecord) -> bool {
    candidate.key == record.key
        && candidate.model == record.model
        && candidate.harness == record.harness
        && candidate.artifact_path == record.artifact_path
        && candidate.transcript_path == record.transcript_path
        && candidate.usage == record.candidate_usage
}

fn is_selected(record: &TrialRecord, selector: &TrialSelector) -> bool {
    record.key.artifact == selector.artifact
        && record.key.tier == selector.tier
        && record.key.case == selector.case
        && record.key.attempt == selector.attempt
}

fn invalid_sequence(line: u64, message: &str) -> SkillEvalError {
    SkillEvalError::InvalidEvent {
        line,
        message: message.to_string(),
    }
}

fn io_error(path: &Path, error: std::io::Error) -> SkillEvalError {
    SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::model::{
        ArtifactName, CandidateArtifact, CaseId, HarnessIdentity, ModelIdentity, PauseReason,
        QualificationPolicy, QualificationPurpose, RunConfiguration, RunEvent, RunId, RunMode,
        SkillEvalError, Tier, Timestamp, TrialKey, TrialRecord, TrialSelector, TrialUsage,
        TrialVerdict,
    };
    use crate::ports::RunStore;

    use super::{FileRunStore, inspect_trial};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn restart_replays_and_finds_one_completed_trial() {
        let directory = TestDirectory::new("restart");
        let run_id = RunId("run-1".to_string());
        let record = trial_record();
        {
            let mut store = FileRunStore::new(directory.path()).unwrap();
            store.append(&run_id, &run_started(&run_id)).unwrap();
            store.append(&run_id, &trial_started(&record)).unwrap();
            store.append(&run_id, &candidate_executed(&record)).unwrap();
            store.append(&run_id, &trial_completed(&record)).unwrap();
        }

        let store = FileRunStore::new(directory.path()).unwrap();
        let mut replayed = 0;
        store
            .replay(&run_id, &mut |event| {
                assert!(matches!(
                    (replayed, event),
                    (0, RunEvent::RunStarted { .. })
                        | (1, RunEvent::TrialStarted { .. })
                        | (2, RunEvent::CandidateExecuted { .. })
                        | (3, RunEvent::TrialCompleted { .. })
                ));
                replayed += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(replayed, 4);
        assert_eq!(inspect_trial(&selector(&run_id), &store).unwrap(), record);

        let bytes = fs::read(directory.path().join("run-1/events.jsonl")).unwrap();
        assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 4);
        assert_eq!(bytes.last(), Some(&b'\n'));
    }

    #[test]
    fn route_fallback_accepts_only_an_effective_model_in_the_started_route() {
        let directory = TestDirectory::new("route");
        let run_id = RunId("run-1".to_string());
        let record = trial_record();
        let mut store = FileRunStore::new(directory.path()).unwrap();
        store.append(&run_id, &run_started(&run_id)).unwrap();

        let empty_route = RunEvent::TrialStarted {
            at: timestamp(),
            key: record.key.clone(),
            models: Vec::new(),
            harness: record.harness.clone(),
        };
        assert_invalid_at(store.append(&run_id, &empty_route), 2);
        store.append(&run_id, &trial_started(&record)).unwrap();

        let mut rejected = record.clone();
        rejected.model = ModelIdentity {
            tier: Tier::T2,
            provider: "other".to_string(),
            model: "not-permitted".to_string(),
            thinking: "low".to_string(),
        };
        assert_invalid_at(store.append(&run_id, &candidate_executed(&rejected)), 3);
        store.append(&run_id, &candidate_executed(&record)).unwrap();
    }

    #[test]
    fn checkpoint_is_required_unique_and_frozen_into_completion() {
        let directory = TestDirectory::new("checkpoint");
        let run_id = RunId("run-1".to_string());
        let record = trial_record();
        let mut store = FileRunStore::new(directory.path()).unwrap();
        store.append(&run_id, &run_started(&run_id)).unwrap();
        store.append(&run_id, &trial_started(&record)).unwrap();

        assert_invalid_at(store.append(&run_id, &trial_completed(&record)), 3);
        let checkpoint = candidate_executed(&record);
        store.append(&run_id, &checkpoint).unwrap();
        assert_invalid_at(store.append(&run_id, &checkpoint), 4);

        let mut changed = record.clone();
        changed.candidate_usage.input_tokens += 1;
        assert_invalid_at(store.append(&run_id, &trial_completed(&changed)), 4);
        store.append(&run_id, &trial_completed(&record)).unwrap();
        assert_invalid_at(store.append(&run_id, &trial_completed(&record)), 5);
    }

    #[test]
    fn pause_after_candidate_checkpoint_replays_and_resumes_without_rerun() {
        let directory = TestDirectory::new("pause");
        let run_id = RunId("run-1".to_string());
        let record = trial_record();
        {
            let mut store = FileRunStore::new(directory.path()).unwrap();
            store.append(&run_id, &run_started(&run_id)).unwrap();
            store.append(&run_id, &trial_started(&record)).unwrap();
            store.append(&run_id, &candidate_executed(&record)).unwrap();
            store
                .append(
                    &run_id,
                    &RunEvent::RunPaused {
                        at: timestamp(),
                        reason: PauseReason::Infrastructure {
                            message: "judge unavailable".to_string(),
                        },
                    },
                )
                .unwrap();
        }

        let mut store = FileRunStore::new(directory.path()).unwrap();
        store
            .append(&run_id, &RunEvent::RunResumed { at: timestamp() })
            .unwrap();
        store.append(&run_id, &trial_completed(&record)).unwrap();
        assert_eq!(store.find_trial(&selector(&run_id)).unwrap(), record);
    }

    #[test]
    fn candidate_checkpoint_requires_matching_key_and_harness() {
        let directory = TestDirectory::new("identity");
        let run_id = RunId("run-1".to_string());
        let record = trial_record();
        let mut store = FileRunStore::new(directory.path()).unwrap();
        store.append(&run_id, &run_started(&run_id)).unwrap();
        store.append(&run_id, &trial_started(&record)).unwrap();

        let mut wrong_key = record.clone();
        wrong_key.key.attempt += 1;
        assert_invalid_at(store.append(&run_id, &candidate_executed(&wrong_key)), 3);

        let mut wrong_harness = record.clone();
        wrong_harness.harness.pi_version = "different".to_string();
        assert_invalid_at(
            store.append(&run_id, &candidate_executed(&wrong_harness)),
            3,
        );
    }

    #[test]
    fn interrupted_write_fails_at_its_line_and_blocks_append() {
        let directory = TestDirectory::new("interrupted");
        let run_id = RunId("run-1".to_string());
        let mut store = FileRunStore::new(directory.path()).unwrap();
        store.append(&run_id, &run_started(&run_id)).unwrap();
        let log_path = directory.path().join("run-1/events.jsonl");
        let mut file = OpenOptions::new().append(true).open(&log_path).unwrap();
        file.write_all(b"{\"event\":\"trial_started\"").unwrap();
        file.flush().unwrap();
        drop(file);
        let before = fs::read(&log_path).unwrap();

        assert!(matches!(
            store.replay(&run_id, &mut |_| Ok(())),
            Err(SkillEvalError::InvalidEvent { line: 2, .. })
        ));
        assert!(matches!(
            store.append(&run_id, &trial_started(&trial_record())),
            Err(SkillEvalError::InvalidEvent { line: 2, .. })
        ));
        assert_eq!(fs::read(log_path).unwrap(), before);
    }

    #[test]
    fn unknown_event_fails_at_its_line() {
        let directory = TestDirectory::new("unknown");
        let run_id = RunId("run-1".to_string());
        let mut store = FileRunStore::new(directory.path()).unwrap();
        store.append(&run_id, &run_started(&run_id)).unwrap();
        let log_path = directory.path().join("run-1/events.jsonl");
        let mut file = OpenOptions::new().append(true).open(log_path).unwrap();
        file.write_all(b"{\"event\":\"future_event\"}\n").unwrap();
        file.flush().unwrap();

        assert!(matches!(
            store.replay(&run_id, &mut |_| Ok(())),
            Err(SkillEvalError::InvalidEvent { line: 2, .. })
        ));
    }

    #[test]
    fn duplicate_completed_trial_fails_before_lookup_returns() {
        let directory = TestDirectory::new("duplicate");
        let run_id = RunId("run-1".to_string());
        let record = trial_record();
        let completed = trial_completed(&record);
        let mut store = FileRunStore::new(directory.path()).unwrap();
        store.append(&run_id, &run_started(&run_id)).unwrap();
        store.append(&run_id, &trial_started(&record)).unwrap();
        store.append(&run_id, &candidate_executed(&record)).unwrap();
        store.append(&run_id, &completed).unwrap();
        let log_path = directory.path().join("run-1/events.jsonl");
        let mut file = OpenOptions::new().append(true).open(log_path).unwrap();
        serde_json::to_writer(&mut file, &completed).unwrap();
        file.write_all(b"\n").unwrap();
        file.flush().unwrap();

        assert!(matches!(
            store.find_trial(&selector(&run_id)),
            Err(SkillEvalError::InvalidEvent { line: 5, .. })
        ));
    }

    #[test]
    fn escaping_identifiers_and_record_paths_are_rejected() {
        let directory = TestDirectory::new("escape");
        let mut store = FileRunStore::new(directory.path()).unwrap();
        let escaping_id = RunId("../outside".to_string());
        assert!(matches!(
            store.append(&escaping_id, &run_started(&escaping_id)),
            Err(SkillEvalError::InvalidArguments(_))
        ));

        let run_id = RunId("run-1".to_string());
        store.append(&run_id, &run_started(&run_id)).unwrap();
        let mut record = trial_record();
        store.append(&run_id, &trial_started(&record)).unwrap();
        record.artifact_path = PathBuf::from("../../outside");
        assert!(matches!(
            store.append(&run_id, &candidate_executed(&record)),
            Err(SkillEvalError::InvalidEvent { line: 3, .. })
        ));
    }

    fn run_started(run_id: &RunId) -> RunEvent {
        RunEvent::RunStarted {
            at: timestamp(),
            configuration: RunConfiguration {
                run_id: run_id.clone(),
                mode: RunMode::Execute,
                artifacts: Vec::new(),
                change: None,
                policy: QualificationPolicy {
                    purpose: QualificationPurpose::Artifact,
                    candidate_tiers: vec![Tier::T2],
                    reference_tier: Tier::T4,
                    judge_tier: Tier::T5,
                    repeats_per_case: 1,
                    minimum_score: 7,
                    noninferiority_margin: 0.1,
                    confidence_level: 0.95,
                },
                created_at: timestamp(),
            },
        }
    }

    fn trial_started(record: &TrialRecord) -> RunEvent {
        RunEvent::TrialStarted {
            at: timestamp(),
            key: record.key.clone(),
            models: vec![fallback_model(), record.model.clone()],
            harness: record.harness.clone(),
        }
    }

    fn candidate_executed(record: &TrialRecord) -> RunEvent {
        RunEvent::CandidateExecuted {
            at: timestamp(),
            candidate: CandidateArtifact {
                key: record.key.clone(),
                model: record.model.clone(),
                harness: record.harness.clone(),
                artifact_path: record.artifact_path.clone(),
                transcript_path: record.transcript_path.clone(),
                usage: record.candidate_usage.clone(),
            },
        }
    }

    fn trial_completed(record: &TrialRecord) -> RunEvent {
        RunEvent::TrialCompleted {
            at: timestamp(),
            record: record.clone(),
        }
    }

    fn fallback_model() -> ModelIdentity {
        ModelIdentity {
            tier: Tier::T2,
            provider: "fixture".to_string(),
            model: "unavailable".to_string(),
            thinking: "low".to_string(),
        }
    }

    fn trial_record() -> TrialRecord {
        TrialRecord {
            key: TrialKey {
                artifact: ArtifactName("create-pr".to_string()),
                tier: Tier::T2,
                case: CaseId("c1".to_string()),
                attempt: 2,
            },
            model: ModelIdentity {
                tier: Tier::T2,
                provider: "fixture".to_string(),
                model: "candidate".to_string(),
                thinking: "low".to_string(),
            },
            harness: HarnessIdentity {
                runner_version: "1".to_string(),
                pi_version: "1".to_string(),
                artifact_revision: "abc".to_string(),
                tool_policy_digest: "def".to_string(),
            },
            artifact_path: PathBuf::from("artifacts/c1.txt"),
            transcript_path: PathBuf::from("transcripts/c1.jsonl"),
            candidate_usage: usage(10),
            judge_model: ModelIdentity {
                tier: Tier::T5,
                provider: "judge".to_string(),
                model: "grader".to_string(),
                thinking: "high".to_string(),
            },
            judge_usage: usage(20),
            verdict: TrialVerdict {
                score: 8,
                is_catastrophic: false,
                failure_mode: None,
                checks: Vec::new(),
            },
        }
    }

    fn usage(input_tokens: u64) -> TrialUsage {
        TrialUsage {
            input_tokens,
            output_tokens: 20,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            turns: 1,
            tool_calls: 0,
            elapsed_milliseconds: 100,
            cost_millionths_of_dollar: 5,
        }
    }

    fn selector(run_id: &RunId) -> TrialSelector {
        TrialSelector {
            run_id: run_id.clone(),
            artifact: ArtifactName("create-pr".to_string()),
            tier: Tier::T2,
            case: CaseId("c1".to_string()),
            attempt: 2,
        }
    }

    fn assert_invalid_at<T>(result: Result<T, SkillEvalError>, expected_line: u64) {
        assert!(matches!(
            result,
            Err(SkillEvalError::InvalidEvent { line, .. }) if line == expected_line
        ));
    }

    fn timestamp() -> Timestamp {
        Timestamp("2026-08-22T05:00:00-0400".to_string())
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "skill-eval-store-{}-{name}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).unwrap();
        }
    }
}
