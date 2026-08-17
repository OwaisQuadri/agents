use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::SyncError;

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct AgentSource {
    path: PathBuf,
    name: String,
    description: String,
    tools: Vec<String>,
    model: String,
    prompt: String,
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

/// Finds source agent definitions beneath one repository directory.
/// It takes the agent root and returns sorted Markdown paths.
///
/// # Errors
///
/// Returns `SyncError` when the directory cannot be read or contains an unsafe path.
// TODO(AGNT-0012.T05): Discover and parse source agents in stable order.
pub fn discover(root: &Path) -> Result<Vec<PathBuf>, SyncError> {
    unimplemented!()
}

/// Renders one source agent as a Pi-compatible agent definition.
/// It takes source and destination paths and returns unit after the destination is verified.
///
/// # Errors
///
/// Returns `SyncError` for unreadable input, invalid frontmatter, unsupported fields, or filesystem failures.
// TODO(AGNT-0012.T06): Normalize metadata and preserve the prompt body.
pub fn render(source: &Path, destination: &Path) -> Result<(), SyncError> {
    unimplemented!()
}

/// Finds project agent names that overlap bundled pi-subagents roles.
/// It takes source agent paths and returns sorted canonical role names.
///
/// # Errors
///
/// Returns `SyncError` when a source cannot be read or parsed.
pub fn builtin_overlaps(sources: &[PathBuf]) -> Result<Vec<String>, SyncError> {
    unimplemented!()
}

fn parse(source: &Path) -> Result<AgentSource, SyncError> {
    unimplemented!()
}

fn adapt(source: AgentSource) -> Result<PiAgent, SyncError> {
    unimplemented!()
}
