use std::path::{Path, PathBuf};

pub enum Mode {
    Apply,
    Check,
    Adopt,
}

pub struct CliArgs {
    pub mode: Mode,
    pub is_dry_run: bool,
    pub targets: Targets,
}

pub struct Targets {
    pub manifest_path: PathBuf,
    pub state_path: PathBuf,
    pub claude_json_path: PathBuf,
    pub codex_toml_path: PathBuf,
    pub codex_hooks_json_path: PathBuf,
    pub agents_src_dir: PathBuf,
    pub codex_agents_dir: PathBuf,
    pub hook_command: PathBuf,
}

/// Parses process arguments into a mode, the dry-run flag, and target paths.
/// Takes the argument iterator after argv0; returns the parsed CliArgs.
///
/// # Errors
/// Returns a usage message for an unknown flag, a missing flag value, or an
/// unknown mode word.
pub fn parse_args(args: impl Iterator<Item = String>) -> Result<CliArgs, String> {
    let _ = args;
    // TODO(AGNT-0001.T08): body lands in the phase-13 build; contract: interfaces.md cli section
    unimplemented!()
}

impl Targets {
    /// Builds the default target set from the repo root and the home dir.
    /// Takes both roots; returns Targets with every path filled.
    ///
    /// # Errors
    /// none
    pub fn from_roots(repo_root: &Path, home: &Path) -> Targets {
        let _ = (repo_root, home);
        // TODO(AGNT-0001.T08): body lands in the phase-13 build; contract: interfaces.md cli section
    unimplemented!()
    }
}
