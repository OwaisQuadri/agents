use std::path::Path;

use crate::drift::{Change, DriftRow};
use crate::error::SyncError;
use crate::manifest::{Manifest, ServerEntry, SyncState};

/// Renders one manifest entry as a Codex [mcp_servers.<name>] table.
/// Takes the entry; returns the TOML table.
///
/// # Errors
/// none
pub fn render_table(entry: &ServerEntry) -> toml_edit::Table {
    let _ = entry;
    unimplemented!()
}

/// Converges the [mcp_servers.*] tables of config.toml onto the manifest.
/// Touches only manifest-scoped names plus state-listed names to remove;
/// [projects.*], [plugins.*], comments, and unmanaged server tables survive
/// via toml_edit decor preservation.
/// Takes the config path, manifest, state, and dry-run flag; returns the
/// changes made or planned.
///
/// # Errors
/// Io, ParseToml, ChangedSinceRead, BackupFailed, VerifyFailed.
pub fn sync(
    path: &Path,
    manifest: &Manifest,
    state: &SyncState,
    is_dry_run: bool,
) -> Result<Vec<Change>, SyncError> {
    let _ = (path, manifest, state, is_dry_run);
    unimplemented!()
}

/// Reports per-server drift between the manifest and config.toml.
/// Takes the config path, manifest, and state; returns one DriftRow per
/// codex-scoped manifest server plus one per unmanaged live server.
///
/// # Errors
/// Io, ParseToml.
pub fn check(
    path: &Path,
    manifest: &Manifest,
    state: &SyncState,
) -> Result<Vec<DriftRow>, SyncError> {
    let _ = (path, manifest, state);
    unimplemented!()
}

/// Parses live Codex servers absent from the manifest into manifest entries.
/// Takes the config path and manifest; returns the adoptable entries.
///
/// # Errors
/// Io, ParseToml.
pub fn unmanaged(path: &Path, manifest: &Manifest) -> Result<Vec<ServerEntry>, SyncError> {
    let _ = (path, manifest);
    unimplemented!()
}
