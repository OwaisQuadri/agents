mod config;
mod dispatch;
mod registry;

use config::TiersFile;
use registry::{
    ModelOverrides, Registry, unknown_model_overrides, unknown_models, unreferenced_newer,
};
use serde_json::json;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  tier-dispatch --tiers-file <path> --tier <T1..T5> --system-prompt-file <path> --input <text> [--dispatch-bin <bin>]
  tier-dispatch --verify-registry --tiers-file <path> [--registry-file <path>] [--models-file <path>]
output:
  stdout: dispatched artifact on success
  stderr: JSON attempt records plus model_ran: <model id> on dispatch success; diagnostics otherwise
exit:
  0 success; 1 dispatch or registry failure; 2 invalid input or unavailable provider catalog; 3 tier unavailable; 4 tier timeout; 130 interrupted dispatch
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
    models_file_explicit: bool,
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
    let models_file_explicit = models_file.is_some();
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
        models_file_explicit,
    })
}

fn timeout_attempt_diagnostic(model: &str, thinking: &str, stderr: &str, stdout: &str) -> String {
    let mut diagnostic = format!("  tried {model} ({thinking}): {}", stderr.trim());
    if !stdout.is_empty() {
        diagnostic.push_str(&format!("\n  partial stdout (rejected):\n{stdout}"));
    }
    diagnostic
}

fn report_attempt(attempt: &dispatch::Attempt) {
    eprintln!(
        "attempt: {}",
        json!({
            "model": attempt.model,
            "thinking": attempt.thinking,
            "elapsed_ms": attempt.elapsed_ms,
            "result": attempt.result,
        })
    );
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
    let unavailable_tier_providers = registry.unavailable_tier_providers(&tiers);
    let registry_findings = unknown_models(&tiers, &registry);
    let actionable_findings = registry_findings
        .iter()
        .filter(|finding| {
            let provider = finding.model.split_once('/').map(|(provider, _)| provider);
            !provider.is_some_and(|provider| {
                unavailable_tier_providers
                    .iter()
                    .any(|item| item == provider)
            })
        })
        .collect::<Vec<_>>();
    for finding in &registry_findings {
        eprintln!(
            "tier-dispatch: {} {} {} does not resolve in the registry",
            finding.tier, finding.slot, finding.model
        );
    }
    if !unavailable_tier_providers.is_empty() {
        eprintln!(
            "tier-dispatch: registry has unavailable model catalogs for tier providers: {}",
            unavailable_tier_providers.join(", ")
        );
    }
    let models_file = args
        .models_file
        .as_ref()
        .expect("registry verification always resolves a models file");
    let (override_findings, override_input_error) = if models_file.is_file() {
        match ModelOverrides::load(models_file) {
            Ok(overrides) => (unknown_model_overrides(&overrides, &registry), false),
            Err(message) => {
                eprintln!("tier-dispatch: {message}");
                (Vec::new(), true)
            }
        }
    } else if args.models_file_explicit {
        eprintln!(
            "tier-dispatch: supplied model overrides file is unavailable: {}",
            models_file.display()
        );
        (Vec::new(), true)
    } else {
        eprintln!(
            "tier-dispatch: advisory: model overrides unavailable: {}",
            models_file.display()
        );
        (Vec::new(), false)
    };
    for model in &override_findings {
        eprintln!(
            "tier-dispatch: advisory: model override {model} from {} does not resolve in the registry",
            models_file.display()
        );
    }
    for model in unreferenced_newer(&tiers, &registry) {
        eprintln!("tier-dispatch: advisory: newer unreferenced model {model}");
    }
    if !actionable_findings.is_empty() {
        ExitCode::FAILURE
    } else if override_input_error || !unavailable_tier_providers.is_empty() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
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

    if let Err(error) = dispatch::install_signal_forwarder() {
        eprintln!("tier-dispatch: cannot install signal forwarding: {error}");
        return ExitCode::FAILURE;
    }

    match dispatch::walk_chain(
        &dispatch_bin,
        &chain,
        &system_prompt_file,
        args.input.as_ref().unwrap(),
    ) {
        dispatch::Outcome::Success {
            model_ran,
            artifact,
            attempts,
        } => {
            for attempt in &attempts {
                report_attempt(attempt);
            }
            println!("{artifact}");
            eprintln!("model_ran: {model_ran}");
            ExitCode::SUCCESS
        }
        dispatch::Outcome::TierExhausted { attempts } => {
            for attempt in &attempts {
                report_attempt(attempt);
            }
            eprintln!(
                "tier-dispatch: tier {tier} unavailable — every model in its chain failed with a retryable quota or availability error"
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
        dispatch::Outcome::TierTimedOut { attempts } => {
            for attempt in &attempts {
                report_attempt(attempt);
            }
            eprintln!("tier-dispatch: tier {tier} timed out");
            for attempt in &attempts {
                eprintln!(
                    "{}",
                    timeout_attempt_diagnostic(
                        &attempt.model,
                        &attempt.thinking,
                        &attempt.stderr,
                        &attempt.stdout
                    )
                );
            }
            ExitCode::from(4)
        }
        dispatch::Outcome::Interrupted { attempts } => {
            for attempt in &attempts {
                report_attempt(attempt);
            }
            eprintln!("tier-dispatch: interrupted");
            for attempt in &attempts {
                eprintln!("  tried {} ({})", attempt.model, attempt.thinking);
            }
            ExitCode::from(130)
        }
        dispatch::Outcome::HardFailure {
            attempt,
            previous_attempts,
        } => {
            for previous in &previous_attempts {
                report_attempt(previous);
            }
            report_attempt(&attempt);
            eprintln!(
                "tier-dispatch: {} failed with an unrelated error, stopping (not trying the rest of tier {tier}'s chain)",
                attempt.model
            );
            for previous in &previous_attempts {
                eprintln!(
                    "{}",
                    timeout_attempt_diagnostic(
                        &previous.model,
                        &previous.thinking,
                        &previous.stderr,
                        &previous.stdout
                    )
                );
            }
            eprintln!("  {}", attempt.stderr.trim());
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_diagnostic_labels_partial_stdout_as_rejected() {
        let diagnostic = timeout_attempt_diagnostic("model", "low", "timed out", "partial");
        assert!(diagnostic.contains("partial stdout (rejected)"));
        assert!(diagnostic.contains("partial"));
    }

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
