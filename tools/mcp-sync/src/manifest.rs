use std::path::Path;

use crate::error::SyncError;

pub struct Manifest {
    pub servers: Vec<ServerEntry>,
}

pub struct ServerEntry {
    pub name: String,
    pub transport: Transport,
    pub scope: ToolScope,
}

pub enum Transport {
    Stdio(StdioSpec),
    Remote(RemoteSpec),
}

pub struct StdioSpec {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<String>,
}

pub struct RemoteSpec {
    pub url: String,
    pub bearer_token_env_var: Option<String>,
}

pub enum ToolScope {
    Both,
    ClaudeOnly,
    CodexOnly,
}

pub struct SyncState {
    pub claude_managed: Vec<String>,
    pub codex_managed: Vec<String>,
}

/// Loads and validates the manifest file.
/// Takes the manifest path; returns the parsed Manifest in file order.
///
/// # Errors
/// Io when the file is unreadable; ParseToml on invalid TOML;
/// ManifestInvalid on a table with both command and url, neither, or an
/// unknown tools entry.
pub fn load_manifest(path: &Path) -> Result<Manifest, SyncError> {
    let _ = path;
    // TODO(AGNT-0001.T02): body lands in the phase-13 build; contract: interfaces.md manifest section
    unimplemented!()
}

/// Loads the managed-names state file, treating an absent file as empty state.
/// Takes the state path; returns SyncState.
///
/// # Errors
/// Io on an unreadable existing file; ParseToml on invalid TOML.
pub fn load_state(path: &Path) -> Result<SyncState, SyncError> {
    let _ = path;
    // TODO(AGNT-0001.T02): body lands in the phase-13 build; contract: interfaces.md manifest section
    unimplemented!()
}

/// Writes the managed-names state file.
/// Takes the state path, the state, and the dry-run flag; returns unit.
///
/// # Errors
/// Io on write failure; VerifyFailed when the re-read does not match.
pub fn save_state(path: &Path, state: &SyncState, is_dry_run: bool) -> Result<(), SyncError> {
    let _ = (path, state, is_dry_run);
    // TODO(AGNT-0001.T02): body lands in the phase-13 build; contract: interfaces.md manifest section
    unimplemented!()
}

/// Appends adopted servers to the manifest file, format-preserving.
/// Takes the manifest path, the entries, and the dry-run flag; returns unit.
///
/// # Errors
/// Io, ParseToml, VerifyFailed, or ChangedSinceRead from the underlying
/// verified write.
pub fn append_servers(
    path: &Path,
    entries: &[ServerEntry],
    is_dry_run: bool,
) -> Result<(), SyncError> {
    let _ = (path, entries, is_dry_run);
    // TODO(AGNT-0001.T02): body lands in the phase-13 build; contract: interfaces.md manifest section
    unimplemented!()
}
