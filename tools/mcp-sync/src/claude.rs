use std::path::Path;

use crate::drift::{Change, DriftRow};
use crate::error::SyncError;
use crate::manifest::{Manifest, ServerEntry, SyncState};

/// Renders one manifest entry as a Claude mcpServers JSON value.
/// Takes the entry; returns the JSON object for that server.
///
/// # Errors
/// none
pub fn render_entry(entry: &ServerEntry) -> serde_json::Value {
    let _ = entry;
    unimplemented!()
}

/// Converges the mcpServers key of the Claude config onto the manifest.
/// Touches only manifest-scoped names plus state-listed names to remove;
/// every other key in the file survives byte-for-byte modulo reserialization.
/// Takes the config path, manifest, state, and dry-run flag; returns the
/// changes made or planned.
///
/// # Errors
/// Io, ParseJson, ChangedSinceRead, BackupFailed, VerifyFailed.
pub fn sync(
    path: &Path,
    manifest: &Manifest,
    state: &SyncState,
    is_dry_run: bool,
) -> Result<Vec<Change>, SyncError> {
    let _ = (path, manifest, state, is_dry_run);
    unimplemented!()
}

/// Reports per-server drift between the manifest and the Claude config.
/// Takes the config path, manifest, and state; returns one DriftRow per
/// claude-scoped manifest server plus one per unmanaged live server.
///
/// # Errors
/// Io, ParseJson.
pub fn check(
    path: &Path,
    manifest: &Manifest,
    state: &SyncState,
) -> Result<Vec<DriftRow>, SyncError> {
    let _ = (path, manifest, state);
    unimplemented!()
}

/// Parses live Claude servers absent from the manifest into manifest entries.
/// Takes the config path and manifest; returns the adoptable entries.
///
/// # Errors
/// Io, ParseJson.
pub fn unmanaged(path: &Path, manifest: &Manifest) -> Result<Vec<ServerEntry>, SyncError> {
    let _ = (path, manifest);
    unimplemented!()
}
