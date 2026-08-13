use std::env;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
usage: mcp-sync [--check | adopt] [--dry-run] [path flags]

modes
  (default)   converge the live Claude and Codex configs onto the manifest
  --check     read-only drift readout; exit 1 on any drift
  adopt       fold unmanaged live servers into the manifest

flags
  --dry-run              print the plan and write nothing
  --manifest <path>      default <repo>/config/mcp-servers.toml
  --state <path>         default <repo>/config/mcp-sync-state.toml
  --claude-json <path>   default $HOME/.claude.json
  --codex-toml <path>    default $HOME/.codex/config.toml
  --codex-hooks <path>   default $HOME/.codex/hooks.json
  --agents-src <path>    default <repo>/agents
  --codex-agents <path>  default $HOME/.codex/agents
  --hook-command <path>  default <repo>/hooks/rag-recall
  -h, --help             print this usage

<repo> is $MCP_SYNC_REPO when set, else $HOME/Documents/agents.";

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
    let mut args = args;
    let mut mode = Mode::Apply;
    let mut is_dry_run = false;
    let mut targets = Targets::from_roots(&repo_root(), &home());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => mode = Mode::Check,
            "adopt" => mode = Mode::Adopt,
            "--dry-run" => is_dry_run = true,
            "-h" | "--help" => return Err(USAGE.to_string()),
            "--manifest" => targets.manifest_path = path_value(&arg, &mut args)?,
            "--state" => targets.state_path = path_value(&arg, &mut args)?,
            "--claude-json" => targets.claude_json_path = path_value(&arg, &mut args)?,
            "--codex-toml" => targets.codex_toml_path = path_value(&arg, &mut args)?,
            "--codex-hooks" => targets.codex_hooks_json_path = path_value(&arg, &mut args)?,
            "--agents-src" => targets.agents_src_dir = path_value(&arg, &mut args)?,
            "--codex-agents" => targets.codex_agents_dir = path_value(&arg, &mut args)?,
            "--hook-command" => targets.hook_command = path_value(&arg, &mut args)?,
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    Ok(CliArgs { mode, is_dry_run, targets })
}

impl Targets {
    /// Builds the default target set from the repo root and the home dir.
    /// Takes both roots; returns Targets with every path filled.
    pub fn from_roots(repo_root: &Path, home: &Path) -> Targets {
        Targets {
            manifest_path: repo_root.join("config/mcp-servers.toml"),
            state_path: repo_root.join("config/mcp-sync-state.toml"),
            claude_json_path: home.join(".claude.json"),
            codex_toml_path: home.join(".codex/config.toml"),
            codex_hooks_json_path: home.join(".codex/hooks.json"),
            agents_src_dir: repo_root.join("agents"),
            codex_agents_dir: home.join(".codex/agents"),
            hook_command: repo_root.join("hooks/rag-recall"),
        }
    }
}

fn path_value(flag: &str, args: &mut dyn Iterator<Item = String>) -> Result<PathBuf, String> {
    match args.next() {
        Some(value) => Ok(PathBuf::from(value)),
        None => Err(format!("{flag} needs a path value\n{USAGE}")),
    }
}

fn repo_root() -> PathBuf {
    match env::var_os("MCP_SYNC_REPO") {
        Some(root) => PathBuf::from(root),
        None => home().join("Documents/agents"),
    }
}

