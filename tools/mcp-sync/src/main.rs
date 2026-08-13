use std::env;
#[cfg(not(test))]
use std::io;
use std::io::Write;
use std::process::ExitCode;

mod agents;
mod claude;
mod cli;
mod codex;
mod drift;
mod error;
mod fingerprint;
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

#[cfg(not(test))]
fn run(args: &CliArgs) -> Result<ExitCode, SyncError> {
    let stdout = io::stdout();
    run_with_output(args, &mut stdout.lock())
}

#[cfg(test)]
fn run(args: &CliArgs) -> Result<ExitCode, SyncError> {
    run_with_output(args, &mut Vec::new())
}

fn run_with_output(args: &CliArgs, output: &mut impl Write) -> Result<ExitCode, SyncError> {
    let targets = &args.targets;
    let manifest = manifest::load_manifest(&targets.manifest_path)?;
    match args.mode {
        Mode::Apply => {
            let state = manifest::load_state(&targets.state_path)?;
            agents::parse_agents_dir(&targets.agents_src_dir)?;
            claude::validate(&targets.claude_json_path)?;
            codex::validate(&targets.codex_toml_path)?;
            hooks::validate(&targets.codex_hooks_json_path)?;
            let mut changes = claude::sync(
                &targets.claude_json_path,
                &manifest,
                &state,
                args.is_dry_run,
            )?;
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
            manifest::save_state(
                &targets.state_path,
                &managed_state(&manifest),
                args.is_dry_run,
            )?;
            output
                .write_all(drift::render_plan(&changes, args.is_dry_run).as_bytes())
                .expect("write apply output");
            Ok(ExitCode::SUCCESS)
        }
        Mode::Check => {
            let state = manifest::load_state(&targets.state_path)?;
            let mut rows = claude::check(&targets.claude_json_path, &manifest, &state)?;
            rows.extend(codex::check(&targets.codex_toml_path, &manifest, &state)?);
            output
                .write_all(drift::render_check(&rows).as_bytes())
                .expect("write check output");
            if drift::is_drift_present(&rows) {
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
            claude_managed.push(claude::managed_server(server));
        }
        if !matches!(server.scope, ToolScope::ClaudeOnly) {
            codex_managed.push(codex::managed_server(server));
        }
    }
    SyncState {
        claude_managed,
        codex_managed,
    }
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

        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets: apply_targets(&dir),
        };
        let err = run(&args).expect_err("invalid agent name aborts apply");
        assert!(
            err.to_string().contains("cannot name an output file"),
            "{err}"
        );

        assert_eq!(
            fs::read_to_string(dir.join("claude.json")).unwrap(),
            CLAUDE_JSON
        );
        assert_eq!(
            fs::read_to_string(dir.join("config.toml")).unwrap(),
            CODEX_TOML
        );
        assert!(!dir.join("state.toml").exists());
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            !names.iter().any(|name| name.contains(".pre-sync-")),
            "{names:?}"
        );
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

        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets: apply_targets(&dir),
        };
        run(&args).expect("valid agent set applies");

        assert!(dir.join("codex-agents/alpha.toml").exists());
        assert!(dir.join("state.toml").exists());
    }

    pub(crate) fn spared_entry_becomes_unmanaged_case() {
        let dir = fixture("release-spared");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
        fs::write(
            dir.join("claude.json"),
            "{\n  \"mcpServers\": {\n    \"stale\": {\"command\": \"hand-added\", \"args\": []}\n  }\n}\n",
        )
        .expect("seed Claude replacement");
        fs::write(
            dir.join("config.toml"),
            "[mcp_servers.stale]\ncommand = \"hand-added\"\nargs = []\n",
        )
        .expect("seed Codex replacement");
        fs::write(
            dir.join("state.toml"),
            "claude_managed = [{ name = \"stale\", fingerprint = \"sha256:old\" }]\n\
             codex_managed = [{ name = \"stale\", fingerprint = \"sha256:old\" }]\n",
        )
        .expect("seed stale ownership");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");

        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets: apply_targets(&dir),
        };
        let manifest = manifest::load_manifest(&args.targets.manifest_path)
            .expect("load manifest for spare preview");
        let state =
            manifest::load_state(&args.targets.state_path).expect("load state for spare preview");
        let mut preview = claude::sync(&args.targets.claude_json_path, &manifest, &state, true)
            .expect("preview Claude spare");
        preview.extend(
            codex::sync(&args.targets.codex_toml_path, &manifest, &state, true)
                .expect("preview Codex spare"),
        );
        let rendered = drift::render_plan(&preview, true);
        assert!(
            rendered.contains("dry:  claude spare stale\n"),
            "{rendered}"
        );
        assert!(rendered.contains("dry:  codex spare stale\n"), "{rendered}");
        run(&args).expect("apply spares replacements");
        let targets = &args.targets;

        let state = manifest::load_state(&targets.state_path).expect("load refreshed state");
        assert_eq!(state.claude_managed.len(), 1);
        assert_eq!(state.codex_managed.len(), 1);
        assert_eq!(state.claude_managed[0].name, "demo");
        assert_eq!(state.codex_managed[0].name, "demo");
        assert!(state.claude_managed[0].fingerprint.is_some());
        assert!(state.codex_managed[0].fingerprint.is_some());

        let manifest = manifest::load_manifest(&targets.manifest_path).expect("reload manifest");
        let claude_rows = claude::check(&targets.claude_json_path, &manifest, &state)
            .expect("check Claude after ownership release");
        let codex_rows = codex::check(&targets.codex_toml_path, &manifest, &state)
            .expect("check Codex after ownership release");
        assert!(claude_rows.iter().any(|row| {
            row.server == "stale" && matches!(row.state, drift::DriftState::Unmanaged)
        }));
        assert!(codex_rows.iter().any(|row| {
            row.server == "stale" && matches!(row.state, drift::DriftState::Unmanaged)
        }));
        assert!(fs::read_to_string(&targets.claude_json_path)
            .expect("read Claude config")
            .contains("hand-added"));
        assert!(fs::read_to_string(&targets.codex_toml_path)
            .expect("read Codex config")
            .contains("hand-added"));
    }

    #[test]
    fn legacy_state_apply_spares_live_entries_and_releases_ownership() {
        let dir = fixture("legacy-state-apply");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
        fs::write(
            dir.join("claude.json"),
            "{\n  \"mcpServers\": {\n    \"legacy-claude\": {\"command\": \"hand-added-claude\", \"args\": []}\n  }\n}\n",
        )
        .expect("seed Claude legacy replacement");
        fs::write(
            dir.join("config.toml"),
            "[mcp_servers.legacy-codex]\ncommand = \"hand-added-codex\"\nargs = []\n",
        )
        .expect("seed Codex legacy replacement");
        fs::write(
            dir.join("state.toml"),
            "claude_managed = [\"legacy-claude\"]\ncodex_managed = [\"legacy-codex\"]\n",
        )
        .expect("seed legacy state");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");

        let targets = apply_targets(&dir);
        let manifest = manifest::load_manifest(&targets.manifest_path).expect("load manifest");
        let legacy = manifest::load_state(&targets.state_path).expect("load legacy state");
        assert!(legacy.claude_managed[0].fingerprint.is_none());
        assert!(legacy.codex_managed[0].fingerprint.is_none());

        let claude_changes = claude::sync(&targets.claude_json_path, &manifest, &legacy, true)
            .expect("preview Claude legacy decision");
        assert!(matches!(
            claude_changes.as_slice(),
            [drift::Change { tool: drift::Tool::Claude, server, kind: drift::ChangeKind::Add }, drift::Change { tool: drift::Tool::Claude, server: spared, kind: drift::ChangeKind::Spare }]
                if server == "demo" && spared == "legacy-claude"
        ));
        let codex_changes = codex::sync(&targets.codex_toml_path, &manifest, &legacy, true)
            .expect("preview Codex legacy decision");
        assert!(matches!(
            codex_changes.as_slice(),
            [drift::Change { tool: drift::Tool::Codex, server, kind: drift::ChangeKind::Add }, drift::Change { tool: drift::Tool::Codex, server: spared, kind: drift::ChangeKind::Spare }]
                if server == "demo" && spared == "legacy-codex"
        ));

        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets,
        };
        run(&args).expect("apply legacy state");
        let targets = &args.targets;

        assert!(fs::read_to_string(&targets.claude_json_path)
            .expect("read Claude after apply")
            .contains("hand-added-claude"));
        assert!(fs::read_to_string(&targets.codex_toml_path)
            .expect("read Codex after apply")
            .contains("hand-added-codex"));
        let saved = manifest::load_state(&targets.state_path).expect("load refreshed state");
        assert!(saved
            .claude_managed
            .iter()
            .all(|managed| managed.name != "legacy-claude"));
        assert!(saved
            .codex_managed
            .iter()
            .all(|managed| managed.name != "legacy-codex"));
    }

    #[test]
    fn fingerprint_boundary_strings_are_deterministic_and_distinct() {
        let longest = "x".repeat(65_536);
        let cases = [
            ("empty-args", Vec::new(), "plain", "/tmp/plain"),
            ("unicode-工具-🚀", vec!["雪"], "ключ=значение", "/tmp/路径"),
            (
                "quotes-\"-'",
                vec!["\"quoted\"", "'quoted'"],
                "Q=\"'",
                "/tmp/'\"",
            ),
            (
                "newline\ncommand",
                vec!["line1\nline2"],
                "N=line1\nline2",
                "/tmp/new\nline",
            ),
            (
                "$(touch /tmp/inert-command); `false` | &",
                vec!["; rm -rf /tmp/inert", "https://127.0.0.1:1/no-request"],
                "PAYLOAD=$(false)",
                "/tmp/$(false)",
            ),
            (
                longest.as_str(),
                vec![longest.as_str()],
                longest.as_str(),
                longest.as_str(),
            ),
        ];
        let mut claude_fingerprints = Vec::new();
        let mut codex_fingerprints = Vec::new();

        for (index, (command, args, env_value, cwd)) in cases.iter().enumerate() {
            let entry = manifest::ServerEntry {
                name: format!("boundary-{index}"),
                transport: manifest::Transport::Stdio(manifest::StdioSpec {
                    command: (*command).to_string(),
                    args: args.iter().map(|arg| (*arg).to_string()).collect(),
                    env: vec![("BOUNDARY".to_string(), (*env_value).to_string())],
                    cwd: Some((*cwd).to_string()),
                }),
                scope: ToolScope::Both,
            };

            let claude_first = claude::managed_server(&entry).fingerprint.unwrap();
            let claude_second = claude::managed_server(&entry).fingerprint.unwrap();
            assert_eq!(claude_first, claude_second);
            claude_fingerprints.push(claude_first);

            let codex_first = codex::managed_server(&entry).fingerprint.unwrap();
            let codex_second = codex::managed_server(&entry).fingerprint.unwrap();
            assert_eq!(codex_first, codex_second);
            codex_fingerprints.push(codex_first);
        }

        for fingerprints in [&claude_fingerprints, &codex_fingerprints] {
            for left in 0..fingerprints.len() {
                for right in left + 1..fingerprints.len() {
                    assert_ne!(fingerprints[left], fingerprints[right]);
                }
            }
        }

        let activity_dir = fixture("fingerprint-inert-activity");
        let command_marker = activity_dir.join("command-ran");
        let command_entry = manifest::ServerEntry {
            name: "inert-command".to_string(),
            transport: manifest::Transport::Stdio(manifest::StdioSpec {
                command: format!("touch {}", command_marker.display()),
                args: Vec::new(),
                env: Vec::new(),
                cwd: None,
            }),
            scope: ToolScope::Both,
        };
        let _ = claude::managed_server(&command_entry);
        let _ = codex::managed_server(&command_entry);
        assert!(!command_marker.exists());

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        listener
            .set_nonblocking(true)
            .expect("make listener nonblocking");
        let remote_entry = manifest::ServerEntry {
            name: "inert-network".to_string(),
            transport: manifest::Transport::Remote(manifest::RemoteSpec {
                url: format!(
                    "http://{}",
                    listener.local_addr().expect("read listener address")
                ),
                bearer_token_env_var: None,
            }),
            scope: ToolScope::Both,
        };
        let _ = claude::managed_server(&remote_entry);
        let _ = codex::managed_server(&remote_entry);
        assert_eq!(
            listener
                .accept()
                .expect_err("fingerprinting must not connect")
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[cfg(unix)]
    pub(crate) fn state_write_failure_preserves_spared_live_entry_case() {
        use std::os::unix::fs::PermissionsExt;

        struct PermissionRestore {
            path: PathBuf,
            mode: u32,
        }

        impl Drop for PermissionRestore {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
            }
        }

        let dir = fixture("state-write-failure");
        fs::write(dir.join("manifest.toml"), "").expect("seed empty manifest");
        let claude_before = "{\n  \"mcpServers\": {\n    \"stale\": {\"command\": \"hand-added\", \"args\": []}\n  }\n}\n";
        fs::write(dir.join("claude.json"), claude_before).expect("seed spared replacement");
        fs::write(dir.join("config.toml"), CODEX_TOML).expect("seed Codex config");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");

        let state_dir = dir.join("state-only");
        fs::create_dir(&state_dir).expect("create state-only dir");
        let state_path = state_dir.join("state.toml");
        let state_before =
            "claude_managed = [{ name = \"stale\", fingerprint = \"sha256:old\" }]\n\
            codex_managed = []\n";
        fs::write(&state_path, state_before).expect("seed original state");
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o555))
            .expect("make state directory read-only");
        let _permission_restore = PermissionRestore {
            path: state_dir,
            mode: 0o755,
        };

        let mut targets = apply_targets(&dir);
        targets.state_path = state_path.clone();
        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets,
        };
        let err = run(&args).expect_err("state write must fail");

        match err {
            SyncError::Io(path, source) => {
                assert_eq!(path, state_path);
                assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
            }
            other => panic!("expected state Io(PermissionDenied), got {other}"),
        }
        assert_eq!(
            fs::read_to_string(&args.targets.claude_json_path).expect("read spared replacement"),
            claude_before
        );
        assert_eq!(
            fs::read_to_string(&args.targets.state_path).expect("read original state"),
            state_before
        );
    }

    #[test]
    fn dry_run_does_not_rewrite_managed_state() {
        let dir = fixture("dry-state");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
        fs::write(dir.join("claude.json"), CLAUDE_JSON).expect("seed claude.json");
        fs::write(dir.join("config.toml"), CODEX_TOML).expect("seed codex config.toml");
        fs::write(
            dir.join("state.toml"),
            "claude_managed = []\ncodex_managed = []\n",
        )
        .expect("seed state");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");

        let before = fs::read_to_string(dir.join("state.toml")).expect("read state before");
        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: true,
            targets: apply_targets(&dir),
        };
        run(&args).expect("dry apply");

        assert_eq!(fs::read_to_string(dir.join("state.toml")).unwrap(), before);
    }

    #[test]
    fn apply_with_absent_state_preserves_foreign_servers_and_saves_current_fingerprints() {
        let dir = fixture("absent-state-apply");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
        fs::write(
            dir.join("claude.json"),
            "{\n  \"mcpServers\": {\n    \"foreign\": {\"command\": \"hand-added\", \"args\": []}\n  }\n}\n",
        )
        .expect("seed Claude foreign server");
        fs::write(
            dir.join("config.toml"),
            "# foreign config comment\n[mcp_servers.foreign]\ncommand = \"hand-added\"\nargs = []\n",
        )
        .expect("seed Codex foreign server");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");
        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets: apply_targets(&dir),
        };
        assert!(!args.targets.state_path.exists());

        let manifest = manifest::load_manifest(&args.targets.manifest_path)
            .expect("load manifest for absent-state preview");
        let state =
            manifest::load_state(&args.targets.state_path).expect("load absent state for preview");
        let mut preview = claude::sync(&args.targets.claude_json_path, &manifest, &state, true)
            .expect("preview Claude absent-state apply");
        preview.extend(
            codex::sync(&args.targets.codex_toml_path, &manifest, &state, true)
                .expect("preview Codex absent-state apply"),
        );
        assert_eq!(preview.len(), 2, "{}", drift::render_plan(&preview, true));
        assert!(preview.iter().all(|change| {
            change.server == "demo" && matches!(change.kind, drift::ChangeKind::Add)
        }));

        run(&args).expect("apply with absent state succeeds");

        let claude_live =
            fs::read_to_string(&args.targets.claude_json_path).expect("read Claude config");
        assert!(claude_live.contains("\"foreign\""), "{claude_live}");
        assert!(claude_live.contains("\"hand-added\""), "{claude_live}");
        assert!(claude_live.contains("\"demo\""), "{claude_live}");
        let codex_live =
            fs::read_to_string(&args.targets.codex_toml_path).expect("read Codex config");
        assert!(codex_live.contains(
            "# foreign config comment\n[mcp_servers.foreign]\ncommand = \"hand-added\"\nargs = []\n"
        ));
        assert!(codex_live.contains("[mcp_servers.demo]"), "{codex_live}");

        let state = manifest::load_state(&args.targets.state_path).expect("load new state");
        assert!(matches!(
            state.claude_managed.as_slice(),
            [manifest::ManagedServer { name, fingerprint: Some(_) }] if name == "demo"
        ));
        assert!(matches!(
            state.codex_managed.as_slice(),
            [manifest::ManagedServer { name, fingerprint: Some(_) }] if name == "demo"
        ));
    }

    #[test]
    fn scope_fingerprint_tracks_each_tool_independently() {
        let dir = fixture("scope-fingerprint");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed both-tool manifest");
        fs::write(dir.join("claude.json"), CLAUDE_JSON).expect("seed claude.json");
        fs::write(dir.join("config.toml"), CODEX_TOML).expect("seed codex config.toml");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");
        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets: apply_targets(&dir),
        };

        run(&args).expect("initial both-tool apply succeeds");
        let initial_state =
            manifest::load_state(&args.targets.state_path).expect("load initial state");
        assert!(matches!(
            initial_state.claude_managed.as_slice(),
            [manifest::ManagedServer { name, fingerprint: Some(_) }] if name == "demo"
        ));
        assert!(matches!(
            initial_state.codex_managed.as_slice(),
            [manifest::ManagedServer { name, fingerprint: Some(_) }] if name == "demo"
        ));
        assert!(fs::read_to_string(&args.targets.codex_toml_path)
            .expect("read initial Codex config")
            .contains("[mcp_servers.demo]"));

        fs::write(
            &args.targets.manifest_path,
            "[servers.demo]\ncommand = \"echo\"\nargs = [\"hi\"]\ntools = [\"claude\"]\n",
        )
        .expect("change scope to Claude-only");
        run(&args).expect("Claude-only apply succeeds");

        let final_state = manifest::load_state(&args.targets.state_path).expect("load final state");
        assert!(matches!(
            final_state.claude_managed.as_slice(),
            [manifest::ManagedServer { name, fingerprint: Some(_) }] if name == "demo"
        ));
        assert!(final_state.codex_managed.is_empty());
        assert!(fs::read_to_string(&args.targets.claude_json_path)
            .expect("read final Claude config")
            .contains("\"demo\""));
        assert!(!fs::read_to_string(&args.targets.codex_toml_path)
            .expect("read final Codex config")
            .contains("mcp_servers.demo"));
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

        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets,
        };
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

        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets: apply_targets(&dir),
        };
        let err = run(&args).expect_err("over-long agent name aborts apply");
        assert!(
            err.to_string().contains("cannot name an output file"),
            "{err}"
        );

        assert_eq!(
            fs::read_to_string(dir.join("claude.json")).unwrap(),
            CLAUDE_JSON
        );
        assert_eq!(
            fs::read_to_string(dir.join("config.toml")).unwrap(),
            CODEX_TOML
        );
        assert!(!dir.join("state.toml").exists());
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            !names.iter().any(|name| name.contains(".pre-sync-")),
            "{names:?}"
        );
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

        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets: apply_targets(&dir),
        };
        let err = run(&args).expect_err("NUL-byte agent name aborts apply");
        assert!(
            err.to_string().contains("cannot name an output file"),
            "{err}"
        );

        assert_eq!(
            fs::read_to_string(dir.join("claude.json")).unwrap(),
            CLAUDE_JSON
        );
        assert_eq!(
            fs::read_to_string(dir.join("config.toml")).unwrap(),
            CODEX_TOML
        );
        assert!(!dir.join("state.toml").exists());
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            !names.iter().any(|name| name.contains(".pre-sync-")),
            "{names:?}"
        );
    }

    #[test]
    fn apply_refuses_before_any_write_on_a_malformed_hooks_json() {
        let dir = fixture("malformed-hooks");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
        fs::write(dir.join("claude.json"), CLAUDE_JSON).expect("seed claude.json");
        fs::write(dir.join("config.toml"), CODEX_TOML).expect("seed codex config.toml");
        fs::write(dir.join("hooks.json"), "{not json").expect("seed malformed hooks.json");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");
        write_agent(
            &dir.join("agents"),
            "alpha",
            "---\nname: alpha\ndescription: alpha does one job.\n---\nbody\n",
        );

        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets: apply_targets(&dir),
        };
        let err = run(&args).expect_err("malformed hooks.json aborts apply");
        assert!(matches!(err, SyncError::ParseJson(_, _)), "{err}");

        assert_eq!(
            fs::read_to_string(dir.join("claude.json")).unwrap(),
            CLAUDE_JSON
        );
        assert_eq!(
            fs::read_to_string(dir.join("config.toml")).unwrap(),
            CODEX_TOML
        );
        assert_eq!(
            fs::read_to_string(dir.join("hooks.json")).unwrap(),
            "{not json"
        );
        assert!(!dir.join("state.toml").exists());
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            !names.iter().any(|name| name.contains(".pre-sync-")),
            "{names:?}"
        );
    }

    #[test]
    fn apply_refuses_before_any_write_on_a_malformed_live_codex_config() {
        let dir = fixture("malformed-codex");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
        fs::write(dir.join("claude.json"), CLAUDE_JSON).expect("seed claude.json");
        fs::write(
            dir.join("config.toml"),
            "[mcp_servers.broken\ncommand = \"npx\"\n",
        )
        .expect("seed malformed codex config.toml");
        fs::create_dir_all(dir.join("agents")).expect("create agents dir");
        write_agent(
            &dir.join("agents"),
            "alpha",
            "---\nname: alpha\ndescription: alpha does one job.\n---\nbody\n",
        );

        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets: apply_targets(&dir),
        };
        let err = run(&args).expect_err("malformed live codex config aborts apply");
        assert!(matches!(err, SyncError::ParseToml(_, _)), "{err}");

        assert_eq!(
            fs::read_to_string(dir.join("claude.json")).unwrap(),
            CLAUDE_JSON
        );
        assert!(!dir.join("state.toml").exists());
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            !names.iter().any(|name| name.contains(".pre-sync-")),
            "{names:?}"
        );
    }

    pub(crate) fn adopt_claims_spared_entry_case() {
        let dir = fixture("adopt-spared");
        fs::write(dir.join("manifest.toml"), "# managed servers\n").expect("seed manifest");
        fs::write(
            dir.join("claude.json"),
            "{\n  \"mcpServers\": {\n    \"spared\": {\"command\": \"hand-added\", \"args\": [\"--flag\"], \"env\": {\"MODE\": \"safe\"}}\n  }\n}\n",
        )
        .expect("seed spared Claude server");
        fs::write(dir.join("config.toml"), CODEX_TOML).expect("seed Codex config");
        let state_before = "claude_managed = []\ncodex_managed = []\n";
        fs::write(dir.join("state.toml"), state_before).expect("seed released state");

        let args = CliArgs {
            mode: Mode::Adopt,
            is_dry_run: false,
            targets: apply_targets(&dir),
        };
        run(&args).expect("adopt spared server");

        let adopted =
            manifest::load_manifest(&args.targets.manifest_path).expect("reload manifest");
        assert_eq!(adopted.servers.len(), 1);
        let server = &adopted.servers[0];
        assert_eq!(server.name, "spared");
        assert!(matches!(server.scope, ToolScope::Both));
        let manifest::Transport::Stdio(spec) = &server.transport else {
            panic!("adopted server is stdio");
        };
        assert_eq!(spec.command, "hand-added");
        assert_eq!(spec.args, ["--flag"]);
        assert_eq!(spec.env, [("MODE".to_string(), "safe".to_string())]);
        assert_eq!(spec.cwd, None);
        assert_eq!(
            fs::read_to_string(&args.targets.state_path).expect("read state after adopt"),
            state_before
        );
        let names: Vec<String> = fs::read_dir(&dir)
            .expect("read fixture dir")
            .map(|entry| {
                entry
                    .expect("read fixture entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(
            !names
                .iter()
                .any(|name| name.starts_with("state.toml.pre-sync-")),
            "{names:?}"
        );
    }

    pub(crate) fn check_fingerprint_case() {
        let dir = fixture("check-fingerprint");
        let prior_path = dir.join("prior-manifest.toml");
        fs::write(
            &prior_path,
            "[servers.claude-match]\ncommand = \"npx\"\nargs = [\"match\"]\ntools = [\"claude\"]\n\
             \n[servers.claude-changed]\ncommand = \"npx\"\nargs = [\"original\"]\ntools = [\"claude\"]\n\
             \n[servers.codex-match]\ncommand = \"npx\"\nargs = [\"match\"]\ntools = [\"codex\"]\n\
             \n[servers.codex-changed]\ncommand = \"npx\"\nargs = [\"original\"]\ntools = [\"codex\"]\n",
        )
        .expect("seed prior manifest");
        let prior = manifest::load_manifest(&prior_path).expect("load prior manifest");
        let state = managed_state(&prior);
        let current = Manifest {
            servers: Vec::new(),
        };
        let claude_path = dir.join("claude.json");
        fs::write(
            &claude_path,
            "{\n  \"mcpServers\": {\n    \"claude-match\": {\"command\": \"npx\", \"args\": [\"match\"]},\n    \"claude-changed\": {\"command\": \"replacement\", \"args\": [\"changed\"]}\n  }\n}\n",
        )
        .expect("seed Claude extras");
        let codex_path = dir.join("config.toml");
        fs::write(
            &codex_path,
            "[mcp_servers.codex-match]\ncommand = \"npx\"\nargs = [\"match\"]\n\
             \n[mcp_servers.codex-changed]\ncommand = \"replacement\"\nargs = [\"changed\"]\n",
        )
        .expect("seed Codex extras");

        let claude_rows = claude::check(&claude_path, &current, &state).expect("check Claude");
        let codex_rows = codex::check(&codex_path, &current, &state).expect("check Codex");
        for (rows, matching_name, changed_name) in [
            (&claude_rows, "claude-match", "claude-changed"),
            (&codex_rows, "codex-match", "codex-changed"),
        ] {
            let matching = rows
                .iter()
                .find(|row| row.server == matching_name)
                .expect("matching extra row");
            assert!(matches!(matching.state, drift::DriftState::Drifted));
            assert!(drift::is_drift_present(std::slice::from_ref(matching)));

            let changed = rows
                .iter()
                .find(|row| row.server == changed_name)
                .expect("changed extra row");
            assert!(matches!(changed.state, drift::DriftState::Spared));
            assert!(drift::is_drift_present(std::slice::from_ref(changed)));
        }
    }

    pub(crate) fn dry_run_fingerprint_case() {
        let dir = fixture("dry-run-fingerprint");
        let prior_path = dir.join("prior-manifest.toml");
        fs::write(
            &prior_path,
            "[servers.claude-match]\ncommand = \"npx\"\nargs = [\"match\"]\ntools = [\"claude\"]\n\
             \n[servers.claude-changed]\ncommand = \"npx\"\nargs = [\"original\"]\ntools = [\"claude\"]\n\
             \n[servers.codex-match]\ncommand = \"npx\"\nargs = [\"match\"]\ntools = [\"codex\"]\n\
             \n[servers.codex-changed]\ncommand = \"npx\"\nargs = [\"original\"]\ntools = [\"codex\"]\n",
        )
        .expect("seed prior manifest");
        let prior = manifest::load_manifest(&prior_path).expect("load prior manifest");
        let state = managed_state(&prior);
        let current = Manifest {
            servers: Vec::new(),
        };
        let claude_path = dir.join("claude.json");
        let claude_before = "{\n  \"mcpServers\": {\n    \"claude-match\": {\"command\": \"npx\", \"args\": [\"match\"]},\n    \"claude-changed\": {\"command\": \"replacement\", \"args\": [\"changed\"]}\n  }\n}\n";
        fs::write(&claude_path, claude_before).expect("seed Claude extras");
        let codex_path = dir.join("config.toml");
        let codex_before = "[mcp_servers.codex-match]\ncommand = \"npx\"\nargs = [\"match\"]\n\
                            \n[mcp_servers.codex-changed]\ncommand = \"replacement\"\nargs = [\"changed\"]\n";
        fs::write(&codex_path, codex_before).expect("seed Codex extras");
        let state_path = dir.join("state.toml");
        manifest::save_state(&state_path, &state, false).expect("seed managed state");
        let state_before = fs::read_to_string(&state_path).expect("snapshot state");
        let files_before = fs::read_dir(&dir).expect("read fixture").count();

        let claude_changes =
            claude::sync(&claude_path, &current, &state, true).expect("dry-run Claude sync");
        let codex_changes =
            codex::sync(&codex_path, &current, &state, true).expect("dry-run Codex sync");
        manifest::save_state(&state_path, &managed_state(&current), true)
            .expect("dry-run state save");

        for (changes, matching_name, changed_name) in [
            (&claude_changes, "claude-match", "claude-changed"),
            (&codex_changes, "codex-match", "codex-changed"),
        ] {
            assert_eq!(changes.len(), 2);
            assert!(changes.iter().any(|change| {
                change.server == matching_name && matches!(&change.kind, drift::ChangeKind::Remove)
            }));
            assert!(changes.iter().any(|change| {
                change.server == changed_name && matches!(&change.kind, drift::ChangeKind::Spare)
            }));
        }
        assert_eq!(fs::read_to_string(&claude_path).unwrap(), claude_before);
        assert_eq!(fs::read_to_string(&codex_path).unwrap(), codex_before);
        assert_eq!(fs::read_to_string(&state_path).unwrap(), state_before);
        assert_eq!(
            fs::read_dir(&dir)
                .expect("read fixture after dry run")
                .count(),
            files_before
        );
    }

    pub(crate) fn malformed_fingerprint_case() {
        let cases = [
            (
                "non-string-name",
                "claude_managed = [{ name = 3 }]\ncodex_managed = []\n",
                "name",
            ),
            (
                "non-string-fingerprint",
                "claude_managed = [{ name = \"stale\", fingerprint = 3 }]\ncodex_managed = []\n",
                "fingerprint",
            ),
            (
                "missing-name",
                "claude_managed = [{ fingerprint = \"sha256:old\" }]\ncodex_managed = []\n",
                "name",
            ),
            (
                "non-array-managed-field",
                "claude_managed = \"stale\"\ncodex_managed = []\n",
                "claude_managed",
            ),
        ];

        for (case_name, malformed_state, malformed_field) in cases {
            let dir = fixture(&format!("malformed-fingerprint-{case_name}"));
            fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
            let claude_before = "{\n  \"mcpServers\": {\n    \"native\": {\"command\": \"keep-claude\", \"args\": []}\n  }\n}\n";
            let codex_before = "[mcp_servers.native]\ncommand = \"keep-codex\"\nargs = []\n";
            fs::write(dir.join("claude.json"), claude_before).expect("seed Claude config");
            fs::write(dir.join("config.toml"), codex_before).expect("seed Codex config");
            fs::write(dir.join("state.toml"), malformed_state).expect("seed malformed state");

            let args = CliArgs {
                mode: Mode::Apply,
                is_dry_run: false,
                targets: apply_targets(&dir),
            };
            let err = run(&args).expect_err("malformed state must stop apply");
            let error_text = err.to_string();
            let SyncError::ParseToml(path, detail) = err else {
                panic!("expected state TOML error, got {error_text}");
            };
            assert_eq!(path, args.targets.state_path);
            assert!(detail.contains(malformed_field), "{detail}");
            assert!(error_text.contains(&args.targets.state_path.display().to_string()));
            assert!(error_text.contains(malformed_field), "{error_text}");
            assert_eq!(
                fs::read_to_string(&args.targets.claude_json_path).unwrap(),
                claude_before
            );
            assert_eq!(
                fs::read_to_string(&args.targets.codex_toml_path).unwrap(),
                codex_before
            );
        }
    }

    #[cfg(unix)]
    pub(crate) fn permission_boundary_case() {
        use std::os::unix::fs::PermissionsExt;

        struct RestorePermissions {
            path: PathBuf,
            permissions: fs::Permissions,
        }

        impl Drop for RestorePermissions {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.path, self.permissions.clone());
            }
        }

        let dir = fixture("permission-boundary");
        let live_dir = dir.join("read-only-live");
        fs::create_dir(&live_dir).expect("create live config dir");
        fs::write(dir.join("manifest.toml"), MANIFEST_TOML).expect("seed manifest");
        let claude_before =
            "{\n  \"mcpServers\": {\n    \"demo\": {\"command\": \"old\", \"args\": []}\n  }\n}\n";
        let claude_path = live_dir.join("claude.json");
        fs::write(&claude_path, claude_before).expect("seed Claude config");
        fs::write(dir.join("config.toml"), CODEX_TOML).expect("seed Codex config");
        let state_before = "claude_managed = []\ncodex_managed = []\n";
        fs::write(dir.join("state.toml"), state_before).expect("seed writable state");
        fs::create_dir(dir.join("agents")).expect("create agents dir");

        let mut targets = apply_targets(&dir);
        targets.claude_json_path = claude_path.clone();
        let manifest = manifest::load_manifest(&targets.manifest_path).expect("load manifest");
        let state = manifest::load_state(&targets.state_path).expect("load state");
        let rows = claude::check(&claude_path, &manifest, &state).expect("check required update");
        assert!(rows.iter().any(|row| {
            row.server == "demo" && matches!(row.state, drift::DriftState::Drifted)
        }));

        let original_permissions = fs::metadata(&live_dir)
            .expect("read live dir metadata")
            .permissions();
        fs::set_permissions(&live_dir, fs::Permissions::from_mode(0o555))
            .expect("make live dir read-only");
        let _restore = RestorePermissions {
            path: live_dir,
            permissions: original_permissions,
        };

        let args = CliArgs {
            mode: Mode::Apply,
            is_dry_run: false,
            targets,
        };
        let mut output = Vec::new();
        let err = run_with_output(&args, &mut output)
            .expect_err("read-only live config must reject update");
        assert!(output.is_empty(), "{}", String::from_utf8_lossy(&output));
        let error_text = err.to_string();
        let SyncError::Io(path, source) = err else {
            panic!("expected exact I/O failure, got {error_text}");
        };
        assert_eq!(path, claude_path);
        assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            error_text,
            format!("cannot read or write {}: {}", path.display(), source)
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), claude_before);
        assert_eq!(
            fs::read_to_string(&args.targets.state_path).unwrap(),
            state_before
        );
        let names: Vec<String> = fs::read_dir(&dir)
            .expect("read fixture dir")
            .map(|entry| {
                entry
                    .expect("read fixture entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(
            !names
                .iter()
                .any(|name| name.starts_with("state.toml.pre-sync-")),
            "{names:?}"
        );
    }
}

#[cfg(test)]
mod main {
    mod tests {
        #[test]
        fn spared_entry_becomes_unmanaged() {
            crate::tests::spared_entry_becomes_unmanaged_case();
        }

        #[test]
        fn adopt_claims_spared_entry() {
            crate::tests::adopt_claims_spared_entry_case();
        }

        #[test]
        fn check_fingerprint() {
            crate::tests::check_fingerprint_case();
        }

        #[test]
        fn dry_run_fingerprint() {
            crate::tests::dry_run_fingerprint_case();
        }

        #[test]
        fn malformed_fingerprint() {
            crate::tests::malformed_fingerprint_case();
        }

        #[cfg(unix)]
        #[test]
        fn permission_boundary() {
            crate::tests::permission_boundary_case();
        }

        #[cfg(unix)]
        #[test]
        fn state_write_failure_preserves_spared_live_entry() {
            crate::tests::state_write_failure_preserves_spared_live_entry_case();
        }
    }
}
