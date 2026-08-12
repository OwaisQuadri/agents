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
