use std::path::PathBuf;

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
}
