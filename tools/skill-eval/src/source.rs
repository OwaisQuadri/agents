use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::model::{
    ArtifactDefinition, ArtifactKind, ArtifactName, CaseDefinition, CaseDrive, CaseId,
    CommandDefinition, ExecutionDefinition, SkillEvalError, Tier, TierAssignment, TierDestination,
};
use crate::ports::ArtifactSource;

const DEFAULT_TIMEOUT_SECONDS: u32 = 120;
const REVISION_HASH_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const REVISION_HASH_PRIME: u64 = 0x0000_0100_0000_01b3;

pub(crate) struct FileArtifactSource;

impl ArtifactSource for FileArtifactSource {
    fn load(&self, root: &Path) -> Result<ArtifactDefinition, SkillEvalError> {
        let root = canonical_directory(root)?;
        let discovered = discover(&root)?;
        let definition = read_frontmatter(&discovered.definition)?;
        validate_name(&definition.name)?;

        let routing = match discovered.kind {
            ArtifactKind::Skill => skill_routing(&definition.metadata)?,
            ArtifactKind::Agent => agent_routing(&root, &definition.name)?,
            ArtifactKind::Workflow => {
                workflow_routing(&definition.metadata, discovered.workflow.as_deref())?
            }
        };
        let cases = load_cases(&root)?;
        let revision = artifact_revision(&root, &discovered, &routing, &cases)?;

        Ok(ArtifactDefinition {
            name: ArtifactName(definition.name),
            kind: discovered.kind,
            root,
            revision,
            required_destinations: routing.required_destinations,
            current_tiers: routing.current_tiers,
            cases,
        })
    }
}

struct DiscoveredArtifact {
    kind: ArtifactKind,
    definition: PathBuf,
    workflow: Option<PathBuf>,
}

fn discover(root: &Path) -> Result<DiscoveredArtifact, SkillEvalError> {
    let mut markdown_files = Vec::new();
    let mut workflow_files = Vec::new();
    let entries = fs::read_dir(root).map_err(|error| io_error(root, error))?;

    for entry in entries {
        let entry = entry.map_err(|error| io_error(root, error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| io_error(&entry.path(), error))?;
        if !file_type.is_file() && !file_type.is_symlink() {
            continue;
        }
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if file_name.ends_with(".workflow.js") {
            workflow_files.push(resolve_existing_under(root, Path::new(file_name.as_ref()))?);
        } else if file_name.ends_with(".md") {
            markdown_files.push(resolve_existing_under(root, Path::new(file_name.as_ref()))?);
        }
    }

    markdown_files.sort();
    workflow_files.sort();
    let skill_path = root.join("SKILL.md");
    let is_skill_present = skill_path.is_file();

    match (is_skill_present, workflow_files.len()) {
        (true, 0) => Ok(DiscoveredArtifact {
            kind: ArtifactKind::Skill,
            definition: resolve_existing_under(root, Path::new("SKILL.md"))?,
            workflow: None,
        }),
        (true, 1) => Ok(DiscoveredArtifact {
            kind: ArtifactKind::Workflow,
            definition: resolve_existing_under(root, Path::new("SKILL.md"))?,
            workflow: workflow_files.pop(),
        }),
        (false, 0) if markdown_files.len() == 1 => Ok(DiscoveredArtifact {
            kind: ArtifactKind::Agent,
            definition: markdown_files.remove(0),
            workflow: None,
        }),
        _ => Err(invalid(format!(
            "artifact root {} does not contain exactly one unambiguous skill, agent, or workflow definition",
            root.display()
        ))),
    }
}

struct Frontmatter {
    name: String,
    metadata: BTreeMap<String, String>,
}

fn read_frontmatter(path: &Path) -> Result<Frontmatter, SkillEvalError> {
    let text = read_text(path)?;
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return Err(invalid(format!(
            "definition {} has no opening frontmatter delimiter",
            path.display()
        )));
    }

    let mut fields = BTreeMap::new();
    let mut metadata = BTreeMap::new();
    let mut is_in_metadata = false;
    let mut is_closed = false;
    let lines = lines.collect::<Vec<_>>();
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        if line == "---" {
            is_closed = true;
            break;
        }
        if line.trim().is_empty() {
            return Err(invalid(format!(
                "definition {} has a blank frontmatter line",
                path.display()
            )));
        }

        if line.starts_with(' ') {
            if !is_in_metadata {
                return Err(invalid(format!(
                    "definition {} has an unexpected indented frontmatter line",
                    path.display()
                )));
            }
            let (key, value) = parse_mapping_line(line.trim(), path)?;
            insert_unique(&mut metadata, key, value, path)?;
            index += 1;
            continue;
        }

        is_in_metadata = false;
        let (key, mut value) = parse_mapping_line(line, path)?;
        if key == "metadata" {
            if !value.is_empty() {
                return Err(invalid(format!(
                    "definition {} metadata must be a mapping",
                    path.display()
                )));
            }
            if fields.contains_key("metadata") {
                return Err(invalid(format!(
                    "definition {} repeats frontmatter field metadata",
                    path.display()
                )));
            }
            fields.insert(key, String::new());
            is_in_metadata = true;
            index += 1;
            continue;
        }

        if value == ">-" || value == ">" {
            let mut folded = Vec::new();
            index += 1;
            while index < lines.len() && lines[index].starts_with(' ') {
                folded.push(lines[index].trim());
                index += 1;
            }
            if folded.is_empty() {
                return Err(invalid(format!(
                    "definition {} has an empty folded field {key}",
                    path.display()
                )));
            }
            value = folded.join(" ");
        } else {
            index += 1;
        }
        insert_unique(&mut fields, key, unquote(&value)?, path)?;
    }

    if !is_closed {
        return Err(invalid(format!(
            "definition {} has no closing frontmatter delimiter",
            path.display()
        )));
    }
    for key in fields.keys() {
        if !matches!(
            key.as_str(),
            "name" | "description" | "metadata" | "tools" | "model"
        ) {
            return Err(invalid(format!(
                "definition {} has unsupported frontmatter field {key}",
                path.display()
            )));
        }
    }

    let name = required_field(&fields, "name", path)?;
    required_field(&fields, "description", path)?;
    Ok(Frontmatter { name, metadata })
}

