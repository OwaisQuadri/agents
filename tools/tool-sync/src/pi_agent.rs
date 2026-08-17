use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::SyncError;

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct AgentSource {
    pub path: PathBuf,
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub model: String,
    pub prompt: String,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
struct PiAgent {
    name: String,
    description: String,
    tools: Vec<String>,
    model: String,
    prompt: String,
}

#[derive(Debug, Eq, PartialEq)]
struct AgentAdapterReport {
    rendered: Vec<PathBuf>,
    overlapping_builtins: Vec<String>,
}

static TEMP_WRITE_ID: AtomicU64 = AtomicU64::new(0);

/// Finds source agent definitions beneath one repository directory.
/// It takes the agent root and returns sorted Markdown paths.
///
/// # Errors
///
/// Returns `SyncError` when the directory cannot be read or contains an unsafe path.
pub fn discover(root: &Path) -> Result<Vec<PathBuf>, SyncError> {
    validate_tree_root(root)?;

    let mut sources = Vec::new();
    collect_sources(root, &mut sources)?;
    sources.sort();

    let mut names = HashSet::new();
    for source in &sources {
        let agent = parse(source)?;
        if !names.insert(agent.name.clone()) {
            return Err(invalid(
                source,
                format!("duplicate agent name {}", agent.name),
            ));
        }
    }

    Ok(sources)
}

/// Computes the deterministic Pi-compatible bytes for one source agent.
/// It takes a source path and returns the rendered Markdown bytes without writing them.
///
/// # Errors
///
/// Returns `SyncError` for unreadable input, invalid frontmatter, or unsupported fields.
pub fn rendered_bytes(source: &Path) -> Result<Vec<u8>, SyncError> {
    let source = parse(source)?;
    let agent = adapt(source)?;
    Ok(render_pi_markdown(&agent).into_bytes())
}

/// Renders one source agent as a Pi-compatible agent definition.
/// It takes source and destination paths and returns unit after the destination is verified.
///
/// # Errors
///
/// Returns `SyncError` for unreadable input, invalid frontmatter, unsupported fields, or filesystem failures.
pub fn render(source: &Path, destination: &Path) -> Result<(), SyncError> {
    let rendered = rendered_bytes(source)?;
    match fs::symlink_metadata(destination) {
        Ok(_) => accept_exact_destination(destination, &rendered),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write_verified(destination, &rendered)
        }
        Err(error) => Err(SyncError::Io(destination.to_path_buf(), error)),
    }
}

/// Finds project agent names that overlap bundled pi-subagents roles.
/// It takes source agent paths and returns sorted canonical role names.
///
/// # Errors
///
/// Returns `SyncError` when a source cannot be read or parsed.
pub fn builtin_overlaps(sources: &[PathBuf]) -> Result<Vec<String>, SyncError> {
    let builtins = pi_subagents_builtins();
    let mut report = AgentAdapterReport {
        rendered: Vec::new(),
        overlapping_builtins: Vec::new(),
    };

    for source in sources {
        let agent = parse(source)?;
        if builtins.iter().any(|builtin| *builtin == agent.name) {
            report.overlapping_builtins.push(agent.name);
        }
    }

    report.overlapping_builtins.sort();
    report.overlapping_builtins.dedup();
    Ok(report.overlapping_builtins)
}

pub fn parse(source: &Path) -> Result<AgentSource, SyncError> {
    let text = fs::read_to_string(source).map_err(|error| SyncError::Io(source.to_path_buf(), error))?;
    let (frontmatter_start, frontmatter_end, body_start) = split_frontmatter(&text)
        .ok_or_else(|| invalid(source, "has no closed frontmatter fence"))?;
    let frontmatter = &text[frontmatter_start..frontmatter_end];
    let prompt = &text[body_start..];
    if prompt.is_empty() {
        return Err(invalid(source, "has no prompt body"));
    }

    let parsed = parse_frontmatter(source, frontmatter)?;
    let name = parse_name(source, &parsed.name)?;
    let description = parse_scalar_field(source, "description", &parsed.description)?;
    let tools = parse_tools(source, parsed.tools)?;
    let model = parse_scalar_field(source, "model", &parsed.model)?;

    Ok(AgentSource {
        path: source.to_path_buf(),
        name,
        description,
        tools,
        model,
        prompt: prompt.to_string(),
    })
}

