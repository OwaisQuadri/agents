//! `tools/tier-dispatch` dispatches an artifact at a model tier or reconciles tiers
//! with Pi's local registry. Dispatch runs one tier's fallback chain on quota errors.
//!
//! Exit 0 means a dispatch ran or reconciliation found no unknown models. Exit 1 means
//! a dispatch failed or reconciliation found an unknown model. Exit 2 means a required
//! file or argument is unavailable or malformed. Exit 3 means every dispatch model in
//! the tier was quota exhausted.

mod config;
mod dispatch;
mod registry;

use config::TiersFile;
use registry::{
    ModelOverrides, Registry, unknown_model_overrides, unknown_models, unreferenced_newer,
};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  tier-dispatch --tiers-file <path> --tier <T1..T5> --system-prompt-file <path> --input <text> [--dispatch-bin <bin>]
  tier-dispatch --verify-registry --tiers-file <path> [--registry-file <path>] [--models-file <path>]
output:
  stdout: dispatched artifact on success
  stderr: model_ran: <model id> on dispatch success; diagnostics otherwise
exit:
  0 success; 1 dispatch or registry failure; 2 invalid or unavailable input; 3 tier quota exhausted
";

struct Args {
    tiers_file: PathBuf,
    tier: Option<String>,
    system_prompt_file: Option<PathBuf>,
    input: Option<String>,
    dispatch_bin: String,
    is_verify_registry: bool,
    registry_file: Option<PathBuf>,
    models_file: Option<PathBuf>,
}

fn default_registry_file() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is required to find the Pi registry")?;
    Ok(PathBuf::from(home).join(".pi/agent/models-store.json"))
}

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut tiers_file = None;
    let mut tier = None;
    let mut system_prompt_file = None;
    let mut input = None;
    let mut dispatch_bin = None;
    let mut is_verify_registry = false;
    let mut registry_file = None;
    let mut models_file = None;

    let mut index = 0;
    while index < raw.len() {
        let flag = raw[index].as_str();
        let mut next = || {
            index += 1;
            raw.get(index)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--tiers-file" => tiers_file = Some(PathBuf::from(next()?)),
            "--tier" => tier = Some(next()?),
            "--system-prompt-file" => system_prompt_file = Some(PathBuf::from(next()?)),
            "--input" => input = Some(next()?),
            "--dispatch-bin" => dispatch_bin = Some(next()?),
            "--verify-registry" => is_verify_registry = true,
            "--registry-file" => registry_file = Some(PathBuf::from(next()?)),
            "--models-file" => models_file = Some(PathBuf::from(next()?)),
            other => return Err(format!("unknown flag {other}\n{USAGE}")),
        }
        index += 1;
    }

    let tiers_file = tiers_file.ok_or(format!("--tiers-file is required\n{USAGE}"))?;
    if is_verify_registry
        && (tier.is_some()
            || system_prompt_file.is_some()
            || input.is_some()
            || dispatch_bin.is_some())
    {
        return Err(format!(
            "dispatch flags cannot be combined with --verify-registry\n{USAGE}"
        ));
    }
    if !is_verify_registry && (registry_file.is_some() || models_file.is_some()) {
        return Err(format!(
            "--registry-file and --models-file require --verify-registry\n{USAGE}"
        ));
    }
    if !is_verify_registry && (tier.is_none() || system_prompt_file.is_none() || input.is_none()) {
        return Err(format!(
            "--tier, --system-prompt-file, and --input are required\n{USAGE}"
        ));
    }

    let registry_file = if is_verify_registry {
        Some(registry_file.map_or_else(default_registry_file, Ok)?)
    } else {
        registry_file
    };
    let models_file = if is_verify_registry {
        Some(models_file.unwrap_or_else(|| tiers_file.with_file_name("models.json")))
    } else {
        models_file
    };
    Ok(Args {
        tiers_file,
        tier,
        system_prompt_file,
        input,
        dispatch_bin: dispatch_bin.unwrap_or_else(|| "pi".to_owned()),
        is_verify_registry,
        registry_file,
        models_file,
    })
}

