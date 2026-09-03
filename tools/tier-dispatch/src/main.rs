//! `tools/tier-dispatch` — dispatches one live run of a skill's or agent's own
//! definition text at a specific model tier, walking that tier's own fallback chain on
//! a quota error and reporting the whole tier unavailable, never a guessed score, if
//! every model in the chain is exhausted. Built for the eval harness's execution arm
//! (`skills/ai-author/evals/run.sh`) — see `plans/eval-tier-sweep.md`.
//!
//! Contract:
//!   tier-dispatch --tiers-file <path> --tier <T1..T5> --system-prompt-file <path> --input <text>
//!   stdout: the artifact the dispatched run produced (only on success)
//!   stderr: `model_ran: <model id>` on success, or one diagnostic line per attempt
//!   exit 0: ran; a model in the chain succeeded
//!   exit 3: every model in the tier's own chain failed with a quota-classified error
//!   exit 1: a non-quota failure, or a usage/config error
//!   exit 2: unknown tier or missing required argument

mod config;
mod dispatch;

use config::TiersFile;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  tier-dispatch --tiers-file <path> --tier <T1..T5> --system-prompt-file <path> --input <text> [--dispatch-bin <bin>]
";

struct Args {
    tiers_file: PathBuf,
    tier: String,
    system_prompt_file: PathBuf,
    input: String,
    dispatch_bin: String,
}

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut tiers_file = None;
    let mut tier = None;
    let mut system_prompt_file = None;
    let mut input = None;
    let mut dispatch_bin = "pi".to_string();

    let mut i = 0;
    while i < raw.len() {
        let flag = raw[i].as_str();
        let mut next = || {
            i += 1;
            raw.get(i).cloned().ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--tiers-file" => tiers_file = Some(PathBuf::from(next()?)),
            "--tier" => tier = Some(next()?),
            "--system-prompt-file" => system_prompt_file = Some(PathBuf::from(next()?)),
            "--input" => input = Some(next()?),
            "--dispatch-bin" => dispatch_bin = next()?,
            other => return Err(format!("unknown flag {other}\n{USAGE}")),
        }
        i += 1;
    }

    Ok(Args {
        tiers_file: tiers_file.ok_or(format!("--tiers-file is required\n{USAGE}"))?,
        tier: tier.ok_or(format!("--tier is required\n{USAGE}"))?,
        system_prompt_file: system_prompt_file.ok_or(format!("--system-prompt-file is required\n{USAGE}"))?,
        input: input.ok_or(format!("--input is required\n{USAGE}"))?,
        dispatch_bin,
    })
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

    // The dispatched child runs inside a throwaway sandbox dir, not this process's
    // cwd, so a relative prompt path or a relative dispatch-bin path handed to the
    // child would resolve against the sandbox and silently miss. Resolve both here.
    let system_prompt_file = match std::fs::canonicalize(&args.system_prompt_file) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("tier-dispatch: cannot resolve --system-prompt-file {}: {error}", args.system_prompt_file.display());
            return ExitCode::from(2);
        }
    };
    let dispatch_bin = if args.dispatch_bin.contains('/') {
        match std::fs::canonicalize(&args.dispatch_bin) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(error) => {
                eprintln!("tier-dispatch: cannot resolve --dispatch-bin {}: {error}", args.dispatch_bin);
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
    let chain = match tiers.chain(&args.tier) {
        Ok(chain) => chain,
        Err(message) => {
            eprintln!("tier-dispatch: {message}");
            return ExitCode::from(2);
        }
    };

    match dispatch::walk_chain(&dispatch_bin, &chain, &system_prompt_file, &args.input) {
        dispatch::Outcome::Success { model_ran, artifact } => {
            println!("{artifact}");
            eprintln!("model_ran: {model_ran}");
            ExitCode::SUCCESS
        }
        dispatch::Outcome::TierExhausted { attempts } => {
            eprintln!(
                "tier-dispatch: tier {} exhausted — every model in its chain failed with a quota error, skipping this tier for this run",
                args.tier
            );
            for attempt in &attempts {
                eprintln!("  tried {} ({}): {}", attempt.model, attempt.thinking, attempt.stderr.trim());
            }
            ExitCode::from(3)
        }
        dispatch::Outcome::HardFailure { attempt } => {
            eprintln!(
                "tier-dispatch: {} failed with a non-quota error, stopping (not trying the rest of tier {}'s chain)",
                attempt.model, args.tier
            );
            eprintln!("  {}", attempt.stderr.trim());
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_required_flags() {
        let raw: Vec<String> = vec![
            "--tiers-file", "config/model-tiers.json",
            "--tier", "T3",
            "--system-prompt-file", "skill.md",
            "--input", "hello",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let args = parse_args(&raw).unwrap();
        assert_eq!(args.tier, "T3");
        assert_eq!(args.dispatch_bin, "pi");
    }

    #[test]
    fn missing_required_flag_is_an_error() {
        let raw: Vec<String> = vec!["--tier", "T3"].into_iter().map(String::from).collect();
        assert!(parse_args(&raw).is_err());
    }

    #[test]
    fn dispatch_bin_override_is_honored() {
        let raw: Vec<String> = vec![
            "--tiers-file", "config/model-tiers.json",
            "--tier", "T3",
            "--system-prompt-file", "skill.md",
            "--input", "hello",
            "--dispatch-bin", "fake-pi",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let args = parse_args(&raw).unwrap();
        assert_eq!(args.dispatch_bin, "fake-pi");
    }
}
