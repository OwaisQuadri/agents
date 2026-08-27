use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::model::{
    ArtifactDefinition, ArtifactName, AuditBrief, CaseId, FailureCount, SkillEvalError, Tier,
    TierDestination,
};

#[derive(Serialize)]
struct Reproduction<'a> {
    artifact: &'a crate::model::ArtifactName,
    case: &'a CaseId,
    failure_modes: &'a [String],
}

pub(crate) struct AuditDraft {
    pub(crate) artifact: ArtifactName,
    pub(crate) failures: Vec<(CaseId, Vec<String>)>,
}

pub(crate) fn reject_candidate_mutations_at_roots(roots: &[PathBuf]) -> Result<(), SkillEvalError> {
    for root in roots {
        reject_candidate_mutations_at_root(root, None)?;
    }
    Ok(())
}

pub(crate) fn reject_candidate_mutations(
    artifacts: &[ArtifactDefinition],
) -> Result<(), SkillEvalError> {
    for artifact in artifacts {
        reject_candidate_mutations_at_root(&artifact.root, Some(&artifact.name))?;
    }
    Ok(())
}

fn reject_candidate_mutations_at_root(
    root: &Path,
    artifact: Option<&ArtifactName>,
) -> Result<(), SkillEvalError> {
    for relative in [Path::new("candidate.md"), Path::new("evals/candidate.md")] {
        let path = root.join(relative);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                let identity = artifact.map_or_else(
                    || format!("artifact root {root:?}"),
                    |artifact| format!("artifact {:?}", artifact.0),
                );
                return Err(SkillEvalError::InvalidConfiguration(format!(
                    "{identity} has an existing candidate mutation"
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(&path, error)),
        }
    }
    Ok(())
}

pub(crate) fn incumbent_tier(artifact: &ArtifactDefinition) -> Result<Tier, SkillEvalError> {
    let destination = match artifact.kind {
        crate::model::ArtifactKind::Skill => TierDestination::SkillMinimum,
        crate::model::ArtifactKind::Agent => TierDestination::Agent,
        crate::model::ArtifactKind::Workflow => TierDestination::WorkflowOrchestrator,
    };
    let tiers = artifact
        .current_tiers
        .iter()
        .filter(|assignment| assignment.destination == destination)
        .map(|assignment| assignment.tier)
        .collect::<Vec<_>>();
    if tiers.len() != 1 {
        return Err(SkillEvalError::InvalidConfiguration(format!(
            "artifact {:?} must declare exactly one incumbent base tier",
            artifact.name.0
        )));
    }
    Ok(tiers[0])
}

pub(crate) fn failure_modes(verdict: &crate::model::TrialVerdict) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(mode) = &verdict.failure_mode {
        failures.push(mode.clone());
    }
    if verdict.is_catastrophic && failures.is_empty() {
        failures.push("catastrophic".to_owned());
    }
    if verdict
        .checks
        .iter()
        .any(|check| check.status == crate::model::CheckStatus::Failed)
        && failures.is_empty()
    {
        failures.push("deterministic_check".to_owned());
    }
    failures
}

pub(crate) fn validate_output_root(path: &Path) -> Result<(), SkillEvalError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(SkillEvalError::InvalidArguments(
            "audit output root must not be empty or contain a parent segment".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn write_audit_briefs(
    output_root: &Path,
    drafts: Vec<AuditDraft>,
) -> Result<Vec<AuditBrief>, SkillEvalError> {
    let output_root = create_output_root(output_root)?;
    reject_output_collisions(&output_root, &drafts)?;
    let mut briefs = Vec::with_capacity(drafts.len());

    for (artifact_index, draft) in drafts.into_iter().enumerate() {
        let artifact_root =
            contained_directory(&output_root, &format!("artifact-{:04}", artifact_index + 1))?;
        let reproduction_root = contained_directory(&artifact_root, "reproductions")?;
        let mut counts = std::collections::BTreeMap::<String, u32>::new();
        let mut reproductions = Vec::with_capacity(draft.failures.len());

        for (case_index, (case, modes)) in draft.failures.iter().enumerate() {
            for mode in modes {
                let count = counts.entry(mode.clone()).or_default();
                *count = count.checked_add(1).ok_or_else(|| {
                    SkillEvalError::InvalidConfiguration(
                        "audit failure count exceeds the supported range".to_owned(),
                    )
                })?;
            }
            let path = reproduction_root.join(format!("case-{:04}.json", case_index + 1));
            ensure_contained(&output_root, &path)?;
            write_json(
                &path,
                &Reproduction {
                    artifact: &draft.artifact,
                    case,
                    failure_modes: modes,
                },
            )?;
            reproductions.push(path);
        }

        let brief = AuditBrief {
            artifact: draft.artifact,
            failure_modes: counts
                .into_iter()
                .map(|(failure_mode, count)| FailureCount {
                    failure_mode,
                    count,
                })
                .collect(),
            reproductions,
        };
        write_json(&artifact_root.join("brief.json"), &brief)?;
        briefs.push(brief);
    }

    Ok(briefs)
}

fn create_output_root(path: &Path) -> Result<PathBuf, SkillEvalError> {
    validate_output_root(path)?;
    fs::create_dir_all(path).map_err(|error| io_error(path, error))?;
    fs::canonicalize(path).map_err(|error| io_error(path, error))
}

fn reject_output_collisions(
    output_root: &Path,
    drafts: &[AuditDraft],
) -> Result<(), SkillEvalError> {
    for (artifact_index, draft) in drafts.iter().enumerate() {
        let artifact_root = output_root.join(format!("artifact-{:04}", artifact_index + 1));
        let reproduction_root = artifact_root.join("reproductions");
        for case_index in 0..draft.failures.len() {
            reject_collision(&reproduction_root.join(format!("case-{:04}.json", case_index + 1)))?;
        }
        reject_collision(&artifact_root.join("brief.json"))?;
        reject_collision(&reproduction_root)?;
        reject_collision(&artifact_root)?;
    }
    Ok(())
}

fn reject_collision(path: &Path) -> Result<(), SkillEvalError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(SkillEvalError::InvalidArguments(format!(
            "audit evidence path {path:?} already exists"
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path, error)),
    }
}

fn contained_directory(root: &Path, component: &str) -> Result<PathBuf, SkillEvalError> {
    let path = root.join(component);
    ensure_contained(root, &path)?;
    fs::create_dir(&path).map_err(|error| io_error(&path, error))?;
    let canonical = fs::canonicalize(&path).map_err(|error| io_error(&path, error))?;
    ensure_contained(root, &canonical)?;
    Ok(canonical)
}

fn ensure_contained(root: &Path, path: &Path) -> Result<(), SkillEvalError> {
    if !path.starts_with(root) {
        return Err(SkillEvalError::InvalidArguments(
            "audit output path escapes the output root".to_owned(),
        ));
    }
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), SkillEvalError> {
    let bytes = serde_json::to_vec(value).map_err(|error| SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.write_all(&bytes)
        .map_err(|error| io_error(path, error))
}

fn io_error(path: &Path, error: std::io::Error) -> SkillEvalError {
    SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}