fn verify_registry(args: &Args) -> ExitCode {
    let tiers = match TiersFile::load(&args.tiers_file) {
        Ok(tiers) => tiers,
        Err(message) => {
            eprintln!("tier-dispatch: {message}");
            return ExitCode::from(2);
        }
    };
    let registry_file = args
        .registry_file
        .as_ref()
        .expect("registry verification always resolves a registry path");
    let registry = match Registry::load(registry_file) {
        Ok(registry) => registry,
        Err(message) => {
            eprintln!("tier-dispatch: {message}");
            return ExitCode::from(2);
        }
    };
    let models_file = args
        .models_file
        .as_ref()
        .expect("registry verification always resolves a models file");
    let overrides = match ModelOverrides::load(models_file) {
        Ok(overrides) => overrides,
        Err(message) => {
            eprintln!("tier-dispatch: {message}");
            return ExitCode::from(2);
        }
    };
    let registry_findings = unknown_models(&tiers, &registry);
    let override_findings = unknown_model_overrides(&overrides, &registry);
    for finding in &registry_findings {
        eprintln!(
            "tier-dispatch: {} {} {} is absent from the registry",
            finding.tier, finding.slot, finding.model
        );
    }
    for model in &override_findings {
        eprintln!(
            "tier-dispatch: {model} from {} is absent from the registry",
            models_file.display()
        );
    }
    for model in unreferenced_newer(&tiers, &registry) {
        eprintln!("tier-dispatch: advisory: newer unreferenced model {model}");
    }
    if registry_findings.is_empty() && override_findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&raw) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("tier-dispatch: {message}");
            return ExitCode::from(2);
        }
    };
    if args.is_verify_registry {
        return verify_registry(&args);
    }

    let system_prompt_file = match std::fs::canonicalize(args.system_prompt_file.as_ref().unwrap())
    {
        Ok(path) => path,
        Err(error) => {
            eprintln!(
                "tier-dispatch: cannot resolve --system-prompt-file {}: {error}",
                args.system_prompt_file.as_ref().unwrap().display()
            );
            return ExitCode::from(2);
        }
    };
    let dispatch_bin = if args.dispatch_bin.contains('/') {
        match std::fs::canonicalize(&args.dispatch_bin) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(error) => {
                eprintln!(
                    "tier-dispatch: cannot resolve --dispatch-bin {}: {error}",
                    args.dispatch_bin
                );
                return ExitCode::from(2);
            }
        }
    } else {
        args.dispatch_bin.clone()
    };

    let tiers = match TiersFile::load(&args.tiers_file) {
        Ok(tiers) => tiers,
        Err(message) => {
            eprintln!("tier-dispatch: {message}");
            return ExitCode::from(2);
        }
    };
    let tier = args.tier.as_ref().unwrap();
    let chain = match tiers.chain(tier) {
        Ok(chain) => chain,
        Err(message) => {
            eprintln!("tier-dispatch: {message}");
            return ExitCode::from(2);
        }
    };

    match dispatch::walk_chain(
        &dispatch_bin,
        &chain,
        &system_prompt_file,
        args.input.as_ref().unwrap(),
    ) {
        dispatch::Outcome::Success {
            model_ran,
            artifact,
        } => {
            println!("{artifact}");
            eprintln!("model_ran: {model_ran}");
            ExitCode::SUCCESS
        }
        dispatch::Outcome::TierExhausted { attempts } => {
            eprintln!(
                "tier-dispatch: tier {tier} exhausted — every model in its chain failed with a quota error, skipping this tier for this run"
            );
            for attempt in &attempts {
                eprintln!(
                    "  tried {} ({}): {}",
                    attempt.model,
                    attempt.thinking,
                    attempt.stderr.trim()
                );
            }
            ExitCode::from(3)
        }
        dispatch::Outcome::HardFailure { attempt } => {
            eprintln!(
                "tier-dispatch: {} failed with a non-quota error, stopping (not trying the rest of tier {tier}'s chain)",
                attempt.model
            );
            eprintln!("  {}", attempt.stderr.trim());
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_required_flags() {
        let raw = [
            "--tiers-file",
            "config/model-tiers.json",
            "--tier",
            "T3",
            "--system-prompt-file",
            "skill.md",
            "--input",
            "hello",
        ]
        .map(String::from);
        let args = parse_args(&raw).unwrap();
        assert_eq!(args.tier.as_deref(), Some("T3"));
        assert_eq!(args.dispatch_bin, "pi");
        assert!(args.registry_file.is_none());
    }

    #[test]
    fn parses_verify_registry_without_dispatch_flags() {
        let raw = [
            "--verify-registry",
            "--tiers-file",
            "config/model-tiers.json",
        ]
        .map(String::from);
        let args = parse_args(&raw).unwrap();
        assert!(args.is_verify_registry);
        assert!(args.tier.is_none());
    }

    #[test]
    fn verify_registry_defaults_the_registry_path_from_home() {
        let raw = [
            "--verify-registry",
            "--tiers-file",
            "config/model-tiers.json",
        ]
        .map(String::from);
        let args = parse_args(&raw).unwrap();
        assert!(
            args.registry_file
                .is_some_and(|path| path.ends_with(".pi/agent/models-store.json"))
        );
        assert_eq!(args.models_file, Some(PathBuf::from("config/models.json")));
    }

    #[test]
    fn rejects_flags_from_the_other_mode() {
        let verify_with_dispatch = [
            "--verify-registry",
            "--tiers-file",
            "config/model-tiers.json",
            "--tier",
            "T3",
        ]
        .map(String::from);
        assert!(parse_args(&verify_with_dispatch).is_err());

        let verify_with_dispatch_bin = [
            "--verify-registry",
            "--tiers-file",
            "config/model-tiers.json",
            "--dispatch-bin",
            "fake-pi",
        ]
        .map(String::from);
        assert!(parse_args(&verify_with_dispatch_bin).is_err());

        let dispatch_with_registry = [
            "--tiers-file",
            "config/model-tiers.json",
            "--tier",
            "T3",
            "--system-prompt-file",
            "skill.md",
            "--input",
            "hello",
            "--registry-file",
            "models-store.json",
        ]
        .map(String::from);
        assert!(parse_args(&dispatch_with_registry).is_err());
    }

    #[test]
    fn missing_required_flag_is_an_error() {
        let raw = ["--tier", "T3"].map(String::from);
        assert!(parse_args(&raw).is_err());
    }

    #[test]
    fn dispatch_bin_override_is_honored() {
        let raw = [
            "--tiers-file",
            "config/model-tiers.json",
            "--tier",
            "T3",
            "--system-prompt-file",
            "skill.md",
            "--input",
            "hello",
            "--dispatch-bin",
            "fake-pi",
        ]
        .map(String::from);
        let args = parse_args(&raw).unwrap();
        assert_eq!(args.dispatch_bin, "fake-pi");
    }
}