fn adapt(source: AgentSource) -> Result<PiAgent, SyncError> {
    let AgentSource {
        path,
        name,
        description,
        tools,
        model,
        prompt,
    } = source;

    let tools = tools
        .into_iter()
        .map(|tool| {
            normalize_tool(&tool)
                .map(str::to_owned)
                .ok_or_else(|| invalid(&path, format!("frontmatter tool {tool} is unsupported")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let model = resolve_model_alias(&model, &path)?;

    Ok(PiAgent {
        name,
        description,
        tools,
        model,
        prompt,
    })
}

fn collect_sources(root: &Path, sources: &mut Vec<PathBuf>) -> Result<(), SyncError> {
    let basename = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| unsafe_path(root, "has an unsafe file name"))?;

    let candidate = root.join(format!("{basename}.md"));
    match fs::symlink_metadata(&candidate) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(unsafe_path(&candidate, "is a symlink"));
        }
        Ok(metadata) if metadata.is_file() => sources.push(candidate),
        Ok(_) => return Err(unsafe_path(&candidate, "is not a regular file")),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(SyncError::Io(candidate.clone(), error));
        }
        Err(_) => {}
    }

    let mut entries = read_dir_paths(root)?;
    entries.sort();
    for entry in entries {
        let metadata = fs::symlink_metadata(&entry).map_err(|error| SyncError::Io(entry.clone(), error))?;
        if metadata.file_type().is_symlink() {
            return Err(unsafe_path(&entry, "is a symlink"));
        }
        if metadata.is_dir() {
            collect_sources(&entry, sources)?;
        }
    }

    Ok(())
}

fn read_dir_paths(path: &Path) -> Result<Vec<PathBuf>, SyncError> {
    let entries = fs::read_dir(path).map_err(|error| SyncError::Io(path.to_path_buf(), error))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| SyncError::Io(path.to_path_buf(), error))?;
        paths.push(entry.path());
    }
    Ok(paths)
}

fn validate_tree_root(root: &Path) -> Result<(), SyncError> {
    if root.as_os_str().is_empty() {
        return Err(unsafe_path(root, "is empty"));
    }
    if root
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(unsafe_path(root, "contains parent directory components"));
    }

    let metadata = fs::symlink_metadata(root).map_err(|error| SyncError::Io(root.to_path_buf(), error))?;
    if metadata.file_type().is_symlink() {
        return Err(unsafe_path(root, "is a symlink"));
    }
    if !metadata.is_dir() {
        return Err(unsafe_path(root, "is not a directory"));
    }
    Ok(())
}

struct ParsedFrontmatter {
    name: String,
    description: String,
    tools: Vec<String>,
    model: String,
}

fn parse_frontmatter(source: &Path, frontmatter: &str) -> Result<ParsedFrontmatter, SyncError> {
    let mut name = None;
    let mut description = None;
    let mut tools = None;
    let mut model = None;
    let mut is_in_tools = false;

    for line in frontmatter.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if is_in_tools {
            if let Some(item) = trimmed.strip_prefix('-') {
                let item = parse_tool_item(source, item)?;
                tools.get_or_insert_with(Vec::new).push(item);
                continue;
            }
            is_in_tools = false;
        }

        let Some((key, raw_value)) = trimmed.split_once(':') else {
            return Err(invalid(source, format!("frontmatter line {line:?} is malformed")));
        };
        let key = key.trim();
        let value = raw_value.trim_start();
        match key {
            "name" => set_field(source, &mut name, "name", value)?,
            "description" => set_field(source, &mut description, "description", value)?,
            "model" => set_field(source, &mut model, "model", value)?,
            "tools" => {
                if tools.is_some() {
                    return Err(invalid(source, "frontmatter tools is duplicated"));
                }
                if value.is_empty() {
                    tools = Some(Vec::new());
                    is_in_tools = true;
                } else {
                    tools = Some(parse_tools_inline(source, value)?);
                }
            }
            _ => {}
        }
    }

    Ok(ParsedFrontmatter {
        name: name.ok_or_else(|| invalid(source, "frontmatter has no name"))?,
        description: description.ok_or_else(|| invalid(source, "frontmatter has no description"))?,
        tools: tools.ok_or_else(|| invalid(source, "frontmatter has no tools"))?,
        model: model.ok_or_else(|| invalid(source, "frontmatter has no model"))?,
    })
}

