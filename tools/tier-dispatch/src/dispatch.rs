use crate::config::ModelEntry;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const RETRYABLE_MODEL_ERROR_MARKERS: &[&str] = &[
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
    "does not support this model",
];

fn is_retryable_model_error(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    RETRYABLE_MODEL_ERROR_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

pub struct Attempt {
    pub model: String,
    pub thinking: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

pub enum Outcome {
    Success { model_ran: String, artifact: String },
    TierExhausted { attempts: Vec<Attempt> },
    HardFailure { attempt: Attempt },
}

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
        if !is_retryable_model_error(&attempt.stderr) {
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
    let dir =
        std::env::temp_dir().join(format!("tier-dispatch-sandbox-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn run_one(
    dispatch_bin: &str,
    entry: &ModelEntry,
    system_prompt_file: &Path,
    input: &str,
) -> Attempt {
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
    let attempt = match output {
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
        assert!(is_retryable_model_error(
            "Error: rate limit exceeded, try again later"
        ));
        assert!(is_retryable_model_error("429 Too Many Requests"));
        assert!(is_retryable_model_error(
            "RESOURCE_EXHAUSTED: quota exceeded"
        ));
        assert!(is_retryable_model_error(
            "Usage limit reached for this account"
        ));
    }

    #[test]
    fn does_not_classify_an_unrelated_failure_as_quota() {
        assert!(!is_retryable_model_error("panic: index out of bounds"));
        assert!(!is_retryable_model_error("connection refused"));
        assert!(!is_retryable_model_error(""));
    }

    fn fake_dispatch_bin(dir: &Path, behavior: &str) -> std::path::PathBuf {
        let path = dir.join("fake-pi");
        std::fs::write(&path, behavior).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tier-dispatch-dispatch-test-{tag}-{}",
            std::process::id()
        ));
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
        let script = fake_dispatch_bin(&dir, "#!/bin/sh\necho \"ran:$3\"\nexit 0\n");
        let chain = vec![
            ModelEntry {
                model: "primary-model".into(),
                thinking: "low".into(),
            },
            ModelEntry {
                model: "fallback-model".into(),
                thinking: "low".into(),
            },
        ];
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain(script.to_str().unwrap(), &chain, &prompt, "hello");
        match outcome {
            Outcome::Success {
                model_ran,
                artifact,
            } => {
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
            ModelEntry {
                model: "primary-model".into(),
                thinking: "low".into(),
            },
            ModelEntry {
                model: "fallback-model".into(),
                thinking: "low".into(),
            },
        ];
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain(script.to_str().unwrap(), &chain, &prompt, "hello");
        match outcome {
            Outcome::Success {
                model_ran,
                artifact,
            } => {
                assert_eq!(model_ran, "fallback-model");
                assert!(artifact.contains("ran:fallback-model"));
            }
            _ => panic!("expected Success on fallback, got a different outcome"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unsupported_model_failure_walks_to_fallback() {
        let dir = temp_dir("unsupported-model-walk");
        let script = fake_dispatch_bin(
            &dir,
            r#"#!/bin/sh
model="$3"
if [ "$model" = "primary-model" ]; then
  echo "Claude Code does not support this model; version 2.1.251 or newer is required" 1>&2
  exit 1
fi
echo "ran:$model"
exit 0
"#,
        );
        let chain = vec![
            ModelEntry {
                model: "primary-model".into(),
                thinking: "low".into(),
            },
            ModelEntry {
                model: "fallback-model".into(),
                thinking: "low".into(),
            },
        ];
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain(script.to_str().unwrap(), &chain, &prompt, "hello");
        match outcome {
            Outcome::Success { model_ran, .. } => assert_eq!(model_ran, "fallback-model"),
            _ => panic!("expected Success on fallback, got a different outcome"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_model_is_a_hard_config_failure() {
        let dir = temp_dir("unknown-model");
        let script = fake_dispatch_bin(
            &dir,
            "#!/bin/sh\necho \"error: model not found: $3\" 1>&2\nexit 1\n",
        );
        let chain = vec![
            ModelEntry {
                model: "mistyped-model".into(),
                thinking: "low".into(),
            },
            ModelEntry {
                model: "fallback-model".into(),
                thinking: "low".into(),
            },
        ];
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain(script.to_str().unwrap(), &chain, &prompt, "hello");
        match outcome {
            Outcome::HardFailure { attempt } => assert_eq!(attempt.model, "mistyped-model"),
            _ => panic!("expected the invalid model configuration to stop the chain"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn quota_failure_still_tries_later_models_from_the_same_provider() {
        let dir = temp_dir("same-provider-fallback");
        let script = fake_dispatch_bin(&dir, "#!/bin/sh\necho \"quota exceeded\" 1>&2\nexit 1\n");
        let chain = vec![
            ModelEntry {
                model: "anthropic/new".into(),
                thinking: "low".into(),
            },
            ModelEntry {
                model: "anthropic/old".into(),
                thinking: "low".into(),
            },
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
    fn quota_failure_on_every_model_in_chain_reports_tier_exhausted() {
        let dir = temp_dir("quota-exhausted");
        let script = fake_dispatch_bin(&dir, "#!/bin/sh\necho \"quota exceeded\" 1>&2\nexit 1\n");
        let chain = vec![
            ModelEntry {
                model: "primary-model".into(),
                thinking: "low".into(),
            },
            ModelEntry {
                model: "fallback-model".into(),
                thinking: "low".into(),
            },
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
        let script = fake_dispatch_bin(&dir, "#!/bin/sh\npwd\ntouch sandbox-marker.txt\nexit 0\n");
        let chain = vec![ModelEntry {
            model: "primary-model".into(),
            thinking: "low".into(),
        }];
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
        assert!(
            !std::path::Path::new(&child_cwd).exists(),
            "sandbox should be removed after the attempt"
        );
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
            ModelEntry {
                model: "primary-model".into(),
                thinking: "low".into(),
            },
            ModelEntry {
                model: "fallback-model".into(),
                thinking: "low".into(),
            },
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
