use std::path::PathBuf;

pub enum SyncError {
    Io(PathBuf, std::io::Error),
    ParseToml(PathBuf, String),
    ParseJson(PathBuf, String),
    ManifestInvalid(String),
    BackupFailed(PathBuf),
    VerifyFailed(PathBuf, String),
    ChangedSinceRead(PathBuf),
}
// TODO(AGNT-0001.T17): Display and std::error::Error impls land in the phase-13 build; contract: interfaces.md + ux.md Error state