fn set_field(
    source: &Path,
    slot: &mut Option<String>,
    field: &str,
    value: &str,
) -> Result<(), SyncError> {
    if slot.is_some() {
        return Err(invalid(source, format!("frontmatter {field} is duplicated")));
    }
    *slot = Some(parse_scalar_field(source, field, value)?);
    Ok(())
}

fn parse_scalar_field(source: &Path, field: &str, value: &str) -> Result<String, SyncError> {
    let value = parse_scalar(value);
    if value.is_empty() {
        return Err(invalid(source, format!("frontmatter has no {field}")));
    }
    if field == "model" && value.starts_with('-') {
        return Err(invalid(source, "frontmatter model must not start with a hyphen"));
    }
    Ok(value)
}

fn parse_name(source: &Path, value: &str) -> Result<String, SyncError> {
    let name = parse_scalar_field(source, "name", value)?;
    if !is_safe_agent_name(&name) {
        return Err(invalid(source, format!("frontmatter name {name} cannot name an output file")));
    }
    Ok(name)
}

fn parse_tools(source: &Path, tools: Vec<String>) -> Result<Vec<String>, SyncError> {
    if tools.is_empty() {
        return Err(invalid(source, "frontmatter tools list is empty"));
    }
    Ok(tools)
}

fn parse_tools_inline(source: &Path, value: &str) -> Result<Vec<String>, SyncError> {
    let value = parse_scalar(value);
    let value = value
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(value.as_str());
    let mut tools = Vec::new();
    for item in value.split(',') {
        let item = parse_tool_item(source, item)?;
        tools.push(item);
    }
    parse_tools(source, tools)
}

fn parse_tool_item(source: &Path, value: &str) -> Result<String, SyncError> {
    let item = parse_scalar(value);
    if item.is_empty() {
        return Err(invalid(source, "frontmatter tools list is malformed"));
    }
    if item.starts_with('-') {
        return Err(invalid(source, "frontmatter tools list is malformed"));
    }
    Ok(item)
}

fn parse_scalar(value: &str) -> String {
    let value = value.trim();
    let quoted = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|rest| rest.strip_suffix('\'')));
    quoted.unwrap_or(value).to_string()
}

fn split_frontmatter(text: &str) -> Option<(usize, usize, usize)> {
    let mut offset = 0;
    let mut lines = text.split_inclusive('\n');
    let first = lines.next()?;
    if trim_line_end(first) != "---" {
        return None;
    }
    offset += first.len();
    let frontmatter_start = offset;

    for line in lines {
        let line_start = offset;
        offset += line.len();
        if trim_line_end(line) == "---" {
            return Some((frontmatter_start, line_start, offset));
        }
    }

    None
}

fn trim_line_end(line: &str) -> &str {
    line.trim_end_matches('\n').trim_end_matches('\r')
}

fn is_safe_agent_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(char::is_control)
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(':')
        && name != "."
        && name != ".."
        && Path::new(name).file_name() == Some(OsStr::new(name))
}

fn invalid(source: &Path, detail: impl Into<String>) -> SyncError {
    SyncError::ManifestInvalid(format!("agent file {} {}", source.display(), detail.into()))
}

fn unsafe_path(path: &Path, detail: &str) -> SyncError {
    SyncError::ManifestInvalid(format!("agent path {} is unsafe: {detail}", path.display()))
}

fn normalize_tool(tool: &str) -> Option<&'static str> {
    match tool.trim().to_ascii_lowercase().as_str() {
        "read" => Some("read"),
        "grep" => Some("grep"),
        "glob" => Some("find"),
        "find" => Some("find"),
        "bash" => Some("bash"),
        "write" => Some("write"),
        "edit" => Some("edit"),
        "websearch" | "web_search" => Some("web_search"),
        "webfetch" | "fetch_content" => Some("fetch_content"),
        _ => None,
    }
}

fn resolve_model_alias(model: &str, source: &Path) -> Result<String, SyncError> {
    match model {
        "opus" => Ok("claude-opus-4-7".to_owned()),
        "sonnet" => Ok("claude-sonnet-4-5".to_owned()),
        model if model.starts_with("claude-") => Ok(model.to_owned()),
        _ => Err(invalid(
            source,
            format!("frontmatter model {model} is unsupported"),
        )),
    }
}

