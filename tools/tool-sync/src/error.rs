use std::fmt;
use std::path::PathBuf;
use std::process::ExitStatus;

/// Exposes plan rendering and application.
/// It takes planned actions and a dry-run selection, returns rendered text or unit, and reports apply failures.
pub mod apply;
/// Contains executable-tool manifest parsing and validation.
pub mod manifest;
/// Exposes ordered installation action data.
/// It takes no module-level inputs, exports plan values, and cannot fail.
pub mod plan;
/// Exposes installation planning without changing managed state.
/// It takes a manifest and context, returns a plan, and reports unsafe state as `SyncError`.
pub mod planner;

/// Describes a failure to plan, render, or apply executable-tool synchronization.
/// Its variants contain the relevant path, process, status, or validation detail and formatting returns a user-facing message.
/// Constructing and formatting this value do not themselves fail.
pub enum SyncError {
    Io(PathBuf, std::io::Error),
    ParseToml(PathBuf, String),
    ManifestInvalid(String),
    ProcessStart {
        program: String,
        working_directory: PathBuf,
        error: std::io::Error,
    },
    GitFailed {
        repository: PathBuf,
        status: ExitStatus,
    },
    InstallerFailed {
        tool: String,
        status: ExitStatus,
    },
    StaleState(PathBuf, String),
    DestinationCollision(PathBuf),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(path, error) => write!(f, "cannot read {}: {error}", path.display()),
            Self::ParseToml(path, detail) => {
                write!(f, "{} is not valid TOML: {detail}", path.display())
            }
            Self::ManifestInvalid(detail) => write!(f, "tool manifest invalid: {detail}"),
            Self::ProcessStart {
                program,
                working_directory,
                error,
            } => write!(
                f,
                "cannot start {program} in {}: {error}",
                working_directory.display()
            ),
            Self::GitFailed { repository, status } => {
                write!(f, "Git failed in {} with {status}", repository.display())
            }
            Self::InstallerFailed { tool, status } => {
                write!(f, "installer for {tool} failed with {status}")
            }
            Self::StaleState(path, detail) => {
                write!(f, "state changed at {}: {detail}", path.display())
            }
            Self::DestinationCollision(path) => {
                write!(
                    f,
                    "destination {} collides with a non-symlink",
                    path.display()
                )
            }
        }
    }
}

impl fmt::Debug for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for SyncError {}