fn home() -> PathBuf {
    match env::var_os("HOME") {
        Some(home) => PathBuf::from(home),
        None => PathBuf::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(tokens: &[&str]) -> Result<CliArgs, String> {
        parse_args(tokens.iter().map(|token| token.to_string()))
    }

    fn parsed(tokens: &[&str]) -> CliArgs {
        parse(tokens).expect("args parse")
    }

    fn usage_error(tokens: &[&str]) -> String {
        let Err(message) = parse(tokens) else {
            panic!("args unexpectedly parsed: {tokens:?}");
        };
        message
    }

    #[test]
    fn defaults_are_apply_with_env_derived_paths() {
        let args = parsed(&[]);
        assert!(matches!(args.mode, Mode::Apply));
        assert!(!args.is_dry_run);
        let expected = Targets::from_roots(&repo_root(), &home());
        assert_eq!(args.targets.manifest_path, expected.manifest_path);
        assert_eq!(args.targets.state_path, expected.state_path);
        assert_eq!(args.targets.claude_json_path, expected.claude_json_path);
        assert_eq!(args.targets.codex_toml_path, expected.codex_toml_path);
        assert_eq!(args.targets.codex_hooks_json_path, expected.codex_hooks_json_path);
        assert_eq!(args.targets.agents_src_dir, expected.agents_src_dir);
        assert_eq!(args.targets.codex_agents_dir, expected.codex_agents_dir);
        assert_eq!(args.targets.hook_command, expected.hook_command);
    }

    #[test]
    fn check_flag_selects_check_mode() {
        assert!(matches!(parsed(&["--check"]).mode, Mode::Check));
    }

    #[test]
    fn adopt_word_selects_adopt_mode() {
        assert!(matches!(parsed(&["adopt"]).mode, Mode::Adopt));
    }

    #[test]
    fn dry_run_flag_sets_is_dry_run() {
        assert!(parsed(&["--dry-run"]).is_dry_run);
        assert!(parsed(&["--check", "--dry-run"]).is_dry_run);
    }

    #[test]
    fn every_path_flag_overrides_its_default() {
        let args = parsed(&[
            "--manifest", "/m.toml",
            "--state", "/s.toml",
            "--claude-json", "/c.json",
            "--codex-toml", "/x.toml",
            "--codex-hooks", "/h.json",
            "--agents-src", "/agents",
            "--codex-agents", "/ag",
            "--hook-command", "/hook",
        ]);
        assert_eq!(args.targets.manifest_path, Path::new("/m.toml"));
        assert_eq!(args.targets.state_path, Path::new("/s.toml"));
        assert_eq!(args.targets.claude_json_path, Path::new("/c.json"));
        assert_eq!(args.targets.codex_toml_path, Path::new("/x.toml"));
        assert_eq!(args.targets.codex_hooks_json_path, Path::new("/h.json"));
        assert_eq!(args.targets.agents_src_dir, Path::new("/agents"));
        assert_eq!(args.targets.codex_agents_dir, Path::new("/ag"));
        assert_eq!(args.targets.hook_command, Path::new("/hook"));
    }

    #[test]
    fn unknown_flag_is_a_usage_error() {
        let message = usage_error(&["--bogus"]);
        assert!(message.contains("--bogus"), "{message}");
        assert!(message.contains("usage:"), "{message}");
    }

    #[test]
    fn unknown_mode_word_is_a_usage_error() {
        let message = usage_error(&["applyy"]);
        assert!(message.contains("applyy"), "{message}");
        assert!(message.contains("usage:"), "{message}");
    }

    #[test]
    fn value_flag_without_a_value_is_a_usage_error() {
        for flag in [
            "--manifest",
            "--state",
            "--claude-json",
            "--codex-toml",
            "--codex-hooks",
            "--agents-src",
            "--codex-agents",
            "--hook-command",
        ] {
            let message = usage_error(&[flag]);
            assert!(message.contains(flag), "{message}");
            assert!(message.contains("needs a path value"), "{message}");
        }
    }

    #[test]
    fn help_returns_the_bare_usage_text() {
        assert_eq!(usage_error(&["-h"]), USAGE);
        assert_eq!(usage_error(&["--help"]), USAGE);
    }

    #[test]
    fn from_roots_fills_all_eight_paths() {
        let targets = Targets::from_roots(Path::new("/repo"), Path::new("/hm"));
        assert_eq!(targets.manifest_path, Path::new("/repo/config/mcp-servers.toml"));
        assert_eq!(targets.state_path, Path::new("/repo/config/mcp-sync-state.toml"));
        assert_eq!(targets.claude_json_path, Path::new("/hm/.claude.json"));
        assert_eq!(targets.codex_toml_path, Path::new("/hm/.codex/config.toml"));
        assert_eq!(targets.codex_hooks_json_path, Path::new("/hm/.codex/hooks.json"));
        assert_eq!(targets.agents_src_dir, Path::new("/repo/agents"));
        assert_eq!(targets.codex_agents_dir, Path::new("/hm/.codex/agents"));
        assert_eq!(targets.hook_command, Path::new("/repo/hooks/rag-recall"));
    }
}