fn pi_subagents_builtins() -> &'static [&'static str] {
    &["planner", "reviewer", "scout", "worker"]
}

fn render_pi_markdown(agent: &PiAgent) -> String {
    let mut rendered = String::new();
    rendered.push_str("---\n");
    rendered.push_str("name: ");
    rendered.push_str(&quote_scalar(&agent.name));
    rendered.push('\n');
    rendered.push_str("description: ");
    rendered.push_str(&quote_scalar(&agent.description));
    rendered.push('\n');
    rendered.push_str("tools:\n");
    for tool in &agent.tools {
        rendered.push_str("  - ");
        rendered.push_str(tool);
        rendered.push('\n');
    }
    rendered.push_str("model: ");
    rendered.push_str(&quote_scalar(&agent.model));
    rendered.push_str("\n---\n");
    rendered.push_str(&agent.prompt);
    rendered
}

fn quote_scalar(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn accept_exact_destination(destination: &Path, rendered: &[u8]) -> Result<(), SyncError> {
    let metadata = match fs::symlink_metadata(destination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(SyncError::DestinationCollision(destination.to_path_buf()));
        }
        Err(error) => return Err(SyncError::Io(destination.to_path_buf(), error)),
    };
    if !metadata.is_file() {
        return Err(SyncError::DestinationCollision(destination.to_path_buf()));
    }

    let observed = fs::read(destination)
        .map_err(|error| SyncError::Io(destination.to_path_buf(), error))?;
    if observed != rendered
        || !matches!(fs::symlink_metadata(destination), Ok(metadata) if metadata.is_file())
    {
        return Err(SyncError::DestinationCollision(destination.to_path_buf()));
    }
    Ok(())
}

fn write_verified(destination: &Path, rendered: &[u8]) -> Result<(), SyncError> {
    let parent = destination
        .parent()
        .ok_or_else(|| unsafe_path(destination, "has no parent directory"))?;
    fs::create_dir_all(parent).map_err(|error| SyncError::Io(parent.to_path_buf(), error))?;

    let (temp, mut file) = loop {
        let temp = verified_temp_path(destination);
        match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => break (temp, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(SyncError::Io(temp, error)),
        }
    };
    let cleanup = |path: &Path| {
        let _ = fs::remove_file(path);
    };

    if let Err(error) = file.write_all(rendered) {
        drop(file);
        cleanup(&temp);
        return Err(SyncError::Io(temp, error));
    }
    drop(file);

    let temp_bytes = fs::read(&temp).map_err(|error| {
        cleanup(&temp);
        SyncError::Io(temp.clone(), error)
    })?;
    if temp_bytes != rendered {
        cleanup(&temp);
        return Err(SyncError::StaleState(
            temp.clone(),
            "temporary render does not match requested content".to_owned(),
        ));
    }

    if let Err(error) = fs::hard_link(&temp, destination) {
        cleanup(&temp);
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            return accept_exact_destination(destination, rendered);
        }
        return Err(SyncError::Io(destination.to_path_buf(), error));
    }

    let destination_bytes = fs::read(destination).map_err(|error| {
        cleanup(&temp);
        SyncError::Io(destination.to_path_buf(), error)
    })?;
    if destination_bytes != rendered {
        cleanup(&temp);
        return Err(SyncError::StaleState(
            destination.to_path_buf(),
            "rendered file does not match requested content".to_owned(),
        ));
    }

    fs::remove_file(&temp).map_err(|error| SyncError::Io(temp, error))?;
    Ok(())
}

