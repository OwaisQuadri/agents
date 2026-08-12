use std::path::Path;

use crate::drift::Change;
use crate::error::SyncError;

pub struct AgentDef {
    pub name: String,
    pub description: String,
    pub developer_instructions: String,
    pub model: Option<String>,
}

/// Parses one Claude agent markdown file into a flat agent definition.
/// Takes the .md path; returns AgentDef with frontmatter name, description,
/// and model, and the body as developer_instructions.
///
/// # Errors
/// Io on read failure; ManifestInvalid on missing frontmatter or a missing
/// name or description key.
pub fn parse_agent_md(path: &Path) -> Result<AgentDef, SyncError> {
    let _ = path;
    unimplemented!()
}

/// Renders an agent definition as a Codex agent TOML document.
/// Takes the definition; returns the TOML text.
///
/// # Errors
/// none
pub fn render_agent_toml(def: &AgentDef) -> String {
    let _ = def;
    unimplemented!()
}

/// Converges the Codex agents dir onto the repo agents dir.
/// Renders every agents/<name>/<name>.md into <dest>/<name>.toml and removes
/// dest files whose source agent no longer exists.
/// Takes the source dir, dest dir, and dry-run flag; returns the changes.
///
/// # Errors
/// Io, ManifestInvalid, BackupFailed, VerifyFailed.
pub fn sync_agents(src_dir: &Path, dest_dir: &Path, is_dry_run: bool) -> Result<Vec<Change>, SyncError> {
    let _ = (src_dir, dest_dir, is_dry_run);
    unimplemented!()
}
