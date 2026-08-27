use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::model::{
    ArtifactDefinition, ArtifactKind, SkillEvalError, Tier, TierAssignment, TierDestination,
};
use crate::ports::{ArtifactSource, TierWriter};
use crate::source::FileArtifactSource;

pub(crate) struct FileTierWriter;

impl TierWriter for FileTierWriter {
    fn write(
        &mut self,
        artifact: &ArtifactDefinition,
        assignments: &[TierAssignment],
    ) -> Result<(), SkillEvalError> {
        let current = FileArtifactSource.load(&artifact.root)?;
        if current.name != artifact.name
            || current.kind != artifact.kind
            || current.revision != artifact.revision
        {
            return Err(invalid("tier write rejected artifact revision drift"));
        }
        validate_assignments(&current, assignments)?;

        let replacements = match current.kind {
            ArtifactKind::Skill => skill_replacements(&current, assignments)?,
            ArtifactKind::Agent => agent_replacements(&current, assignments)?,
            ArtifactKind::Workflow => workflow_replacements(&current, assignments)?,
        };
        replace_atomically(&replacements)?;

        let updated = FileArtifactSource.load(&current.root)?;
        let expected = assignments
            .iter()
            .map(|item| (item.destination.clone(), item.tier))
            .collect::<BTreeMap<_, _>>();
        let actual = updated
            .current_tiers
            .iter()
            .map(|item| (item.destination.clone(), item.tier))
            .collect::<BTreeMap<_, _>>();
        if updated.name != current.name
            || updated.kind != current.kind
            || updated.required_destinations != current.required_destinations
            || actual != expected
        {
            return Err(invalid("tier write verification failed"));
        }
        Ok(())
    }
}

fn validate_assignments(
    artifact: &ArtifactDefinition,
    assignments: &[TierAssignment],
) -> Result<(), SkillEvalError> {
    let required = artifact
        .required_destinations
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let supplied = assignments
        .iter()
        .map(|assignment| assignment.destination.clone())
        .collect::<BTreeSet<_>>();
    if supplied.len() != assignments.len() || supplied != required {
        return Err(invalid(
            "tier write requires exactly the artifact tier destinations",
        ));
    }
    for destination in supplied {
        let is_owned = matches!(
            (artifact.kind, destination),
            (
                ArtifactKind::Skill,
                TierDestination::SkillMinimum | TierDestination::SkillTarget
            ) | (ArtifactKind::Agent, TierDestination::Agent)
                | (
                    ArtifactKind::Workflow,
                    TierDestination::WorkflowOrchestrator | TierDestination::WorkflowNode { .. }
                )
        );
        if !is_owned {
            return Err(invalid(
                "tier write destination belongs to another artifact kind",
            ));
        }
    }
    Ok(())
}

fn skill_replacements(
    artifact: &ArtifactDefinition,
    assignments: &[TierAssignment],
) -> Result<Vec<Replacement>, SkillEvalError> {
    let path = artifact.root.join("SKILL.md");
    let source = read(&path)?;
    let minimum = assignment(assignments, &TierDestination::SkillMinimum)?;
    let target = assignments
        .iter()
        .find(|item| item.destination == TierDestination::SkillTarget)
        .map(|item| item.tier);
    let output = update_frontmatter_tiers(&source, minimum, target)?;
    Ok(vec![Replacement {
        path,
        original: source,
        output,
    }])
}