fn verified_temp_path(destination: &Path) -> PathBuf {
    let id = TEMP_WRITE_ID.fetch_add(1, Ordering::Relaxed);
    let stem = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("agent");
    destination.with_file_name(format!(".{stem}.tmp-{}-{id}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    fn fixture_dir(name: &str) -> PathBuf {
        let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("tool-sync-pi-agent-{name}-{id}-{}", std::process::id()))
    }

    fn write_agent(dir: &Path, name: &str, frontmatter: &str, body: &str) -> PathBuf {
        let agent_dir = dir.join(name);
        fs::create_dir_all(&agent_dir).expect("create agent dir");
        let path = agent_dir.join(format!("{name}.md"));
        fs::write(&path, format!("---\n{frontmatter}---\n{body}")).expect("write agent");
        path
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root")
    }

    fn agent_file(name: &str) -> PathBuf {
        repo_root().join(format!("agents/{0}/{0}.md", name))
    }

    fn prompt_hash(prompt: &str) -> String {
        const OFFSET: u64 = 0xcbf29ce484222325;
        const PRIME: u64 = 0x100000001b3;
        let mut hash = OFFSET;
        for byte in prompt.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
        format!("{hash:016x}")
    }

    fn expected_render(agent: &PiAgent) -> String {
        let mut rendered = String::from("---\n");
        rendered.push_str("name: ");
        rendered.push_str(&quote_scalar(&agent.name));
        rendered.push('\n');
        rendered.push_str("description: ");
        rendered.push_str(&quote_scalar(&agent.description));
        rendered.push('\n');
        rendered.push_str("tools:\n");
        for tool in &agent.tools {
            rendered.push_str("  - ");
            rendered.push_str(tool);
            rendered.push('\n');
        }
        rendered.push_str("model: ");
        rendered.push_str(&quote_scalar(&agent.model));
        rendered.push_str("\n---\n");
        rendered.push_str(&agent.prompt);
        rendered
    }

    struct RepoAgentCase {
        name: &'static str,
        description: &'static str,
        source_tools: &'static [&'static str],
        tools: &'static [&'static str],
        model: &'static str,
        prompt_hash: &'static str,
    }

    fn repository_agent_cases() -> [RepoAgentCase; 6] {
        [
            RepoAgentCase {
                name: "anchor-verifier",
                description: r"Use to verify ONE worker's finished work product when the dispatch names work_product_paths, verify_command, and rubric — runs the verification command in fresh context and grades every rubric item on anchors (executed command output, file plus line on disk), never on the worker's self-report. Skip for judging authored artifacts over their accumulated logs and votes (ai-author's blind judge owns that), for open-ended diff review with no dispatch rubric (a code-review shape), for any ask to fix or patch what fails, and for verifying work it produced itself.",
                source_tools: &["Read", "Grep", "Glob", "Bash"],
                tools: &["read", "grep", "find", "bash"],
                model: "claude-opus-4-7",
                prompt_hash: "4c438693c3d89509",
            },
            RepoAgentCase {
                name: "code-reviewer",
                description: r"Use for a fresh-context review of a diff or branch — dispatch with repo_path plus an optional diff_range; returns ranked Critical / Warnings / Suggestions findings, each anchored to file:line with the command that proves it, checkable by the dispatcher without redoing the review. Skip when anything should be fixed, patched, or committed (this agent never modifies a file — that is a different agent's job), when a builder wants a self-check inside its own session (a checker sharing the worker's context is self-verification — dispatch fresh instead), and when there is no diff to review (whole-repo audits are not this role).",
                source_tools: &["Read", "Grep", "Glob", "Bash"],
                tools: &["read", "grep", "find", "bash"],
                model: "claude-opus-4-7",
                prompt_hash: "66fd0c3fede8b9af",
            },
            RepoAgentCase {
                name: "debugger",
                description: r"Use when a dispatch names a failing repro(reproduction) — a command plus expected vs actual output — and the job is to root-cause the failure and apply a minimal fix. Skip when the dispatch carries no repro command (it reports invalid-dispatch and stops, never invents one), for code review or test authoring (different roles), for refactors beyond the minimal fix, and for grading its own fix (a fresh checker owns the verdict).",
                source_tools: &["Read", "Edit", "Bash", "Grep", "Glob"],
                tools: &["read", "edit", "bash", "grep", "find"],
                model: "claude-sonnet-4-5",
                prompt_hash: "b6c42a002c783e2a",
            },
            RepoAgentCase {
                name: "maestro-tester",
                description: r"Use to turn ONE flow objective into a Maestro YAML flow run against an already-booted simulator/emulator with the app already installed — writes or repairs the flow file, runs `maestro test --format junit`, and returns a verdict anchored to the junit report on disk; dispatch carries app_id, flow_objective, flows_dir. Skip for web-page testing (browser tools own it), for building/installing the app or booting devices (XcodeBuildMCP owns those), for exploratory what-is-on-screen poking that leaves no flow artifact, and for grading its own past runs.",
                source_tools: &["Read", "Write", "Edit", "Bash", "Glob", "Grep"],
                tools: &["read", "write", "edit", "bash", "find", "grep"],
                model: "claude-sonnet-4-5",
                prompt_hash: "4ec0c90df11c8cb5",
            },
            RepoAgentCase {
                name: "spec-tester",
                description: r"Use to execute natural-language test cases (mode confirm) or one attack-angle charter (mode break) against a runnable SUT(system under test) through its drive harness, fresh context, returning per-case verdicts and debugger-ready failures (repro_command + expected + actual); dispatch carries mode, drive_matrix, scratch_dir, and cases or angle_charter. Skip for mobile YAML flow runs (maestro-tester owns those), for verifying a worker's product against a named verify command (anchor-verifier), for any ask to fix what fails, and for grading its own past runs.",
                source_tools: &["Read", "Write", "Bash", "Grep", "Glob"],
                tools: &["read", "write", "bash", "grep", "find"],
                model: "claude-sonnet-4-5",
                prompt_hash: "88505d7232ce401b",
            },
            RepoAgentCase {
                name: "web-research-summarizer",
                description: r"Use to fan out over external documentation and web sources with fresh context and return a 1000-2000 token cited findings block — every claim carrying a source URL(Uniform Resource Locator) plus date, stale sources flagged — so the parent never ingests raw pages; dispatch carries objective, boundaries, source guidance, recency. Skip for repository or codebase search (built-in Explore owns it), for a single-fact lookup the parent settles in a few tool calls, and for anything that writes files.",
                source_tools: &["WebSearch", "WebFetch", "Read"],
                tools: &["web_search", "fetch_content", "read"],
                model: "claude-sonnet-4-5",
                prompt_hash: "027b0df6454d4d81",
            },
        ]
    }

    #[test]
    fn pi_agent_renders_repository_agents_with_exact_metadata_and_prompt_hashes() {
        let fixture_root = fixture_dir("render-repository");
        let destination_root = fixture_root.join("rendered");
        fs::create_dir_all(&destination_root).expect("create destination root");

        for case in repository_agent_cases() {
            let source = agent_file(case.name);
            let source_before = fs::read_to_string(&source).expect("read source");
            let parsed = parse(&source).expect("parse source");
            assert_eq!(parsed.name, case.name, "{name}", name = case.name);
            assert_eq!(parsed.description, case.description, "{name}", name = case.name);
            assert_eq!(parsed.tools, case.source_tools, "{name}", name = case.name);
            assert_eq!(parsed.model, if case.model == "claude-opus-4-7" { "opus" } else { "sonnet" }, "{name}", name = case.name);
            assert_eq!(prompt_hash(&parsed.prompt), case.prompt_hash, "{name}", name = case.name);

            let adapted = adapt(parsed.clone()).expect("adapt source");
            assert_eq!(adapted.name, case.name, "{name}", name = case.name);
            assert_eq!(adapted.description, case.description, "{name}", name = case.name);
            assert_eq!(adapted.tools, case.tools, "{name}", name = case.name);
            assert_eq!(adapted.model, case.model, "{name}", name = case.name);
            assert_eq!(adapted.prompt, parsed.prompt, "{name}", name = case.name);

            let destination = destination_root.join(format!("{}.md", case.name));
            render(&source, &destination).expect("render source");

            assert_eq!(fs::read_to_string(&source).expect("source unchanged"), source_before);
            let rendered = fs::read_to_string(&destination).expect("read rendered file");
            assert_eq!(rendered, expected_render(&adapted), "{name}", name = case.name);

            let round_trip = parse(&destination).expect("parse rendered file");
            assert_eq!(round_trip.name, case.name, "{name}", name = case.name);
            assert_eq!(round_trip.description, case.description, "{name}", name = case.name);
            assert_eq!(round_trip.tools, case.tools, "{name}", name = case.name);
            assert_eq!(round_trip.model, case.model, "{name}", name = case.name);
            assert_eq!(round_trip.prompt, parsed.prompt, "{name}", name = case.name);
            assert_eq!(prompt_hash(&round_trip.prompt), case.prompt_hash, "{name}", name = case.name);
        }
    }

    #[cfg(unix)]
    #[test]
    fn render_preserves_colliding_files_symlinks_and_directories() {
        use std::os::unix::fs::symlink;

        let root = fixture_dir("render-collisions");
        fs::create_dir_all(&root).expect("create fixture root");
        let source = agent_file("anchor-verifier");
        let unrelated = root.join("unrelated");
        fs::write(&unrelated, b"unrelated bytes\0").expect("write unrelated file");

        let foreign_file = root.join("foreign.md");
        fs::write(&foreign_file, b"foreign bytes\0").expect("write foreign file");
        assert!(matches!(
            render(&source, &foreign_file),
            Err(SyncError::DestinationCollision(path)) if path == foreign_file
        ));
        assert_eq!(fs::read(&foreign_file).expect("read foreign file"), b"foreign bytes\0");

        let target = root.join("target.md");
        fs::write(&target, rendered_bytes(&source).expect("rendered bytes"))
            .expect("write symlink target");
        let foreign_symlink = root.join("foreign-link.md");
        symlink(&target, &foreign_symlink).expect("write foreign symlink");
        assert!(matches!(
            render(&source, &foreign_symlink),
            Err(SyncError::DestinationCollision(path)) if path == foreign_symlink
        ));
        assert_eq!(fs::read_link(&foreign_symlink).expect("read symlink target"), target);

        let foreign_directory = root.join("foreign-directory.md");
        fs::create_dir(&foreign_directory).expect("create foreign directory");
        fs::write(foreign_directory.join("owned"), b"directory bytes\0")
            .expect("write directory contents");
        assert!(matches!(
            render(&source, &foreign_directory),
            Err(SyncError::DestinationCollision(path)) if path == foreign_directory
        ));
        assert_eq!(
            fs::read(foreign_directory.join("owned")).expect("read directory contents"),
            b"directory bytes\0"
        );
        assert_eq!(fs::read(&unrelated).expect("read unrelated file"), b"unrelated bytes\0");
    }

    #[test]
    fn pi_agent_reports_builtin_overlaps_without_disabling_them() {
        let current = repository_agent_cases()
            .into_iter()
            .map(|case| agent_file(case.name))
            .collect::<Vec<_>>();
        assert!(builtin_overlaps(&current).expect("current agents").is_empty());

        let root = fixture_dir("builtin-overlaps");
        let scout = write_agent(
            &root,
            "scout",
            "name: scout\ndescription: Fast recon\ntools: Read\nmodel: sonnet\n",
            "scout body\n",
        );
        let reviewer = write_agent(
            &root,
            "reviewer",
            "name: reviewer\ndescription: Code review\ntools: Read\nmodel: sonnet\n",
            "reviewer body\n",
        );
        let overlaps = builtin_overlaps(&[scout, reviewer]).expect("overlap report");
        assert_eq!(overlaps, ["reviewer", "scout"]);
    }

    #[test]
    fn pi_agent_rejects_unknown_tools_and_unresolved_model_aliases() {
        let root = fixture_dir("adapter-rejects");
        let unsupported_tool = write_agent(
            &root,
            "unsupported-tool",
            "name: unsupported-tool\ndescription: bad tool\ntools: Read, Fly\nmodel: sonnet\n",
            "body\n",
        );
        let unsupported_tool_error = adapt(parse(&unsupported_tool).expect("parse unsupported tool"))
            .expect_err("unsupported tool must fail");
        assert!(unsupported_tool_error.to_string().contains("unsupported"));

        let unresolved_model = write_agent(
            &root,
            "unresolved-model",
            "name: unresolved-model\ndescription: bad model\ntools: Read\nmodel: haiku\n",
            "body\n",
        );
        let unresolved_model_error = adapt(parse(&unresolved_model).expect("parse unresolved model"))
            .expect_err("unresolved model must fail");
        assert!(unresolved_model_error.to_string().contains("unsupported"));
    }

    #[test]
    fn discover_returns_sorted_agent_paths() {
        let root = fixture_dir("discover-sorted");
        let charlie = write_agent(
            &root,
            "charlie",
            "name: charlie\ndescription: third\ntools: Read\nmodel: opus\n",
            "charlie body\n",
        );
        let alpha = write_agent(
            &root,
            "alpha",
            "name: alpha\ndescription: first\ntools: Read\nmodel: sonnet\n",
            "alpha body\n",
        );
        let bravo = write_agent(
            &root,
            "bravo",
            "name: bravo\ndescription: second\ntools: Read\nmodel: haiku\n",
            "bravo body\n",
        );
        fs::create_dir_all(root.join("bravo/evals")).expect("create eval dir");
        fs::write(root.join("bravo/evals/rubric.md"), "no frontmatter\n").expect("seed rubric");

        let paths = discover(&root).expect("discover succeeds");
        assert_eq!(paths, vec![alpha, bravo, charlie]);
    }

    #[test]
    fn discover_rejects_duplicate_names() {
        let root = fixture_dir("discover-duplicate");
        write_agent(
            &root,
            "alpha",
            "name: shared\ndescription: first\ntools: Read\nmodel: opus\n",
            "alpha body\n",
        );
        write_agent(
            &root,
            "bravo",
            "name: shared\ndescription: second\ntools: Read\nmodel: sonnet\n",
            "bravo body\n",
        );

        let error = discover(&root).expect_err("duplicate names must fail");
        assert!(error.to_string().contains("duplicate agent name shared"));
    }

    #[test]
    fn discover_rejects_unsafe_paths() {
        let root = fixture_dir("discover-unsafe");
        fs::create_dir_all(&root).expect("create fixture root");
        let error = discover(&root.join("..")).expect_err("unsafe root must fail");
        assert!(error.to_string().contains("unsafe"));
    }

    #[test]
    fn parse_preserves_prompt_body_and_required_frontmatter() {
        let root = fixture_dir("parse-body");
        let path = write_agent(
            &root,
            "anchor-verifier",
            "name: anchor-verifier\ndescription: Use to verify one worker.\ntools: Read, Grep, Glob, Bash\nmodel: opus\n",
            "\nYou keep the prompt body exact.\n\n- no edits\n---\nstill body\n",
        );

        let agent = parse(&path).expect("parse succeeds");
        assert_eq!(agent.path, path);
        assert_eq!(agent.name, "anchor-verifier");
        assert_eq!(agent.description, "Use to verify one worker.");
        assert_eq!(agent.tools, ["Read", "Grep", "Glob", "Bash"]);
        assert_eq!(agent.model, "opus");
        assert_eq!(
            agent.prompt,
            "\nYou keep the prompt body exact.\n\n- no edits\n---\nstill body\n"
        );
    }

    #[test]
    fn parse_accepts_block_tool_lists() {
        let root = fixture_dir("parse-tools");
        let path = write_agent(
            &root,
            "spec-tester",
            "name: spec-tester\ndescription: Use to execute tests.\ntools:\n  - Read\n  - Bash\nmodel: sonnet\n",
            "You execute tests.\n",
        );

        let agent = parse(&path).expect("parse succeeds");
        assert_eq!(agent.tools, ["Read", "Bash"]);
    }

    #[test]
    fn parse_rejects_missing_fields_and_malformed_tools() {
        let root = fixture_dir("parse-invalid");
        fs::create_dir_all(&root).expect("create fixture root");
        let missing_description = root.join("missing-description.md");
        fs::write(
            &missing_description,
            "---\nname: missing-description\ntools: Read\nmodel: opus\n---\nbody\n",
        )
        .expect("write missing description");
        assert!(parse(&missing_description).expect_err("missing description fails").to_string().contains("frontmatter has no description"));

        let malformed_tools = root.join("malformed-tools.md");
        fs::write(
            &malformed_tools,
            "---\nname: malformed-tools\ndescription: bad tools\ntools: Read, , Bash\nmodel: opus\n---\nbody\n",
        )
        .expect("write malformed tools");
        assert!(parse(&malformed_tools).expect_err("malformed tools fail").to_string().contains("frontmatter tools list is malformed"));

        let hyphen_model = root.join("hyphen-model.md");
        fs::write(
            &hyphen_model,
            "---\nname: hyphen-model\ndescription: bad model\ntools: Read\nmodel: -bad\n---\nbody\n",
        )
        .expect("write hyphen model");
        assert!(parse(&hyphen_model).expect_err("hyphen model fails").to_string().contains("must not start with a hyphen"));
    }

    #[test]
    fn parse_rejects_non_text_input() {
        let root = fixture_dir("parse-binary");
        let path = root.join("binary.md");
        fs::create_dir_all(&root).expect("create root");
        fs::write(
            &path,
            [
                &b"---\nname: binary\ndescription: bad input\ntools: Read\nmodel: opus\n---\n"[..],
                &b"\xff\xfe\x00"[..],
            ]
            .concat(),
        )
        .expect("write binary input");

        assert!(parse(&path).is_err());
    }
}
