// TODO(AGNT-0008.T02): register the planner modules in the shared crate root.
use std::fmt;
use std::path::PathBuf;

/// Contains executable-tool manifest parsing and validation.
pub mod manifest;

/// Describes a failure to read or validate executable-tool configuration.
/// The variant contains the failing path or validation detail; formatting it
/// returns a user-facing message. It does not produce errors itself.
pub enum SyncError {
    Io(PathBuf, std::io::Error),
    ParseToml(PathBuf, String),
    ManifestInvalid(String),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(path, error) => write!(f, "cannot read {}: {error}", path.display()),
            Self::ParseToml(path, detail) => {
                write!(f, "{} is not valid TOML: {detail}", path.display())
            }
            Self::ManifestInvalid(detail) => write!(f, "tool manifest invalid: {detail}"),
        }
    }
}

impl fmt::Debug for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for SyncError {}
