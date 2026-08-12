use std::env;
use std::process::ExitCode;

mod agents;
mod claude;
mod cli;
mod codex;
mod drift;
mod error;
mod fsio;
mod hooks;
mod manifest;

use cli::{CliArgs, Mode};
use error::SyncError;
use hooks::HookRegistration;
use manifest::{Manifest, SyncState, ToolScope};

const HOOK_TIMEOUT_SECS: u64 = 10;

fn main() -> ExitCode {
    let args = match cli::parse_args(env::args().skip(1)) {
        Ok(args) => args,
        Err(message) if message == cli::USAGE => {
            println!("{message}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn run(args: &CliArgs) -> Result<ExitCode, SyncError> {
    let targets = &args.targets;
    let manifest = manifest::load_manifest(&targets.manifest_path)?;
    match args.mode {
        Mode::Apply => {
            let state = manifest::load_state(&targets.state_path)?;
            agents::parse_agents_dir(&targets.agents_src_dir)?;
            claude::validate(&targets.claude_json_path)?;
            codex::validate(&targets.codex_toml_path)?;
            let mut changes =
                claude::sync(&targets.claude_json_path, &manifest, &state, args.is_dry_run)?;
            changes.extend(codex::sync(
                &targets.codex_toml_path,
                &manifest,
                &state,
                args.is_dry_run,
            )?);
            let reg = HookRegistration {
                command: targets.hook_command.display().to_string(),
                timeout_secs: HOOK_TIMEOUT_SECS,
            };
            changes.extend(hooks::sync_codex_hook(
                &targets.codex_hooks_json_path,
                &reg,
                args.is_dry_run,
            )?);
            changes.extend(agents::sync_agents(
                &targets.agents_src_dir,
                &targets.codex_agents_dir,
                args.is_dry_run,
            )?);
            manifest::save_state(&targets.state_path, &managed_state(&manifest), args.is_dry_run)?;
            print!("{}", drift::render_plan(&changes, args.is_dry_run));
            Ok(ExitCode::SUCCESS)
        }
        Mode::Check => {
            let state = manifest::load_state(&targets.state_path)?;
            let mut rows = claude::check(&targets.claude_json_path, &manifest, &state)?;
            rows.extend(codex::check(&targets.codex_toml_path, &manifest, &state)?);
            print!("{}", drift::render_check(&rows));
            if drift::has_drift(&rows) {
                Ok(ExitCode::from(1))
            } else {
                Ok(ExitCode::SUCCESS)
            }
        }
        Mode::Adopt => {
            let mut entries = claude::unmanaged(&targets.claude_json_path, &manifest)?;
            for entry in codex::unmanaged(&targets.codex_toml_path, &manifest)? {
                if entries.iter().all(|held| held.name != entry.name) {
                    entries.push(entry);
                }
            }
            if !entries.is_empty() {
                manifest::append_servers(&targets.manifest_path, &entries, args.is_dry_run)?;
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn managed_state(manifest: &Manifest) -> SyncState {
    let mut claude_managed = Vec::new();
    let mut codex_managed = Vec::new();
    for server in &manifest.servers {
        if !matches!(server.scope, ToolScope::CodexOnly) {
            claude_managed.push(server.name.clone());
        }
        if !matches!(server.scope, ToolScope::ClaudeOnly) {
            codex_managed.push(server.name.clone());
        }
    }
    SyncState { claude_managed, codex_managed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cli::Targets;
    use std::fs;
    use std::path::{Path, PathBuf};

    const CLAUDE_JSON: &str = "{\n  \"unrelatedKey\": \"keepme\",\n  \"mcpServers\": {}\n}\n";
    const CODEX_TOML: &str = "# a comment that must survive\n[projects.foo]\ntrust = true\n";
    const MANIFEST_TOML: &str =
        "[servers.demo]\ncommand = \"echo\"\nargs = [\"hi\"]\ntools = [\"claude\", \"codex\"]\n";

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mcp-sync-main-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    fn write_agent(src_dir: &Path, name: &str, md: &str) {
        let dir = src_dir.join(name);
        fs::create_dir_all(&dir).expect("create agent dir");
        fs::write(dir.join(format!("{name}.md")), md).expect("seed agent md");
    }

    fn apply_targets(dir: &Path) -> Targets {
        Targets {
            manifest_path: dir.join("manifest.toml"),
            state_path: dir.join("state.toml"),
            claude_json_path: dir.join("claude.json"),
            codex_toml_path: dir.join("config.toml"),
            codex_hooks_json_path: dir.join("hooks.json"),
            agents_src_dir: dir.join("agents"),
            codex_agents_dir: dir.join("codex-agents"),
            hook_command: dir.join("hooks/rag-recall"),
        }
    }

    #[test]
    fn apply_refuses_before_any_write_on_an_invalid_agent_name() {
        let dir = fixture("invalid-agent");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
        fs::write(dir.join("claude.json"), CLAUDE_JSON).expect("seed claude.json");
        fs::write(dir.join("config.toml"), CODEX_TOML).expect("seed codex config.toml");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");
        write_agent(
            &dir.join("agents"),
            "dotdir",
            "---\nname: ..\ndescription: name is all dots\n---\nbody\n",
        );

        let args = CliArgs { mode: Mode::Apply, is_dry_run: false, targets: apply_targets(&dir) };
        let err = run(&args).expect_err("invalid agent name aborts apply");
        assert!(err.to_string().contains("cannot name an output file"), "{err}");

        assert_eq!(fs::read_to_string(dir.join("claude.json")).unwrap(), CLAUDE_JSON);
        assert_eq!(fs::read_to_string(dir.join("config.toml")).unwrap(), CODEX_TOML);
        assert!(!dir.join("state.toml").exists());
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|name| name.contains(".pre-sync-")), "{names:?}");
    }

    #[test]
    fn apply_converges_a_valid_agent_set() {
        let dir = fixture("valid-agent");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
        fs::write(dir.join("claude.json"), CLAUDE_JSON).expect("seed claude.json");
        fs::write(dir.join("config.toml"), CODEX_TOML).expect("seed codex config.toml");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");
        write_agent(
            &dir.join("agents"),
            "alpha",
            "---\nname: alpha\ndescription: alpha does one job.\n---\nbody\n",
        );

        let args = CliArgs { mode: Mode::Apply, is_dry_run: false, targets: apply_targets(&dir) };
        run(&args).expect("valid agent set applies");

        assert!(dir.join("codex-agents/alpha.toml").exists());
        assert!(dir.join("state.toml").exists());
    }

    #[test]
    fn apply_converges_the_real_repo_manifest_and_agents() {
        let dir = fixture("real-repo");
        fs::write(dir.join("claude.json"), CLAUDE_JSON).expect("seed claude.json");
        fs::write(dir.join("config.toml"), CODEX_TOML).expect("seed codex config.toml");
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut targets = apply_targets(&dir);
        targets.manifest_path = repo_root.join("config/mcp-servers.toml");
        targets.agents_src_dir = repo_root.join("agents");

        let args = CliArgs { mode: Mode::Apply, is_dry_run: false, targets };
        run(&args).expect("real repo manifest and agents apply");

        for name in [
            "anchor-verifier",
            "code-reviewer",
            "debugger",
            "maestro-tester",
            "spec-tester",
            "web-research-summarizer",
        ] {
            let rendered = dir.join("codex-agents").join(format!("{name}.toml"));
            assert!(rendered.exists(), "{}", rendered.display());
        }
        assert!(dir.join("state.toml").exists());
        let claude_after = fs::read_to_string(dir.join("claude.json")).unwrap();
        assert!(claude_after.contains("XcodeBuildMCP"), "{claude_after}");
    }

    #[test]
    fn apply_refuses_before_any_write_on_a_too_long_agent_name() {
        let dir = fixture("too-long-name");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
        fs::write(dir.join("claude.json"), CLAUDE_JSON).expect("seed claude.json");
        fs::write(dir.join("config.toml"), CODEX_TOML).expect("seed codex config.toml");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");
        let long_name = "a".repeat(300);
        write_agent(
            &dir.join("agents"),
            "toolong",
            &format!("---\nname: {long_name}\ndescription: name is 300 chars long\n---\nbody\n"),
        );

        let args = CliArgs { mode: Mode::Apply, is_dry_run: false, targets: apply_targets(&dir) };
        let err = run(&args).expect_err("over-long agent name aborts apply");
        assert!(err.to_string().contains("cannot name an output file"), "{err}");

        assert_eq!(fs::read_to_string(dir.join("claude.json")).unwrap(), CLAUDE_JSON);
        assert_eq!(fs::read_to_string(dir.join("config.toml")).unwrap(), CODEX_TOML);
        assert!(!dir.join("state.toml").exists());
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|name| name.contains(".pre-sync-")), "{names:?}");
    }

    #[test]
    fn apply_refuses_before_any_write_on_a_control_char_agent_name() {
        let dir = fixture("control-char-name");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
        fs::write(dir.join("claude.json"), CLAUDE_JSON).expect("seed claude.json");
        fs::write(dir.join("config.toml"), CODEX_TOML).expect("seed codex config.toml");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");
        write_agent(
            &dir.join("agents"),
            "nulname",
            "---\nname: bad\u{0}name\ndescription: name has a NUL byte\n---\nbody\n",
        );

        let args = CliArgs { mode: Mode::Apply, is_dry_run: false, targets: apply_targets(&dir) };
        let err = run(&args).expect_err("NUL-byte agent name aborts apply");
        assert!(err.to_string().contains("cannot name an output file"), "{err}");

        assert_eq!(fs::read_to_string(dir.join("claude.json")).unwrap(), CLAUDE_JSON);
        assert_eq!(fs::read_to_string(dir.join("config.toml")).unwrap(), CODEX_TOML);
        assert!(!dir.join("state.toml").exists());
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|name| name.contains(".pre-sync-")), "{names:?}");
    }

    #[test]
    fn apply_refuses_before_any_write_on_a_malformed_live_codex_config() {
        let dir = fixture("malformed-codex");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
        fs::write(dir.join("claude.json"), CLAUDE_JSON).expect("seed claude.json");
        fs::write(dir.join("config.toml"), "[mcp_servers.broken\ncommand = \"npx\"\n")
            .expect("seed malformed codex config.toml");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");
        write_agent(
            &dir.join("agents"),
            "alpha",
            "---\nname: alpha\ndescription: alpha does one job.\n---\nbody\n",
        );

        let args = CliArgs { mode: Mode::Apply, is_dry_run: false, targets: apply_targets(&dir) };
        let err = run(&args).expect_err("malformed live codex config aborts apply");
        assert!(matches!(err, SyncError::ParseToml(_, _)), "{err}");

        assert_eq!(fs::read_to_string(dir.join("claude.json")).unwrap(), CLAUDE_JSON);
        assert!(!dir.join("state.toml").exists());
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|name| name.contains(".pre-sync-")), "{names:?}");
    }
}