fn parse_mapping_line(line: &str, path: &Path) -> Result<(String, String), SkillEvalError> {
    let Some((key, value)) = line.split_once(':') else {
        return Err(invalid(format!(
            "definition {} has malformed frontmatter line {line:?}",
            path.display()
        )));
    };
    let key = key.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err(invalid(format!(
            "definition {} has malformed frontmatter key {key:?}",
            path.display()
        )));
    }
    Ok((key.to_owned(), value.trim().to_owned()))
}

fn insert_unique(
    fields: &mut BTreeMap<String, String>,
    key: String,
    value: String,
    path: &Path,
) -> Result<(), SkillEvalError> {
    if fields.insert(key.clone(), value).is_some() {
        return Err(invalid(format!(
            "definition {} repeats frontmatter field {key}",
            path.display()
        )));
    }
    Ok(())
}

fn required_field(
    fields: &BTreeMap<String, String>,
    key: &str,
    path: &Path,
) -> Result<String, SkillEvalError> {
    let value = fields
        .get(key)
        .map(|value| value.trim())
        .unwrap_or_default();
    if value.is_empty() {
        return Err(invalid(format!(
            "definition {} is missing a non-empty {key}",
            path.display()
        )));
    }
    Ok(value.to_owned())
}

fn unquote(value: &str) -> Result<String, SkillEvalError> {
    if value.len() >= 2 {
        let first = value.as_bytes()[0];
        let last = value.as_bytes()[value.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return Ok(value[1..value.len() - 1].to_owned());
        }
    }
    if value.starts_with('\'') || value.starts_with('"') {
        return Err(invalid(format!("unterminated quoted value {value:?}")));
    }
    Ok(value.to_owned())
}

fn validate_name(name: &str) -> Result<(), SkillEvalError> {
    let is_valid = !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        });
    if !is_valid {
        return Err(invalid(format!("invalid artifact name {name:?}")));
    }
    Ok(())
}

struct ArtifactRouting {
    required_destinations: Vec<TierDestination>,
    current_tiers: Vec<TierAssignment>,
    workflow_nodes: Vec<WorkflowNodeRouting>,
}

struct WorkflowNodeRouting {
    name: String,
    model: Option<String>,
    tier: Option<String>,
}

fn skill_routing(metadata: &BTreeMap<String, String>) -> Result<ArtifactRouting, SkillEvalError> {
    let mut required_destinations = vec![TierDestination::SkillMinimum];
    let mut current_tiers = Vec::new();
    if let Some(value) = metadata.get("minimum-tier") {
        current_tiers.push(TierAssignment {
            destination: TierDestination::SkillMinimum,
            tier: parse_tier(value)?,
        });
    }
    if let Some(value) = metadata.get("target-tier") {
        required_destinations.push(TierDestination::SkillTarget);
        current_tiers.push(TierAssignment {
            destination: TierDestination::SkillTarget,
            tier: parse_tier(value)?,
        });
    }
    reject_unknown_tier_metadata(metadata)?;
    Ok(ArtifactRouting {
        required_destinations,
        current_tiers,
        workflow_nodes: Vec::new(),
    })
}

fn reject_unknown_tier_metadata(metadata: &BTreeMap<String, String>) -> Result<(), SkillEvalError> {
    for key in metadata.keys() {
        if key.ends_with("-tier") && !matches!(key.as_str(), "minimum-tier" | "target-tier") {
            return Err(invalid(format!("unknown tier destination metadata.{key}")));
        }
    }
    Ok(())
}

fn agent_routing(root: &Path, name: &str) -> Result<ArtifactRouting, SkillEvalError> {
    let current_tiers = agent_tiers(root, name)?;
    Ok(ArtifactRouting {
        required_destinations: vec![TierDestination::Agent],
        current_tiers,
        workflow_nodes: Vec::new(),
    })
}

