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
