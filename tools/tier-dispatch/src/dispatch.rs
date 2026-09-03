//! Dispatches one live run of a skill's or agent's own definition text at a specific
//! model, and classifies a failed run as either "this model is out of quota, try the
//! next one in the tier's own chain" or "something else broke, stop and say so" — the
//! two are never conflated, per this repo's own rule (`invariants.md`, AGNT-INV-003)
//! against reporting success, or in this case a same-tier substitution, for anything
//! other than what actually ran.
//!
//! Verified against a real known input/output pair before this module was written:
//! `pi -p --model anthropic/claude-haiku-4-5 --thinking off --no-session "reply with
//! exactly the single word: PONG"` printed exactly `PONG` to stdout, exit 0, with
//! unrelated setup noise on stderr only — confirming `pi -p --model <provider/id>`
//! takes `config/model-tiers.json`'s own id format directly, no alias translation
//! needed (unlike the Claude Code CLI path `install.sh` already handles separately).

use crate::config::ModelEntry;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Substrings that mark a failure as "this model is out of quota, not a real defect" —
/// intentionally narrow. An unmatched failure is a hard stop, not swallowed into a
/// same-tier retry that would misattribute which model actually produced a result.
const QUOTA_MARKERS: &[&str] = &[
    "rate limit",
    "rate_limit",
    "usage limit",
    "usage-limit",
    "quota",
    "resource_exhausted",
    "429 ",
    "http 429",
    "status: 429",
    "too many requests",
];

pub fn is_quota_error(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    QUOTA_MARKERS.iter().any(|marker| lower.contains(marker))
}