fn agent_tiers(root: &Path, name: &str) -> Result<Vec<TierAssignment>, SkillEvalError> {
    let Some(path) = find_routing_file(root) else {
        return Ok(Vec::new());
    };
    let value: Value = serde_json::from_str(&read_text(&path)?).map_err(|error| {
        invalid(format!(
            "routing file {} is malformed: {error}",
            path.display()
        ))
    })?;
    let object = value.as_object().ok_or_else(|| {
        invalid(format!(
            "routing file {} must contain an object",
            path.display()
        ))
    })?;
    let tiers = object
        .get("tiers")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            invalid(format!(
                "routing file {} is missing the tiers object",
                path.display()
            ))
        })?;
    for tier_name in tiers.keys() {
        parse_tier(tier_name)?;
    }
    let agents = object
        .get("agents")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            invalid(format!(
                "routing file {} is missing the agents object",
                path.display()
            ))
        })?;
    let Some(value) = agents.get(name) else {
        return Ok(Vec::new());
    };
    let tier_name = value.as_str().ok_or_else(|| {
        invalid(format!(
            "routing destination agents.{name} in {} must be a tier name",
            path.display()
        ))
    })?;
    if !tiers.contains_key(tier_name) {
        return Err(invalid(format!(
            "routing destination agents.{name} names unknown tier {tier_name}"
        )));
    }
    Ok(vec![TierAssignment {
        destination: TierDestination::Agent,
        tier: parse_tier(tier_name)?,
    }])
}

fn find_routing_file(root: &Path) -> Option<PathBuf> {
    root.ancestors()
        .map(|ancestor| ancestor.join("config/model-tiers.json"))
        .find(|candidate| candidate.is_file())
}

fn workflow_routing(
    metadata: &BTreeMap<String, String>,
    workflow_path: Option<&Path>,
) -> Result<ArtifactRouting, SkillEvalError> {
    reject_unknown_tier_metadata(metadata)?;
    if metadata.contains_key("target-tier") {
        return Err(invalid(
            "workflow metadata.target-tier is a destination for the wrong artifact kind",
        ));
    }
    let floor = metadata
        .get("minimum-tier")
        .ok_or_else(|| invalid("workflow is missing orchestrator floor metadata.minimum-tier"))?;
    let path = workflow_path.ok_or_else(|| invalid("workflow executable is missing"))?;
    let source = read_text(path)?;
    let objects = javascript_objects(&source)?;
    let mut required_destinations = vec![TierDestination::WorkflowOrchestrator];
    let mut current_tiers = vec![TierAssignment {
        destination: TierDestination::WorkflowOrchestrator,
        tier: parse_tier(floor)?,
    }];
    let mut workflow_nodes = Vec::new();
    let mut node_names = BTreeSet::new();

    for object in objects {
        let properties = javascript_string_properties(object)?;
        let model = properties.get("model");
        let tier = properties.get("tier");
        if model.is_none() && tier.is_none() {
            continue;
        }
        if model.is_some() && tier.is_some() {
            return Err(invalid("workflow node has both model and tier"));
        }
        let node = ["node", "title", "label", "name"]
            .iter()
            .find_map(|key| properties.get(*key))
            .filter(|node| !node.trim().is_empty())
            .ok_or_else(|| invalid("workflow model or tier has no named node destination"))?;
        if !node_names.insert(node.clone()) {
            return Err(invalid(format!(
                "workflow repeats destination for node {node}"
            )));
        }
        let destination = TierDestination::WorkflowNode { node: node.clone() };
        required_destinations.push(destination.clone());
        if let Some(tier_name) = tier {
            current_tiers.push(TierAssignment {
                destination,
                tier: parse_tier(tier_name)?,
            });
        }
        workflow_nodes.push(WorkflowNodeRouting {
            name: node.clone(),
            model: model.cloned(),
            tier: tier.cloned(),
        });
    }
    if workflow_nodes.is_empty() {
        return Err(invalid(
            "workflow has no named model or tier node destinations",
        ));
    }
    Ok(ArtifactRouting {
        required_destinations,
        current_tiers,
        workflow_nodes,
    })
}

fn javascript_objects(source: &str) -> Result<Vec<&str>, SkillEvalError> {
    let bytes = source.as_bytes();
    let mut stack = Vec::new();
    let mut objects = Vec::new();
    let mut index = 0;
    let mut quote = None;
    let mut is_escaped = false;
    let mut is_line_comment = false;
    let mut is_block_comment = false;

    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();
        if is_line_comment {
            if byte == b'\n' {
                is_line_comment = false;
            }
            index += 1;
            continue;
        }
        if is_block_comment {
            if byte == b'*' && next == Some(b'/') {
                is_block_comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(active_quote) = quote {
            if is_escaped {
                is_escaped = false;
            } else if byte == b'\\' {
                is_escaped = true;
            } else if byte == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }
        if byte == b'/' && next == Some(b'/') {
            is_line_comment = true;
            index += 2;
            continue;
        }
        if byte == b'/' && next == Some(b'*') {
            is_block_comment = true;
            index += 2;
            continue;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
        } else if byte == b'{' {
            stack.push(index);
        } else if byte == b'}' {
            let start = stack
                .pop()
                .ok_or_else(|| invalid("workflow executable has unmatched closing brace"))?;
            objects.push(&source[start + 1..index]);
        }
        index += 1;
    }
    if quote.is_some() || is_block_comment || !stack.is_empty() {
        return Err(invalid(
            "workflow executable has an unterminated string, comment, or object",
        ));
    }
    Ok(objects)
}

fn javascript_string_properties(object: &str) -> Result<BTreeMap<String, String>, SkillEvalError> {
    let bytes = object.as_bytes();
    let mut properties = BTreeMap::new();
    let mut index = 0;
    let mut depth = 0_u32;

    while index < bytes.len() {
        match bytes[index] {
            b'{' | b'[' | b'(' => {
                depth += 1;
                index += 1;
            }
            b'}' | b']' | b')' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            b'\'' | b'"' | b'`' => {
                index = skip_javascript_string(bytes, index)?;
            }
            byte if depth == 0 && (byte.is_ascii_alphabetic() || byte == b'_') => {
                let key_start = index;
                index += 1;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'-'))
                {
                    index += 1;
                }
                let key = &object[key_start..index];
                while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                    index += 1;
                }
                if bytes.get(index) != Some(&b':') {
                    continue;
                }
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                    index += 1;
                }
                if !matches!(bytes.get(index), Some(b'\'' | b'"')) {
                    continue;
                }
                let value_start = index + 1;
                let end = skip_javascript_string(bytes, index)?;
                let value = &object[value_start..end - 1];
                if properties
                    .insert(key.to_owned(), value.to_owned())
                    .is_some()
                {
                    return Err(invalid(format!("workflow object repeats property {key}")));
                }
                index = end;
            }
            _ => index += 1,
        }
    }
    Ok(properties)
}