fn update_frontmatter_tiers(
    source: &[u8],
    minimum: Tier,
    target: Option<Tier>,
) -> Result<Vec<u8>, SkillEvalError> {
    let text = std::str::from_utf8(source).map_err(|_| invalid("definition is not UTF-8"))?;
    let lines = line_ranges(text);
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    if lines.first().map(|range| line_text(text, range)) != Some("---") {
        return Err(invalid("definition has no opening frontmatter delimiter"));
    }
    let close = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, range)| line_text(text, range) == "---")
        .map(|(index, _)| index)
        .ok_or_else(|| invalid("definition has no closing frontmatter delimiter"))?;
    let metadata = (1..close).find(|index| line_text(text, &lines[*index]) == "metadata:");
    let mut edits = Vec::new();
    let wanted = [("minimum-tier", Some(minimum)), ("target-tier", target)];

    if let Some(metadata_index) = metadata {
        let end = (metadata_index + 1..close)
            .find(|index| !line_text(text, &lines[*index]).starts_with(' '))
            .unwrap_or(close);
        for (key, tier) in wanted {
            let found = (metadata_index + 1..end).find(|index| {
                line_text(text, &lines[*index])
                    .trim_start()
                    .starts_with(&format!("{key}:"))
            });
            match (found, tier) {
                (Some(index), Some(value)) => {
                    let range = &lines[index];
                    let line = line_text(text, range);
                    let colon = line.find(':').unwrap();
                    edits.push((
                        range.start + colon + 1,
                        range.content_end,
                        format!(" {}", tier_name(value)),
                    ));
                }
                (None, Some(value)) => {
                    let at = lines[end].start;
                    edits.push((at, at, format!("  {key}: {}{newline}", tier_name(value))));
                }
                (Some(_), None) | (None, None) => {}
            }
        }
    } else {
        let at = lines[close].start;
        let mut block = format!(
            "metadata:{newline}  minimum-tier: {}{newline}",
            tier_name(minimum)
        );
        if let Some(value) = target {
            block.push_str(&format!("  target-tier: {}{newline}", tier_name(value)));
        }
        edits.push((at, at, block));
    }
    apply_edits(source, edits)
}

