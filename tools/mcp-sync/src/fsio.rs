use std::path::{Path, PathBuf};

use crate::error::SyncError;

/// Reads a file to a string, mapping absence to None.
/// Takes the path; returns the contents or None when the file does not exist.
///
/// # Errors
/// Io on any read failure other than not-found.
pub fn read_opt(path: &Path) -> Result<Option<String>, SyncError> {
    let _ = path;
    unimplemented!()
}

/// Copies the target to a stamped backup and verifies the copy is on disk.
/// Takes the path; returns the backup path, or None when the target is absent.
///
/// # Errors
/// BackupFailed when the copy is missing after the write; Io on read failure.
pub fn backup(path: &Path) -> Result<Option<PathBuf>, SyncError> {
    let _ = path;
    unimplemented!()
}

/// Writes content atomically after proving the target is unchanged since read.
/// Compares the target against the snapshot taken at read time, backs it up,
/// writes a temp file in the same dir, renames it in, and re-reads to verify.
/// Takes the path, the new content, the read-time snapshot, and the dry-run
/// flag; returns unit. In dry-run it prints the would-write line and touches
/// nothing.
///
/// # Errors
/// ChangedSinceRead when the target no longer matches the snapshot;
/// BackupFailed from the backup step; VerifyFailed when the re-read differs;
/// Io on any filesystem failure.
pub fn write_verified(
    path: &Path,
    content: &str,
    snapshot: Option<&str>,
    is_dry_run: bool,
) -> Result<(), SyncError> {
    let _ = (path, content, snapshot, is_dry_run);
    unimplemented!()
}