fn skip_javascript_string(bytes: &[u8], start: usize) -> Result<usize, SkillEvalError> {
    let quote = bytes[start];
    let mut index = start + 1;
    let mut is_escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if is_escaped {
            is_escaped = false;
        } else if byte == b'\\' {
            is_escaped = true;
        } else if byte == quote {
            return Ok(index + 1);
        }
        index += 1;
    }
    Err(invalid("workflow object contains an unterminated string"))
}

fn parse_tier(value: &str) -> Result<Tier, SkillEvalError> {
    match value {
        "T1" | "t1" => Ok(Tier::T1),
        "T2" | "t2" => Ok(Tier::T2),
        "T3" | "t3" => Ok(Tier::T3),
        "T4" | "t4" => Ok(Tier::T4),
        "T5" | "t5" => Ok(Tier::T5),
        _ => Err(invalid(format!("unknown tier {value:?}"))),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCase {
    id: String,
    input: String,
    expect: String,
    source: String,
    #[serde(default, rename = "sentinel")]
    _sentinel: String,
    #[serde(default, rename = "snapshot")]
    _snapshot: BTreeMap<String, Value>,
    #[serde(default, rename = "holdout")]
    is_holdout: bool,
    #[serde(default)]
    files: Vec<PathBuf>,
    #[serde(default)]
    execution: Option<RawExecution>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExecution {
    drive: RawDrive,
    #[serde(default)]
    allowed_tools: Vec<String>,
    #[serde(default)]
    checkpoints: Vec<String>,
    #[serde(default = "default_timeout_seconds")]
    timeout_seconds: u32,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawDrive {
    Response,
    Fixture {
        source: PathBuf,
        #[serde(default)]
        verify_commands: Vec<RawCommand>,
    },
    ExistingHarness {
        command: RawCommand,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    program: String,
    #[serde(default)]
    arguments: Vec<String>,
    working_directory: Option<PathBuf>,
}

fn default_timeout_seconds() -> u32 {
    DEFAULT_TIMEOUT_SECONDS
}

fn load_cases(root: &Path) -> Result<Vec<CaseDefinition>, SkillEvalError> {
    let path = resolve_existing_under(root, Path::new("evals/cases.jsonl"))?;
    let text = read_text(&path)?;
    let mut cases = Vec::new();
    let mut identifiers = BTreeSet::new();

    for (line_index, line) in text.lines().enumerate() {
        let line_number = line_index + 1;
        if line.trim().is_empty() {
            return Err(invalid(format!(
                "case file {} has blank line {line_number}",
                path.display()
            )));
        }
        let raw: RawCase = serde_json::from_str(line).map_err(|error| {
            invalid(format!(
                "case file {} line {line_number} is malformed: {error}",
                path.display()
            ))
        })?;
        validate_case_text(&raw, line_number)?;
        if !identifiers.insert(raw.id.clone()) {
            return Err(invalid(format!("duplicate case identifier {}", raw.id)));
        }

        let mut support_files = Vec::new();
        let mut support_set = BTreeSet::new();
        for support in raw.files {
            let resolved = resolve_existing_under(root, &support)?;
            if !support_set.insert(resolved.clone()) {
                return Err(invalid(format!(
                    "case {} repeats support file {}",
                    raw.id,
                    support.display()
                )));
            }
            support_files.push(resolved);
        }
        let execution = normalize_execution(root, &raw.id, raw.execution)?;
        cases.push(CaseDefinition {
            id: CaseId(raw.id),
            input: raw.input,
            expect: raw.expect,
            source: raw.source,
            is_holdout: raw.is_holdout,
            support_files,
            execution,
        });
    }
    if cases.is_empty() {
        return Err(invalid(format!("case file {} is empty", path.display())));
    }
    Ok(cases)
}

fn validate_case_text(raw: &RawCase, line_number: usize) -> Result<(), SkillEvalError> {
    for (field, value) in [
        ("id", raw.id.as_str()),
        ("input", raw.input.as_str()),
        ("expect", raw.expect.as_str()),
        ("source", raw.source.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(invalid(format!(
                "case line {line_number} has empty {field}"
            )));
        }
    }
    Ok(())
}

fn normalize_execution(
    root: &Path,
    case_id: &str,
    raw: Option<RawExecution>,
) -> Result<ExecutionDefinition, SkillEvalError> {
    let raw = raw.unwrap_or(RawExecution {
        drive: RawDrive::Response,
        allowed_tools: Vec::new(),
        checkpoints: Vec::new(),
        timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
    });
    if raw
        .checkpoints
        .iter()
        .any(|checkpoint| checkpoint.trim().is_empty())
    {
        return Err(invalid(format!(
            "case {case_id} has an empty execution checkpoint"
        )));
    }
    if raw.timeout_seconds == 0 {
        return Err(invalid(format!(
            "case {case_id} has a zero execution timeout"
        )));
    }
    let mut tools = BTreeSet::new();
    for tool in &raw.allowed_tools {
        if tool.trim().is_empty() || !tools.insert(tool.to_ascii_lowercase()) {
            return Err(invalid(format!(
                "case {case_id} has an empty or duplicate allowed tool"
            )));
        }
    }

    let drive = match raw.drive {
        RawDrive::Response => {
            if raw.allowed_tools.iter().any(|tool| is_write_tool(tool)) {
                return Err(invalid(format!(
                    "case {case_id} grants write tools to a response drive; a fixture is required"
                )));
            }
            CaseDrive::Response
        }
        RawDrive::Fixture {
            source,
            verify_commands,
        } => CaseDrive::Fixture {
            source: resolve_existing_under(root, &source)?,
            verify_commands: verify_commands
                .into_iter()
                .map(|command| normalize_command(root, case_id, command))
                .collect::<Result<Vec<_>, _>>()?,
        },
        RawDrive::ExistingHarness { command } => CaseDrive::ExistingHarness {
            command: normalize_command(root, case_id, command)?,
        },
    };
    Ok(ExecutionDefinition {
        drive,
        allowed_tools: raw.allowed_tools,
        timeout_seconds: raw.timeout_seconds,
    })
}

fn is_write_tool(tool: &str) -> bool {
    matches!(
        tool.trim().to_ascii_lowercase().as_str(),
        "write" | "edit" | "bash" | "shell" | "apply_patch"
    )
}

fn normalize_command(
    root: &Path,
    case_id: &str,
    raw: RawCommand,
) -> Result<CommandDefinition, SkillEvalError> {
    if raw.program.trim().is_empty() || raw.program.contains('\0') {
        return Err(invalid(format!(
            "case {case_id} has an invalid command program"
        )));
    }
    if raw.arguments.iter().any(|argument| argument.contains('\0')) {
        return Err(invalid(format!(
            "case {case_id} has a command argument containing a null byte"
        )));
    }
    let working_directory = raw
        .working_directory
        .map(|path| resolve_existing_under(root, &path))
        .transpose()?;
    Ok(CommandDefinition {
        program: raw.program,
        arguments: raw.arguments,
        working_directory,
    })
}

fn artifact_revision(
    root: &Path,
    discovered: &DiscoveredArtifact,
    routing: &ArtifactRouting,
    cases: &[CaseDefinition],
) -> Result<String, SkillEvalError> {
    let mut hasher = RevisionHasher::new();
    hash_revision_path(&mut hasher, root, &discovered.definition, "definition")?;
    if let Some(workflow) = &discovered.workflow {
        hash_revision_path(&mut hasher, root, workflow, "workflow")?;
    }
    hash_routing_revision(&mut hasher, routing);
    hash_revision_path(&mut hasher, root, &root.join("evals/cases.jsonl"), "cases")?;
    for case in cases {
        for support in &case.support_files {
            hash_revision_path(&mut hasher, root, support, "support")?;
        }
        match &case.execution.drive {
            CaseDrive::Response => {}
            CaseDrive::Fixture {
                source,
                verify_commands,
            } => {
                hash_revision_path(&mut hasher, root, source, "fixture")?;
                for command in verify_commands {
                    hash_local_program(&mut hasher, root, command, "verification program")?;
                }
            }
            CaseDrive::ExistingHarness { command } => {
                hash_local_program(&mut hasher, root, command, "harness program")?;
            }
        }
    }
    Ok(hasher.finish())
}

fn hash_routing_revision(hasher: &mut RevisionHasher, routing: &ArtifactRouting) {
    hasher.add(b"required destinations");
    for destination in &routing.required_destinations {
        match destination {
            TierDestination::SkillMinimum => hasher.add(b"skill minimum"),
            TierDestination::SkillTarget => hasher.add(b"skill target"),
            TierDestination::Agent => hasher.add(b"agent"),
            TierDestination::WorkflowOrchestrator => hasher.add(b"workflow orchestrator"),
            TierDestination::WorkflowNode { node } => {
                hasher.add(b"workflow node");
                hasher.add(node.as_bytes());
            }
        }
    }
    hasher.add(b"workflow node routing");
    for node in &routing.workflow_nodes {
        hasher.add(node.name.as_bytes());
        if let Some(model) = &node.model {
            hasher.add(b"model");
            hasher.add(model.as_bytes());
        }
        if let Some(tier) = &node.tier {
            hasher.add(b"tier");
            hasher.add(tier.as_bytes());
        }
    }
}

fn hash_local_program(
    hasher: &mut RevisionHasher,
    root: &Path,
    command: &CommandDefinition,
    role: &str,
) -> Result<(), SkillEvalError> {
    let program = Path::new(&command.program);
    let mut candidates = Vec::new();
    if program.is_absolute() {
        candidates.push(program.to_path_buf());
    } else {
        if let Some(working_directory) = &command.working_directory {
            candidates.push(working_directory.join(program));
        }
        candidates.push(root.join(program));
    }
    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }
        let canonical =
            fs::canonicalize(&candidate).map_err(|error| io_error(&candidate, error))?;
        if canonical.starts_with(root) {
            return hash_revision_path(hasher, root, &canonical, role);
        }
    }
    Ok(())
}

fn hash_revision_path(
    hasher: &mut RevisionHasher,
    root: &Path,
    path: &Path,
    role: &str,
) -> Result<(), SkillEvalError> {
    let canonical = fs::canonicalize(path).map_err(|error| io_error(path, error))?;
    if !canonical.starts_with(root) {
        return Err(invalid(format!(
            "revision input {} escapes artifact root {}",
            path.display(),
            root.display()
        )));
    }
    let relative = canonical
        .strip_prefix(root)
        .map_err(|_| invalid("revision input has no artifact-relative path"))?;
    hasher.add(role.as_bytes());
    hash_revision_entry(hasher, root, &canonical, relative)
}

fn hash_revision_entry(
    hasher: &mut RevisionHasher,
    root: &Path,
    path: &Path,
    relative: &Path,
) -> Result<(), SkillEvalError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink() {
        return Err(invalid(format!(
            "revision input {} must not be a symbolic link",
            path.display()
        )));
    }
    hasher.add(relative.as_os_str().as_encoded_bytes());
    if metadata.is_file() {
        hasher.add(b"file");
        hasher.add(&fs::read(path).map_err(|error| io_error(path, error))?);
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(invalid(format!(
            "revision input {} is not a regular file or directory",
            path.display()
        )));
    }
    hasher.add(b"directory");
    let mut children = fs::read_dir(path)
        .map_err(|error| io_error(path, error))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| io_error(path, error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    children.sort();
    for child in children {
        let canonical = fs::canonicalize(&child).map_err(|error| io_error(&child, error))?;
        if !canonical.starts_with(root) {
            return Err(invalid(format!(
                "revision input {} escapes artifact root {}",
                child.display(),
                root.display()
            )));
        }
        let name = child
            .file_name()
            .ok_or_else(|| invalid("revision input entry has no file name"))?;
        hash_revision_entry(hasher, root, &child, &relative.join(name))?;
    }
    Ok(())
}

struct RevisionHasher {
    state: u64,
}

impl RevisionHasher {
    fn new() -> Self {
        Self {
            state: REVISION_HASH_OFFSET,
        }
    }

    fn add(&mut self, bytes: &[u8]) {
        for byte in (bytes.len() as u64).to_le_bytes().iter().chain(bytes) {
            self.state ^= u64::from(*byte);
            self.state = self.state.wrapping_mul(REVISION_HASH_PRIME);
        }
    }

    fn finish(self) -> String {
        format!("fnv1a64:{:016x}", self.state)
    }
}

fn canonical_directory(path: &Path) -> Result<PathBuf, SkillEvalError> {
    let canonical = fs::canonicalize(path).map_err(|error| io_error(path, error))?;
    if !canonical.is_dir() {
        return Err(invalid(format!(
            "artifact root {} is not a directory",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn resolve_existing_under(root: &Path, relative: &Path) -> Result<PathBuf, SkillEvalError> {
    validate_relative_path(relative)?;
    let candidate = root.join(relative);
    let canonical = fs::canonicalize(&candidate).map_err(|error| io_error(&candidate, error))?;
    if !canonical.starts_with(root) {
        return Err(invalid(format!(
            "path {} escapes artifact root {}",
            relative.display(),
            root.display()
        )));
    }
    Ok(canonical)
}

fn validate_relative_path(path: &Path) -> Result<(), SkillEvalError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(invalid(format!(
            "path {} must be a non-empty relative path",
            path.display()
        )));
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(invalid(format!(
                "path {} escapes or is not normalized",
                path.display()
            )));
        }
    }
    Ok(())
}

fn read_text(path: &Path) -> Result<String, SkillEvalError> {
    fs::read_to_string(path).map_err(|error| io_error(path, error))
}

fn io_error(path: &Path, error: std::io::Error) -> SkillEvalError {
    SkillEvalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

fn invalid(message: impl Into<String>) -> SkillEvalError {
    SkillEvalError::InvalidConfiguration(message.into())
}

#[cfg(test)]
include!("../tests/source_destinations.rs");

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "skill-eval-source-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(root.join("evals")).unwrap();
            Self { root }
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, contents).unwrap();
        }

        fn load(&self) -> Result<ArtifactDefinition, SkillEvalError> {
            FileArtifactSource.load(&self.root)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    fn skill_definition(metadata: &str) -> String {
        format!(
            "---\nname: fixture-skill\ndescription: A complete fixture description.\nmetadata:\n  short-description: Fixture\n{metadata}---\n\n# fixture\n"
        )
    }

    fn one_case(extra: &str) -> String {
        format!(
            "{{\"id\":\"c1\",\"input\":\"ordinary input\",\"expect\":\"ordinary output\",\"source\":\"seed\"{extra}}}\n"
        )
    }

    fn invalid_message(error: SkillEvalError) -> String {
        match error {
            SkillEvalError::InvalidConfiguration(message) => message,
            other => panic!("expected invalid configuration, got {other:?}"),
        }
    }

    #[test]
    fn source_loads_skill_tiers_support_and_response_default() {
        let fixture = Fixture::new();
        fixture.write(
            "SKILL.md",
            &skill_definition("  minimum-tier: T3\n  target-tier: T2\n"),
        );
        fixture.write("support.md", "support");
        fixture.write(
            "evals/cases.jsonl",
            &one_case(",\"files\":[\"support.md\"]"),
        );

        let artifact = fixture.load().unwrap();

        assert_eq!(artifact.kind, ArtifactKind::Skill);
        assert_eq!(artifact.name, ArtifactName("fixture-skill".to_owned()));
        assert_eq!(
            artifact.current_tiers,
            vec![
                TierAssignment {
                    destination: TierDestination::SkillMinimum,
                    tier: Tier::T3,
                },
                TierAssignment {
                    destination: TierDestination::SkillTarget,
                    tier: Tier::T2,
                },
            ]
        );
        assert_eq!(
            artifact.cases[0].support_files,
            vec![artifact.root.join("support.md")]
        );
        assert_eq!(artifact.cases[0].execution.drive, CaseDrive::Response);
        assert!(artifact.cases[0].execution.allowed_tools.is_empty());
        assert_eq!(artifact.cases[0].execution.timeout_seconds, 120);
    }

    #[test]
    fn source_revision_is_stable_for_unchanged_inputs() {
        let fixture = Fixture::new();
        fixture.write("SKILL.md", &skill_definition(""));
        fixture.write("support.md", "support");
        fixture.write("fixtures/input.txt", "fixture");
        fixture.write(
            "evals/cases.jsonl",
            &one_case(
                ",\"files\":[\"support.md\"],\"execution\":{\"drive\":{\"kind\":\"fixture\",\"source\":\"fixtures\",\"verify_commands\":[]},\"allowed_tools\":[\"Edit\"],\"timeout_seconds\":30}",
            ),
        );

        let first = fixture.load().unwrap().revision;
        let second = fixture.load().unwrap().revision;

        assert_eq!(first, second);
    }

    #[test]
    fn source_revision_changes_with_definition_or_executable_case_data() {
        let fixture = Fixture::new();
        fixture.write("SKILL.md", &skill_definition(""));
        fixture.write("fixtures/input.txt", "first");
        fixture.write(
            "evals/cases.jsonl",
            &one_case(
                ",\"execution\":{\"drive\":{\"kind\":\"fixture\",\"source\":\"fixtures\",\"verify_commands\":[]},\"allowed_tools\":[\"Edit\"],\"timeout_seconds\":30}",
            ),
        );
        let original = fixture.load().unwrap().revision;

        fixture.write("SKILL.md", &skill_definition("  minimum-tier: T2\n"));
        let definition_revision = fixture.load().unwrap().revision;
        fixture.write("fixtures/input.txt", "second");
        let fixture_revision = fixture.load().unwrap().revision;
        fixture.write(
            "evals/cases.jsonl",
            &one_case(
                ",\"execution\":{\"drive\":{\"kind\":\"fixture\",\"source\":\"fixtures\",\"verify_commands\":[]},\"allowed_tools\":[\"Edit\"],\"timeout_seconds\":31}",
            ),
        );
        let case_revision = fixture.load().unwrap().revision;
        fixture.write("evals/run.sh", "first\n");
        fixture.write(
            "evals/cases.jsonl",
            &one_case(
                ",\"execution\":{\"drive\":{\"kind\":\"existing_harness\",\"command\":{\"program\":\"evals/run.sh\",\"arguments\":[],\"working_directory\":\"evals\"}},\"timeout_seconds\":31}",
            ),
        );
        let harness_revision = fixture.load().unwrap().revision;
        fixture.write("evals/run.sh", "second\n");
        let changed_harness_revision = fixture.load().unwrap().revision;

        assert_ne!(original, definition_revision);
        assert_ne!(definition_revision, fixture_revision);
        assert_ne!(fixture_revision, case_revision);
        assert_ne!(case_revision, harness_revision);
        assert_ne!(harness_revision, changed_harness_revision);
    }

    #[test]
    fn source_loads_agent_assignment_from_fixture_routing_file() {
        let fixture = Fixture::new();
        fixture.write(
            "fixture-agent.md",
            "---\nname: fixture-agent\ndescription: A complete agent description.\ntools: Read\nmodel: sonnet\n---\nbody\n",
        );
        fixture.write(
            "config/model-tiers.json",
            r#"{"tiers":{"T2":{}},"agents":{"fixture-agent":"T2"}}"#,
        );
        fixture.write("evals/cases.jsonl", &one_case(""));

        let artifact = fixture.load().unwrap();

        assert_eq!(artifact.kind, ArtifactKind::Agent);
        assert_eq!(
            artifact.current_tiers,
            vec![TierAssignment {
                destination: TierDestination::Agent,
                tier: Tier::T2,
            }]
        );
    }

    #[test]
    fn source_loads_workflow_floor_and_named_node_tiers() {
        let fixture = Fixture::new();
        fixture.write("SKILL.md", &skill_definition("  minimum-tier: T3\n"));
        fixture.write(
            "fixture.workflow.js",
            "export const meta = { phases: [\n  { title: 'Plan', tier: 'T2' },\n  { title: 'Review', tier: 'T4' },\n] }\n",
        );
        fixture.write("evals/cases.jsonl", &one_case(""));

        let artifact = fixture.load().unwrap();

        assert_eq!(artifact.kind, ArtifactKind::Workflow);
        assert_eq!(
            artifact.current_tiers,
            vec![
                TierAssignment {
                    destination: TierDestination::WorkflowOrchestrator,
                    tier: Tier::T3,
                },
                TierAssignment {
                    destination: TierDestination::WorkflowNode {
                        node: "Plan".to_owned(),
                    },
                    tier: Tier::T2,
                },
                TierAssignment {
                    destination: TierDestination::WorkflowNode {
                        node: "Review".to_owned(),
                    },
                    tier: Tier::T4,
                },
            ]
        );
    }

    source_destination_tests!();

    #[test]
    fn source_accepts_workflow_model_names() {
        let fixture = Fixture::new();
        fixture.write("SKILL.md", &skill_definition("  minimum-tier: T3\n"));
        fixture.write(
            "fixture.workflow.js",
            "export const meta = { phases: [{ title: 'Plan', model: 'sonnet' }] }\n",
        );
        fixture.write("evals/cases.jsonl", &one_case(""));

        let artifact = fixture.load().unwrap();

        assert_eq!(
            artifact.required_destinations,
            vec![
                TierDestination::WorkflowOrchestrator,
                TierDestination::WorkflowNode {
                    node: "Plan".to_owned(),
                },
            ]
        );
        assert_eq!(artifact.current_tiers.len(), 1);
    }

    #[test]
    fn source_rejects_duplicate_malformed_and_undescribed_inputs() {
        let fixture = Fixture::new();
        fixture.write(
            "SKILL.md",
            "---\nname: fixture-skill\ndescription:\n---\nbody\n",
        );
        fixture.write("evals/cases.jsonl", &(one_case("") + &one_case("")));
        assert!(invalid_message(fixture.load().unwrap_err()).contains("description"));

        fixture.write("SKILL.md", &skill_definition(""));
        assert!(invalid_message(fixture.load().unwrap_err()).contains("duplicate case"));

        fixture.write("evals/cases.jsonl", "not-json\n");
        assert!(invalid_message(fixture.load().unwrap_err()).contains("malformed"));
    }

    #[test]
    fn source_out_of_root_rejected_before_access() {
        let fixture = Fixture::new();
        fixture.write("SKILL.md", &skill_definition(""));
        let sentinel = fixture.root.parent().unwrap().join(format!(
            "source-sentinel-{}",
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&sentinel, "unchanged").unwrap();
        let sentinel_name = sentinel.file_name().unwrap().to_string_lossy();
        fixture.write(
            "evals/cases.jsonl",
            &one_case(&format!(",\"files\":[\"../{sentinel_name}\"]")),
        );
        let before = fs::metadata(&sentinel).unwrap().modified().unwrap();

        let message = invalid_message(fixture.load().unwrap_err());

        assert!(message.contains("escapes or is not normalized"));
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "unchanged");
        assert_eq!(fs::metadata(&sentinel).unwrap().modified().unwrap(), before);
        fs::remove_file(sentinel).unwrap();
    }

    #[test]
    fn source_response_drive_rejects_write_tools() {
        let fixture = Fixture::new();
        fixture.write("SKILL.md", &skill_definition(""));
        fixture.write(
            "evals/cases.jsonl",
            &one_case(
                ",\"execution\":{\"drive\":{\"kind\":\"response\"},\"allowed_tools\":[\"Write\"],\"timeout_seconds\":30}",
            ),
        );

        let message = invalid_message(fixture.load().unwrap_err());

        assert!(message.contains("fixture is required"));
    }

    #[test]
    fn source_loads_fixture_and_existing_harness_drives() {
        let fixture = Fixture::new();
        fixture.write("SKILL.md", &skill_definition(""));
        fixture.write("fixtures/input.txt", "input");
        fixture.write("work/.keep", "");
        fixture.write("evals/run.sh", "exit 0\n");
        let first = one_case(
            ",\"execution\":{\"drive\":{\"kind\":\"fixture\",\"source\":\"fixtures/input.txt\",\"verify_commands\":[{\"program\":\"check\",\"arguments\":[\"ok\"],\"working_directory\":\"work\"}]},\"allowed_tools\":[\"Edit\"],\"timeout_seconds\":30}",
        );
        let second = "{\"id\":\"c2\",\"input\":\"input two\",\"expect\":\"output two\",\"source\":\"seed\",\"holdout\":true,\"execution\":{\"drive\":{\"kind\":\"existing_harness\",\"command\":{\"program\":\"evals/run.sh\",\"arguments\":[],\"working_directory\":\"evals\"}},\"timeout_seconds\":45}}\n";
        fixture.write("evals/cases.jsonl", &(first + second));

        let artifact = fixture.load().unwrap();

        let CaseDrive::Fixture {
            source,
            verify_commands,
        } = &artifact.cases[0].execution.drive
        else {
            panic!("expected fixture drive");
        };
        assert_eq!(source, &artifact.root.join("fixtures/input.txt"));
        assert_eq!(
            verify_commands[0].working_directory,
            Some(artifact.root.join("work"))
        );
        assert!(artifact.cases[1].is_holdout);
        assert!(matches!(
            artifact.cases[1].execution.drive,
            CaseDrive::ExistingHarness { .. }
        ));
    }

    #[test]
    fn source_rejects_unknown_tier_destination() {
        let fixture = Fixture::new();
        fixture.write("SKILL.md", &skill_definition("  reviewer-tier: T4\n"));
        fixture.write("evals/cases.jsonl", &one_case(""));

        let message = invalid_message(fixture.load().unwrap_err());

        assert!(message.contains("unknown tier destination"));
    }
}