pub struct Attempt {
    pub model: String,
    pub thinking: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

pub enum Outcome {
    /// A model in the chain ran and produced output. `model_ran` is the exact model
    /// that produced `artifact` — never the tier's nominal primary if a fallback fired.
    Success { model_ran: String, artifact: String },
    /// Every model in the tier's own chain failed with a quota-classified error. The
    /// tier is skipped for this run; no score is recorded, never a guessed 0.
    TierExhausted { attempts: Vec<Attempt> },
    /// A non-quota failure on some model in the chain. Reported immediately, not
    /// retried across the rest of the chain, so a real defect is never hidden behind a
    /// same-tier substitution.
    HardFailure { attempt: Attempt },
}

/// Runs `dispatch_bin` (normally `pi`) once per model in `chain`, in order, stopping at
/// the first success or the first non-quota failure. `system_prompt_file` is the
/// skill's or agent's own definition text, appended as the dispatched run's system
/// prompt — the same pattern `agents/spec-tester/evals/run.sh` already uses for a real
/// agent, generalized here to also cover a skill (no separate "run a skill" mechanism
/// exists; a skill IS the system prompt an agent follows).
pub fn walk_chain(
    dispatch_bin: &str,
    chain: &[ModelEntry],
    system_prompt_file: &Path,
    input: &str,
) -> Outcome {
    let mut attempts = Vec::new();
    for entry in chain {
        let attempt = run_one(dispatch_bin, entry, system_prompt_file, input);
        let failed = attempt.exit_code != Some(0);
        if !failed {
            return Outcome::Success {
                model_ran: attempt.model.clone(),
                artifact: attempt.stdout.clone(),
            };
        }
        if !is_quota_error(&attempt.stderr) {
            return Outcome::HardFailure { attempt };
        }
        attempts.push(attempt);
    }
    Outcome::TierExhausted { attempts }
}

static SANDBOX_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Creates a fresh empty directory under the OS temp dir, unique per process and per
/// call, for one dispatched attempt to run in.
fn make_sandbox() -> std::io::Result<PathBuf> {
    let n = SANDBOX_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("tier-dispatch-sandbox-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn run_one(dispatch_bin: &str, entry: &ModelEntry, system_prompt_file: &Path, input: &str) -> Attempt {
    // The child runs with tools ON but its working directory set to a fresh throwaway
    // sandbox, discarded after the attempt. Tools stay on because the harness grades
    // what a tier can actually DO, and a dispatch stripped of tools is a different,
    // easier task whose score stops predicting real capability. The sandbox exists
    // because a dispatch pointed at the live repo once edited this repository's own
    // tracked CLAUDE.md for real during a graded run of eval case a1 -- the case's own
    // EXPECT names CLAUDE.md as the right destination, and the model, given write
    // access and no signal this was an exercise, made the edit instead of stating the
    // verdict. Each attempt gets its own sandbox so a fallback model never sees the
    // primary's leftover writes.
    let sandbox = match make_sandbox() {
        Ok(dir) => dir,
        Err(error) => {
            return Attempt {
                model: entry.model.clone(),
                thinking: entry.thinking.clone(),
                stdout: String::new(),
                stderr: format!("failed to create sandbox dir: {error}"),
                exit_code: None,
            };
        }
    };
    let output = Command::new(dispatch_bin)
        .arg("-p")
        .arg("--model")
        .arg(&entry.model)
        .arg("--thinking")
        .arg(&entry.thinking)
        .arg("--append-system-prompt")
        .arg(system_prompt_file)
        .arg("--no-session")
        .arg(input)
        .current_dir(&sandbox)
        .output();
    let attempt =
    match output {
        Ok(output) => Attempt {
            model: entry.model.clone(),
            thinking: entry.thinking.clone(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code(),
        },
        Err(error) => Attempt {
            model: entry.model.clone(),
            thinking: entry.thinking.clone(),
            stdout: String::new(),
            stderr: format!("failed to spawn {dispatch_bin}: {error}"),
            exit_code: None,
        },
    };
    std::fs::remove_dir_all(&sandbox).ok();
    attempt
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn recognizes_common_quota_phrasings() {
        assert!(is_quota_error("Error: rate limit exceeded, try again later"));
        assert!(is_quota_error("429 Too Many Requests"));
        assert!(is_quota_error("RESOURCE_EXHAUSTED: quota exceeded"));
        assert!(is_quota_error("Usage limit reached for this account"));
    }

    #[test]
    fn does_not_classify_an_unrelated_failure_as_quota() {
        assert!(!is_quota_error("panic: index out of bounds"));
        assert!(!is_quota_error("connection refused"));
        assert!(!is_quota_error(""));
    }

    /// Writes a fake `pi`-like script into a temp dir and returns (dir, script_path).
    /// The script echoes which model it was called with, so a test can assert on
    /// exactly which model in the chain actually ran — the same "never claim more than
    /// what really executed" concern `dispatch.rs`'s own doc comment names.
    fn fake_dispatch_bin(dir: &Path, behavior: &str) -> std::path::PathBuf {
        let path = dir.join("fake-pi");
        std::fs::write(&path, behavior).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tier-dispatch-dispatch-test-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_system_prompt(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("skill.md");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"# fake skill\n").unwrap();
        path
    }

    #[test]
    fn success_on_primary_never_touches_fallbacks() {
        let dir = temp_dir("success-primary");
        let script = fake_dispatch_bin(
            &dir,
            "#!/bin/sh\necho \"ran:$3\"\nexit 0\n",
        );
        let chain = vec![
            ModelEntry { model: "primary-model".into(), thinking: "low".into() },
            ModelEntry { model: "fallback-model".into(), thinking: "low".into() },
        ];
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain(script.to_str().unwrap(), &chain, &prompt, "hello");
        match outcome {
            Outcome::Success { model_ran, artifact } => {
                assert_eq!(model_ran, "primary-model");
                assert!(artifact.contains("ran:primary-model"));
            }
            _ => panic!("expected Success"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn quota_failure_on_primary_walks_to_fallback() {
        let dir = temp_dir("quota-walk");
        // $4 is --model's argument (arg order: -p --model <model> ...).
        let script = fake_dispatch_bin(
            &dir,
            r#"#!/bin/sh
model="$3"
if [ "$model" = "primary-model" ]; then
  echo "rate limit exceeded" 1>&2
  exit 1
fi
echo "ran:$model"
exit 0
"#,
        );
        let chain = vec![
            ModelEntry { model: "primary-model".into(), thinking: "low".into() },
            ModelEntry { model: "fallback-model".into(), thinking: "low".into() },
        ];
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain(script.to_str().unwrap(), &chain, &prompt, "hello");
        match outcome {
            Outcome::Success { model_ran, artifact } => {
                assert_eq!(model_ran, "fallback-model");
                assert!(artifact.contains("ran:fallback-model"));
            }
            _ => panic!("expected Success on fallback, got a different outcome"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn quota_failure_on_every_model_in_chain_reports_tier_exhausted() {
        let dir = temp_dir("quota-exhausted");
        let script = fake_dispatch_bin(
            &dir,
            "#!/bin/sh\necho \"quota exceeded\" 1>&2\nexit 1\n",
        );
        let chain = vec![
            ModelEntry { model: "primary-model".into(), thinking: "low".into() },
            ModelEntry { model: "fallback-model".into(), thinking: "low".into() },
        ];
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain(script.to_str().unwrap(), &chain, &prompt, "hello");
        match outcome {
            Outcome::TierExhausted { attempts } => assert_eq!(attempts.len(), 2),
            _ => panic!("expected TierExhausted"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn child_runs_in_a_throwaway_sandbox_not_the_callers_cwd() {
        let dir = temp_dir("sandbox");
        // The fake child reports its own cwd and drops a file there, imitating a
        // dispatched model that writes into whatever directory it lands in.
        let script = fake_dispatch_bin(
            &dir,
            "#!/bin/sh\npwd\ntouch sandbox-marker.txt\nexit 0\n",
        );
        let chain = vec![ModelEntry { model: "primary-model".into(), thinking: "low".into() }];
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain(script.to_str().unwrap(), &chain, &prompt, "hello");
        let child_cwd = match outcome {
            Outcome::Success { artifact, .. } => artifact.lines().next().unwrap().to_string(),
            _ => panic!("expected Success"),
        };
        let caller_cwd = std::env::current_dir().unwrap();
        assert_ne!(std::path::Path::new(&child_cwd), caller_cwd.as_path());
        assert!(child_cwd.contains("tier-dispatch-sandbox-"));
        assert!(!caller_cwd.join("sandbox-marker.txt").exists());
        assert!(!std::path::Path::new(&child_cwd).exists(), "sandbox should be removed after the attempt");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_quota_failure_stops_immediately_without_trying_fallback() {
        let dir = temp_dir("hard-failure");
        let script = fake_dispatch_bin(
            &dir,
            "#!/bin/sh\necho \"panic: something genuinely broke\" 1>&2\nexit 1\n",
        );
        let chain = vec![
            ModelEntry { model: "primary-model".into(), thinking: "low".into() },
            ModelEntry { model: "fallback-model".into(), thinking: "low".into() },
        ];
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain(script.to_str().unwrap(), &chain, &prompt, "hello");
        match outcome {
            Outcome::HardFailure { attempt } => assert_eq!(attempt.model, "primary-model"),
            _ => panic!("expected HardFailure on the primary, never reaching the fallback"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