fn agent_replacements(
    artifact: &ArtifactDefinition,
    assignments: &[TierAssignment],
) -> Result<Vec<Replacement>, SkillEvalError> {
    let path =
        routing_file(&artifact.root).ok_or_else(|| invalid("agent routing file is missing"))?;
    let source = read(&path)?;
    let value: Value = serde_json::from_slice(&source)
        .map_err(|error| invalid(format!("agent routing file is malformed: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid("agent routing file must be an object"))?;
    let tiers = object
        .get("tiers")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("agent routing file is missing tiers"))?;
    let tier = assignment(assignments, &TierDestination::Agent)?;
    let tier = tier_name(tier);
    if !tiers.contains_key(tier) {
        return Err(invalid("accepted tier has no model route"));
    }
    let agents = object
        .get("agents")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("agent routing file is missing agents"))?;
    if !agents.contains_key(&artifact.name.0) {
        return Err(invalid("agent routing destination is missing"));
    }
    let output = replace_json_object_string(&source, "agents", &artifact.name.0, tier)?;
    Ok(vec![Replacement {
        path,
        original: source,
        output,
    }])
}

fn workflow_replacements(
    artifact: &ArtifactDefinition,
    assignments: &[TierAssignment],
) -> Result<Vec<Replacement>, SkillEvalError> {
    let mut replacements = skill_replacements(
        artifact,
        &[TierAssignment {
            destination: TierDestination::SkillMinimum,
            tier: assignment(assignments, &TierDestination::WorkflowOrchestrator)?,
        }],
    )?;
    let workflow_path = workflow_file(&artifact.root)?;
    let source = read(&workflow_path)?;
    let text = std::str::from_utf8(&source).map_err(|_| invalid("workflow is not UTF-8"))?;
    let mut edits = Vec::new();
    let mut found = BTreeSet::new();
    for object in javascript_objects(text)? {
        let properties = javascript_properties(text, object)?;
        let Some(name) = ["node", "title", "label", "name"]
            .iter()
            .find_map(|key| properties.get(*key))
            .map(|property| property.value.as_str())
        else {
            continue;
        };
        let destination = TierDestination::WorkflowNode {
            node: name.to_owned(),
        };
        let Some(item) = assignments
            .iter()
            .find(|item| item.destination == destination)
        else {
            continue;
        };
        let property = match (properties.get("model"), properties.get("tier")) {
            (Some(property), None) => property,
            (None, Some(property)) => property,
            (Some(_), Some(_)) => return Err(invalid("workflow node has both model and tier")),
            (None, None) => continue,
        };
        if !found.insert(destination) {
            return Err(invalid("workflow repeats an assigned node"));
        }
        if property.key == "model" {
            edits.push((property.key_start, property.key_end, "tier".to_owned()));
        }
        edits.push((
            property.value_start,
            property.value_end,
            tier_name(item.tier).to_owned(),
        ));
    }
    let expected = assignments
        .iter()
        .filter_map(|item| match &item.destination {
            TierDestination::WorkflowNode { .. } => Some(item.destination.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if found != expected {
        return Err(invalid("workflow is missing an assigned node"));
    }
    let output = apply_edits(&source, edits)?;
    if javascript_properties_with_key(std::str::from_utf8(&output).unwrap(), "model")? {
        return Err(invalid("workflow still contains a model destination"));
    }
    replacements.push(Replacement {
        path: workflow_path,
        original: source,
        output,
    });
    Ok(replacements)
}

fn assignment(
    assignments: &[TierAssignment],
    destination: &TierDestination,
) -> Result<Tier, SkillEvalError> {
    assignments
        .iter()
        .find(|item| &item.destination == destination)
        .map(|item| item.tier)
        .ok_or_else(|| invalid("tier assignment is missing"))
}

fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::T1 => "T1",
        Tier::T2 => "T2",
        Tier::T3 => "T3",
        Tier::T4 => "T4",
        Tier::T5 => "T5",
    }
}

struct Replacement {
    path: PathBuf,
    original: Vec<u8>,
    output: Vec<u8>,
}

fn replace_atomically(replacements: &[Replacement]) -> Result<(), SkillEvalError> {
    replace_atomically_with_rename(replacements, |from, to| fs::rename(from, to))
}

fn replace_atomically_with_rename(
    replacements: &[Replacement],
    rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), SkillEvalError> {
    replace_atomically_with_operations(replacements, rename, |path| fs::remove_file(path))
}

fn replace_atomically_with_operations(
    replacements: &[Replacement],
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
    mut remove_backup: impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<(), SkillEvalError> {
    let permissions = replacements
        .iter()
        .map(|replacement| {
            let metadata = fs::symlink_metadata(&replacement.path)
                .map_err(|error| io(&replacement.path, error))?;
            if !metadata.file_type().is_file() {
                return Err(invalid("tier destination is not a regular file"));
            }
            Ok(metadata.permissions())
        })
        .collect::<Result<Vec<_>, SkillEvalError>>()?;

    let mut staged = Vec::new();
    for (replacement, permissions) in replacements.iter().zip(&permissions) {
        let temp = match temporary_path(&replacement.path) {
            Ok(path) => path,
            Err(error) => {
                clean_files(&staged);
                return Err(error);
            }
        };
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|error| io(&temp, error))?;
            file.set_permissions(permissions.clone())
                .map_err(|error| io(&temp, error))?;
            file.write_all(&replacement.output)
                .map_err(|error| io(&temp, error))?;
            file.sync_all().map_err(|error| io(&temp, error))?;
            let verified = read(&temp)?;
            if verified != replacement.output {
                return Err(invalid("temporary tier write verification failed"));
            }
            Ok(())
        })();
        if let Err(error) = result {
            clean_files(&staged);
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
        staged.push(temp);
    }
    for replacement in replacements {
        let is_unchanged =
            read(&replacement.path).is_ok_and(|current| current == replacement.original);
        if !is_unchanged {
            clean_files(&staged);
            return Err(invalid("tier write rejected a concurrent change"));
        }
    }

    let mut committed = Vec::new();
    for (replacement, temp) in replacements.iter().zip(&staged) {
        let backup = match backup_path(&replacement.path) {
            Ok(path) => path,
            Err(error) => {
                return Err(rollback_result(
                    error,
                    rollback(&committed, None, &staged, &mut rename),
                ));
            }
        };
        if let Err(error) = rename(&replacement.path, &backup) {
            let commit_error = io(&replacement.path, error);
            return Err(rollback_result(
                commit_error,
                rollback(&committed, None, &staged, &mut rename),
            ));
        }
        if let Err(error) = rename(temp, &replacement.path) {
            let commit_error = io(&replacement.path, error);
            return Err(rollback_result(
                commit_error,
                rollback(
                    &committed,
                    Some((&replacement.path, &backup)),
                    &staged,
                    &mut rename,
                ),
            ));
        }
        committed.push((replacement.path.clone(), backup));
    }

    for (_, backup) in &committed {
        if let Err(error) = remove_backup(backup) {
            let cleanup_error = io(backup, error);
            return match restore_after_cleanup_failure(
                replacements,
                &permissions,
                &committed,
                &staged,
                &mut rename,
            ) {
                Ok(()) => Err(cleanup_error),
                Err(rollback_error) => Err(invalid(format!(
                    "tier write rollback failed after backup cleanup error: {rollback_error:?}; cleanup error: {cleanup_error:?}"
                ))),
            };
        }
    }
    Ok(())
}

fn restore_after_cleanup_failure(
    replacements: &[Replacement],
    permissions: &[fs::Permissions],
    committed: &[(PathBuf, PathBuf)],
    staged: &[PathBuf],
    rename: &mut impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), SkillEvalError> {
    let mut restoration_temps = Vec::new();
    let mut rollback_error = None;

    for ((replacement, permissions), (target, backup)) in
        replacements.iter().zip(permissions).zip(committed)
    {
        let result = if backup.exists() {
            rename(backup, target).map_err(|error| io(target, error))
        } else {
            restore_original_from_memory(replacement, permissions, &mut restoration_temps, rename)
        };
        if let Err(error) = result
            && rollback_error.is_none()
        {
            rollback_error = Some(error);
        }
    }

    clean_restoration_files(staged, &mut rollback_error);
    clean_restoration_files(&restoration_temps, &mut rollback_error);
    let backups = committed
        .iter()
        .map(|(_, backup)| backup.clone())
        .collect::<Vec<_>>();
    clean_restoration_files(&backups, &mut rollback_error);

    for replacement in replacements {
        if !read(&replacement.path).is_ok_and(|bytes| bytes == replacement.original)
            && rollback_error.is_none()
        {
            rollback_error = Some(invalid(
                "tier write rollback did not restore original bytes",
            ));
        }
    }

    match rollback_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn restore_original_from_memory(
    replacement: &Replacement,
    permissions: &fs::Permissions,
    restoration_temps: &mut Vec<PathBuf>,
    rename: &mut impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), SkillEvalError> {
    let temp = restoration_path(&replacement.path)?;
    restoration_temps.push(temp.clone());
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| io(&temp, error))?;
    file.set_permissions(permissions.clone())
        .map_err(|error| io(&temp, error))?;
    file.write_all(&replacement.original)
        .map_err(|error| io(&temp, error))?;
    file.sync_all().map_err(|error| io(&temp, error))?;
    if read(&temp)? != replacement.original {
        return Err(invalid("temporary tier restoration verification failed"));
    }
    rename(&temp, &replacement.path).map_err(|error| io(&replacement.path, error))
}

fn rollback(
    committed: &[(PathBuf, PathBuf)],
    current: Option<(&Path, &Path)>,
    staged: &[PathBuf],
    rename: &mut impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), SkillEvalError> {
    let mut rollback_error = None;
    if let Some((target, backup)) = current {
        restore_backup(target, backup, rename, &mut rollback_error);
    }
    for (target, backup) in committed.iter().rev() {
        restore_backup(target, backup, rename, &mut rollback_error);
    }
    clean_restoration_files(staged, &mut rollback_error);
    match rollback_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn restore_backup(
    target: &Path,
    backup: &Path,
    rename: &mut impl FnMut(&Path, &Path) -> std::io::Result<()>,
    rollback_error: &mut Option<SkillEvalError>,
) {
    if let Err(error) = fs::remove_file(target)
        && error.kind() != std::io::ErrorKind::NotFound
        && rollback_error.is_none()
    {
        *rollback_error = Some(io(target, error));
    }
    if let Err(error) = rename(backup, target)
        && rollback_error.is_none()
    {
        *rollback_error = Some(io(target, error));
    }
}

fn rollback_result(
    initiating_error: SkillEvalError,
    rollback: Result<(), SkillEvalError>,
) -> SkillEvalError {
    match rollback {
        Ok(()) => initiating_error,
        Err(rollback_error) => invalid(format!(
            "tier write rollback failed: {rollback_error:?}; initiating error: {initiating_error:?}"
        )),
    }
}

fn clean_files(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn clean_restoration_files(paths: &[PathBuf], rollback_error: &mut Option<SkillEvalError>) {
    for path in paths {
        if let Err(error) = fs::remove_file(path)
            && error.kind() != std::io::ErrorKind::NotFound
            && rollback_error.is_none()
        {
            *rollback_error = Some(io(path, error));
        }
    }
}

fn temporary_path(path: &Path) -> Result<PathBuf, SkillEvalError> {
    unique_sibling(
        path,
        "tier-write",
        "cannot allocate a temporary tier destination",
    )
}

fn backup_path(path: &Path) -> Result<PathBuf, SkillEvalError> {
    unique_sibling(
        path,
        "tier-backup",
        "cannot allocate a tier destination backup",
    )
}

fn restoration_path(path: &Path) -> Result<PathBuf, SkillEvalError> {
    unique_sibling(
        path,
        "tier-restore",
        "cannot allocate a temporary tier restoration",
    )
}

fn unique_sibling(path: &Path, role: &str, failure: &str) -> Result<PathBuf, SkillEvalError> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("tier destination has no parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| invalid("tier destination has no file name"))?
        .to_string_lossy();
    for suffix in 0..1000_u16 {
        let candidate = parent.join(format!(".{name}.{role}-{suffix}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(invalid(failure))
}

fn read(path: &Path) -> Result<Vec<u8>, SkillEvalError> {
    fs::read(path).map_err(|error| io(path, error))
}

fn io(path: &Path, error: std::io::Error) -> SkillEvalError {
    SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

fn invalid(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(message.into())
}

fn routing_file(root: &Path) -> Option<PathBuf> {
    root.ancestors()
        .map(|ancestor| ancestor.join("config/model-tiers.json"))
        .find(|path| path.is_file())
}

fn workflow_file(root: &Path) -> Result<PathBuf, SkillEvalError> {
    let mut paths = fs::read_dir(root)
        .map_err(|error| io(root, error))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".workflow.js"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    if paths.len() != 1 {
        return Err(invalid("workflow destination shape is ambiguous"));
    }
    Ok(paths.remove(0))
}

struct LineRange {
    start: usize,
    content_end: usize,
}

fn line_ranges(text: &str) -> Vec<LineRange> {
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            let content_end = if index > start && bytes[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            result.push(LineRange { start, content_end });
            start = index + 1;
        }
    }
    result.push(LineRange {
        start,
        content_end: bytes.len(),
    });
    result
}

fn line_text<'a>(text: &'a str, range: &LineRange) -> &'a str {
    &text[range.start..range.content_end]
}

fn apply_edits(
    source: &[u8],
    mut edits: Vec<(usize, usize, String)>,
) -> Result<Vec<u8>, SkillEvalError> {
    edits.sort_by_key(|edit| edit.0);
    if edits.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(invalid("tier destination edits overlap"));
    }
    let mut output = source.to_vec();
    for (start, end, value) in edits.into_iter().rev() {
        output.splice(start..end, value.bytes());
    }
    Ok(output)
}

fn replace_json_object_string(
    source: &[u8],
    object_key: &str,
    member_key: &str,
    value: &str,
) -> Result<Vec<u8>, SkillEvalError> {
    let text = std::str::from_utf8(source).map_err(|_| invalid("routing file is not UTF-8"))?;
    let root = json_object_members(text, 0, text.len())?;
    let object = root
        .get(object_key)
        .ok_or_else(|| invalid("routing object is missing"))?;
    let members = json_object_members(text, object.value_start, object.value_end)?;
    let member = members
        .get(member_key)
        .ok_or_else(|| invalid("routing member is missing"))?;
    if !text[member.value_start..member.value_end]
        .trim_start()
        .starts_with('"')
    {
        return Err(invalid("routing member is not a string"));
    }
    let value_start = text[member.value_start..member.value_end]
        .find('"')
        .unwrap()
        + member.value_start
        + 1;
    let value_end = scan_json_string(text.as_bytes(), value_start - 1)? - 1;
    apply_edits(source, vec![(value_start, value_end, value.to_owned())])
}

#[derive(Clone)]
struct JsonMember {
    value_start: usize,
    value_end: usize,
}

fn json_object_members(
    text: &str,
    start: usize,
    end: usize,
) -> Result<BTreeMap<String, JsonMember>, SkillEvalError> {
    let bytes = text.as_bytes();
    let mut index = skip_space(bytes, start);
    if bytes.get(index) != Some(&b'{') {
        return Err(invalid("routing value is not an object"));
    }
    index += 1;
    let mut members = BTreeMap::new();
    loop {
        index = skip_space(bytes, index);
        if bytes.get(index) == Some(&b'}') {
            break;
        }
        if bytes.get(index) != Some(&b'"') {
            return Err(invalid("routing object key is not a string"));
        }
        let key_end = scan_json_string(bytes, index)?;
        let key: String = serde_json::from_str(&text[index..key_end])
            .map_err(|error| invalid(error.to_string()))?;
        index = skip_space(bytes, key_end);
        if bytes.get(index) != Some(&b':') {
            return Err(invalid("routing object member has no colon"));
        }
        let value_start = skip_space(bytes, index + 1);
        let value_end = scan_json_value(bytes, value_start, end)?;
        if members
            .insert(
                key,
                JsonMember {
                    value_start,
                    value_end,
                },
            )
            .is_some()
        {
            return Err(invalid("routing object repeats a key"));
        }
        index = skip_space(bytes, value_end);
        match bytes.get(index) {
            Some(b',') => index += 1,
            Some(b'}') => break,
            _ => return Err(invalid("routing object has malformed separators")),
        }
    }
    Ok(members)
}

fn skip_space(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    index
}

fn scan_json_string(bytes: &[u8], start: usize) -> Result<usize, SkillEvalError> {
    let mut index = start + 1;
    let mut is_escaped = false;
    while let Some(byte) = bytes.get(index) {
        if is_escaped {
            is_escaped = false;
        } else if *byte == b'\\' {
            is_escaped = true;
        } else if *byte == b'"' {
            return Ok(index + 1);
        }
        index += 1;
    }
    Err(invalid("routing string is unterminated"))
}

fn scan_json_value(bytes: &[u8], start: usize, end: usize) -> Result<usize, SkillEvalError> {
    if bytes.get(start) == Some(&b'"') {
        return scan_json_string(bytes, start);
    }
    let mut index = start;
    let mut depth = 0_i32;
    while index < end {
        match bytes[index] {
            b'"' => index = scan_json_string(bytes, index)?,
            b'{' | b'[' => {
                depth += 1;
                index += 1;
            }
            b'}' | b']' if depth > 0 => {
                depth -= 1;
                index += 1;
            }
            b',' | b'}' if depth == 0 => break,
            _ => index += 1,
        }
    }
    Ok(index)
}

#[derive(Clone)]
struct JsObject {
    start: usize,
    end: usize,
}
#[derive(Clone)]
struct JsProperty {
    key: String,
    key_start: usize,
    key_end: usize,
    value: String,
    value_start: usize,
    value_end: usize,
}

fn javascript_objects(source: &str) -> Result<Vec<JsObject>, SkillEvalError> {
    let bytes = source.as_bytes();
    let mut stack = Vec::new();
    let mut result = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' | b'`' => index = scan_js_string(bytes, index)?,
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
                    index += 1;
                }
                if index + 1 >= bytes.len() {
                    return Err(invalid("workflow comment is unterminated"));
                }
                index += 2;
            }
            b'{' => {
                stack.push(index);
                index += 1;
            }
            b'}' => {
                let start = stack
                    .pop()
                    .ok_or_else(|| invalid("workflow object is unbalanced"))?;
                result.push(JsObject {
                    start,
                    end: index + 1,
                });
                index += 1;
            }
            _ => index += 1,
        }
    }
    if !stack.is_empty() {
        return Err(invalid("workflow object is unterminated"));
    }
    Ok(result)
}

fn javascript_properties(
    source: &str,
    object: JsObject,
) -> Result<BTreeMap<String, JsProperty>, SkillEvalError> {
    let bytes = source.as_bytes();
    let mut properties = BTreeMap::new();
    let mut index = object.start + 1;
    let mut depth = 0_i32;
    while index < object.end - 1 {
        match bytes[index] {
            b'{' | b'[' | b'(' => {
                depth += 1;
                index += 1;
            }
            b'}' | b']' | b')' => {
                depth -= 1;
                index += 1;
            }
            b'\'' | b'"' | b'`' => index = scan_js_string(bytes, index)?,
            byte if depth == 0 && (byte.is_ascii_alphabetic() || byte == b'_') => {
                let key_start = index;
                index += 1;
                while bytes
                    .get(index)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                {
                    index += 1;
                }
                let key_end = index;
                let key = &source[key_start..key_end];
                index = skip_space(bytes, index);
                if bytes.get(index) != Some(&b':') {
                    continue;
                }
                index = skip_space(bytes, index + 1);
                if !matches!(bytes.get(index), Some(b'\'' | b'"')) {
                    continue;
                }
                let string_start = index;
                let end = scan_js_string(bytes, index)?;
                let property = JsProperty {
                    key: key.to_owned(),
                    key_start,
                    key_end,
                    value: source[string_start + 1..end - 1].to_owned(),
                    value_start: string_start + 1,
                    value_end: end - 1,
                };
                if properties.insert(key.to_owned(), property).is_some() {
                    return Err(invalid("workflow object repeats a property"));
                }
                index = end;
            }
            _ => index += 1,
        }
    }
    Ok(properties)
}

fn scan_js_string(bytes: &[u8], start: usize) -> Result<usize, SkillEvalError> {
    let quote = bytes[start];
    let mut index = start + 1;
    let mut is_escaped = false;
    while let Some(byte) = bytes.get(index) {
        if is_escaped {
            is_escaped = false;
        } else if *byte == b'\\' {
            is_escaped = true;
        } else if *byte == quote {
            return Ok(index + 1);
        }
        index += 1;
    }
    Err(invalid("workflow string is unterminated"))
}

fn javascript_properties_with_key(source: &str, key: &str) -> Result<bool, SkillEvalError> {
    for object in javascript_objects(source)? {
        if javascript_properties(source, object)?.contains_key(key) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
include!("../tests/tier_writer.rs");
#[cfg(test)]
tier_writer_tests!();
