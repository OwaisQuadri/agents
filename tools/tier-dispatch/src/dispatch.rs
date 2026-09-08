use crate::config::ModelEntry;
use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::sys::signal::{SigSet, Signal, killpg};
use nix::unistd::Pid;
use std::io::{ErrorKind, Read};
use std::mem::MaybeUninit;
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::ptr;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

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
const POLL: Duration = Duration::from_millis(10);
const POST_EXIT_DRAIN: Duration = Duration::from_millis(100);

struct SignalState {
    is_interrupted: AtomicBool,
    active_group: AtomicI32,
}

impl SignalState {
    fn new() -> Self {
        Self {
            is_interrupted: AtomicBool::new(false),
            active_group: AtomicI32::new(0),
        }
    }

    fn terminate_active_group(&self, signal: Signal) {
        let group = self.active_group.load(Ordering::Acquire);
        if group > 0 {
            let _ = killpg(Pid::from_raw(group), signal);
        }
    }
}

static SIGNALS: OnceLock<Arc<SignalState>> = OnceLock::new();

fn signal_state() -> Arc<SignalState> {
    Arc::clone(SIGNALS.get_or_init(|| Arc::new(SignalState::new())))
}

pub fn install_signal_forwarder() -> Result<(), String> {
    let mut signals = SigSet::empty();
    signals.add(Signal::SIGINT);
    signals.add(Signal::SIGTERM);
    signals.thread_block().map_err(|error| error.to_string())?;
    let state = signal_state();
    thread::spawn(move || {
        loop {
            match signals.wait() {
                Ok(signal) => {
                    state.is_interrupted.store(true, Ordering::Release);
                    state.terminate_active_group(signal);
                }
                Err(error) => {
                    eprintln!("tier-dispatch: signal wait failed: {error}");
                    state.terminate_active_group(Signal::SIGKILL);
                    std::process::exit(1);
                }
            }
        }
    });
    Ok(())
}

fn is_retryable_model_error(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    RETRYABLE_MODEL_ERROR_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}
#[derive(Clone, Copy)]
struct Limits {
    attempt: Duration,
    chain: Duration,
    grace: Duration,
}
impl Limits {
    const PRODUCTION: Self = Self {
        attempt: Duration::from_secs(600),
        chain: Duration::from_secs(1800),
        grace: Duration::from_secs(2),
    };
}
pub struct Attempt {
    pub model: String,
    pub thinking: String,
    pub elapsed_ms: u128,
    pub result: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    is_timed_out: bool,
    is_wait_failed: bool,
}
pub enum Outcome {
    Success {
        model_ran: String,
        artifact: String,
        attempts: Vec<Attempt>,
    },
    TierExhausted {
        attempts: Vec<Attempt>,
    },
    TierTimedOut {
        attempts: Vec<Attempt>,
    },
    Interrupted {
        attempts: Vec<Attempt>,
    },
    HardFailure {
        attempt: Attempt,
        previous_attempts: Vec<Attempt>,
    },
}
pub fn walk_chain(
    dispatch_bin: &str,
    chain: &[ModelEntry],
    system_prompt_file: &Path,
    input: &str,
) -> Outcome {
    walk_chain_with_limits(
        dispatch_bin,
        chain,
        system_prompt_file,
        input,
        Limits::PRODUCTION,
    )
}
fn walk_chain_with_limits(
    dispatch_bin: &str,
    chain: &[ModelEntry],
    system_prompt_file: &Path,
    input: &str,
    limits: Limits,
) -> Outcome {
    let deadline = Instant::now() + limits.chain;
    let mut attempts = Vec::new();
    let mut is_any_timed_out = false;
    for entry in chain {
        if deadline.saturating_duration_since(Instant::now()) <= limits.grace {
            return Outcome::TierTimedOut { attempts };
        }
        let attempt = run_one(
            dispatch_bin,
            entry,
            system_prompt_file,
            input,
            limits,
            deadline,
        );
        if signal_state().is_interrupted.load(Ordering::Acquire) {
            attempts.push(attempt);
            return Outcome::Interrupted { attempts };
        }
        if attempt.exit_code == Some(0) && !attempt.is_timed_out && !attempt.is_wait_failed {
            let model_ran = attempt.model.clone();
            let artifact = attempt.stdout.clone();
            attempts.push(attempt);
            return Outcome::Success {
                model_ran,
                artifact,
                attempts,
            };
        }
        if attempt.is_timed_out {
            is_any_timed_out = true;
            attempts.push(attempt);
            if deadline.saturating_duration_since(Instant::now()) <= limits.grace {
                return Outcome::TierTimedOut { attempts };
            }
        } else if is_retryable_model_error(&attempt.stderr) {
            attempts.push(attempt)
        } else {
            return Outcome::HardFailure {
                attempt,
                previous_attempts: attempts,
            };
        }
    }
    if is_any_timed_out {
        Outcome::TierTimedOut { attempts }
    } else {
        if let Some(last) = attempts.last_mut() {
            last.result = "exhausted".to_string();
        }
        Outcome::TierExhausted { attempts }
    }
}
static SANDBOX_COUNTER: AtomicUsize = AtomicUsize::new(0);
fn make_sandbox() -> std::io::Result<PathBuf> {
    let n = SANDBOX_COUNTER.fetch_add(1, Ordering::SeqCst);
    let path =
        std::env::temp_dir().join(format!("tier-dispatch-sandbox-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&path)?;
    Ok(path)
}
enum DrainOutcome {
    Complete,
    Deadline,
    Stopped,
    Error(String),
}

struct Drain {
    data: Vec<u8>,
    outcome: DrainOutcome,
}

impl Drain {
    fn is_complete(&self) -> bool {
        matches!(self.outcome, DrainOutcome::Complete)
    }
}

fn drain(
    mut pipe: impl Read + AsFd + Send + 'static,
    deadline: Instant,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<Drain> {
    thread::spawn(move || {
        let mut data = Vec::new();
        let flags = match fcntl(&pipe, FcntlArg::F_GETFL) {
            Ok(flags) => flags,
            Err(error) => {
                return Drain {
                    data,
                    outcome: DrainOutcome::Error(format!("fcntl F_GETFL: {error}")),
                };
            }
        };
        if let Err(error) = fcntl(
            &pipe,
            FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
        ) {
            return Drain {
                data,
                outcome: DrainOutcome::Error(format!("fcntl F_SETFL: {error}")),
            };
        }
        let mut buffer = [0; 8192];
        let mut stop_deadline = None;
        loop {
            if stop_deadline.is_none() && stop.load(Ordering::Acquire) {
                stop_deadline = Instant::now().checked_add(POST_EXIT_DRAIN);
            }
            let active_deadline = stop_deadline.map_or(deadline, |stop| stop.min(deadline));
            if Instant::now() >= active_deadline {
                return Drain {
                    data,
                    outcome: if stop_deadline.is_some() {
                        DrainOutcome::Stopped
                    } else {
                        DrainOutcome::Deadline
                    },
                };
            }
            match pipe.read(&mut buffer) {
                Ok(0) => {
                    return Drain {
                        data,
                        outcome: DrainOutcome::Complete,
                    };
                }
                Ok(size) => data.extend_from_slice(&buffer[..size]),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(
                        POLL.min(active_deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(error) => {
                    return Drain {
                        data,
                        outcome: DrainOutcome::Error(format!("read: {error}")),
                    };
                }
            }
        }
    })
}
enum WaitOutcome {
    Exited(std::process::ExitStatus),
    Deadline,
    Interrupted,
}

fn wait_until(
    child: &mut std::process::Child,
    deadline: Instant,
    signals: &SignalState,
) -> Result<WaitOutcome, std::io::Error> {
    loop {
        if signals.is_interrupted.load(Ordering::Acquire) {
            return Ok(WaitOutcome::Interrupted);
        }
        if let Some(status) = child.try_wait()? {
            return Ok(WaitOutcome::Exited(status));
        }
        if Instant::now() >= deadline {
            return Ok(WaitOutcome::Deadline);
        }
        thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn terminate(
    child: &mut std::process::Child,
    deadline: Instant,
) -> Result<std::process::ExitStatus, std::io::Error> {
    let group = Pid::from_raw(child.id() as i32);
    let _ = killpg(group, Signal::SIGTERM);
    while Instant::now() < deadline {
        thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
    let _ = killpg(group, Signal::SIGKILL);
    child.wait()
}
fn run_one(
    dispatch_bin: &str,
    entry: &ModelEntry,
    system_prompt_file: &Path,
    input: &str,
    limits: Limits,
    total: Instant,
) -> Attempt {
    let started = Instant::now();
    // A fresh sandbox prevents graded attempts from editing the live checkout.
    // It also prevents a fallback from seeing files left by an earlier attempt.
    let sandbox = match make_sandbox() {
        Ok(path) => path,
        Err(error) => {
            return Attempt {
                model: entry.model.clone(),
                thinking: entry.thinking.clone(),
                elapsed_ms: started.elapsed().as_millis(),
                result: "hard_failure".to_string(),
                stdout: String::new(),
                stderr: format!("failed to create sandbox dir: {error}"),
                exit_code: None,
                is_timed_out: false,
                is_wait_failed: false,
            };
        }
    };
    let signals = signal_state();
    let mut command = Command::new(dispatch_bin);
    command
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
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    // SAFETY: the child runs only async-signal-safe libc calls between fork and exec.
    unsafe {
        command.pre_exec(|| {
            let mut empty = MaybeUninit::<nix::libc::sigset_t>::uninit();
            if nix::libc::sigemptyset(empty.as_mut_ptr()) != 0
                || nix::libc::sigprocmask(nix::libc::SIG_SETMASK, empty.as_ptr(), ptr::null_mut())
                    != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let result = match command.spawn() {
        Ok(mut child) => {
            let hard_deadline = Instant::now()
                .checked_add(limits.attempt)
                .unwrap_or(total)
                .min(total);
            let execution_deadline = hard_deadline
                .checked_sub(limits.grace)
                .filter(|deadline| *deadline > Instant::now())
                .unwrap_or_else(Instant::now);
            let group = Pid::from_raw(child.id() as i32);
            signals
                .active_group
                .store(group.as_raw(), Ordering::Release);
            if signals.is_interrupted.load(Ordering::Acquire) {
                signals.terminate_active_group(Signal::SIGTERM);
            }
            let stop_readers = Arc::new(AtomicBool::new(false));
            let stdout = drain(
                child.stdout.take().expect("piped stdout"),
                execution_deadline,
                Arc::clone(&stop_readers),
            );
            let stderr = drain(
                child.stderr.take().expect("piped stderr"),
                execution_deadline,
                Arc::clone(&stop_readers),
            );
            let (status, is_wall_clock_timeout, wait_error) =
                match wait_until(&mut child, execution_deadline, &signals) {
                    Ok(WaitOutcome::Exited(status)) => (Some(status), false, None),
                    Ok(WaitOutcome::Deadline) => match terminate(&mut child, hard_deadline) {
                        Ok(status) => (Some(status), true, None),
                        Err(error) => (None, false, Some(error)),
                    },
                    Ok(WaitOutcome::Interrupted) => {
                        let interrupt_deadline = (Instant::now() + limits.grace).min(hard_deadline);
                        match terminate(&mut child, interrupt_deadline) {
                            Ok(status) => (Some(status), false, None),
                            Err(error) => (None, false, Some(error)),
                        }
                    }
                    Err(error) => {
                        let termination_deadline =
                            (Instant::now() + limits.grace).min(hard_deadline);
                        let reap_error = terminate(&mut child, termination_deadline).err();
                        let message = reap_error.map_or_else(
                            || error.to_string(),
                            |reap_error| format!("{error}; failed to reap child: {reap_error}"),
                        );
                        (None, false, Some(std::io::Error::other(message)))
                    }
                };
            stop_readers.store(true, Ordering::Release);
            let stdout_drain = stdout.join().unwrap_or(Drain {
                data: Vec::new(),
                outcome: DrainOutcome::Error("reader thread panicked".to_string()),
            });
            let stderr_drain = stderr.join().unwrap_or(Drain {
                data: Vec::new(),
                outcome: DrainOutcome::Error("reader thread panicked".to_string()),
            });
            let is_output_complete = stdout_drain.is_complete() && stderr_drain.is_complete();
            let is_output_open = matches!(
                &stdout_drain.outcome,
                DrainOutcome::Deadline | DrainOutcome::Stopped
            ) || matches!(
                &stderr_drain.outcome,
                DrainOutcome::Deadline | DrainOutcome::Stopped
            );
            let stdout = String::from_utf8_lossy(&stdout_drain.data)
                .trim()
                .to_string();
            let mut stderr = String::from_utf8_lossy(&stderr_drain.data).to_string();
            if is_output_open {
                // After the direct child is reaped, an open pipe proves a group member remains; its process group keeps the leader's identifier reserved.
                let _ = killpg(group, Signal::SIGTERM);
                let _ = killpg(group, Signal::SIGKILL);
            }
            for outcome in [&stdout_drain.outcome, &stderr_drain.outcome] {
                match outcome {
                    DrainOutcome::Deadline => {
                        stderr.push_str("tier-dispatch: output reader reached its deadline\n");
                    }
                    DrainOutcome::Stopped => {
                        stderr.push_str(
                            "tier-dispatch: output descriptor remained open after child exit\n",
                        );
                    }
                    DrainOutcome::Error(error) => {
                        stderr.push_str(&format!("tier-dispatch: output reader failed: {error}\n"));
                    }
                    DrainOutcome::Complete => {}
                }
            }
            let is_wait_failed = wait_error.is_some() || !is_output_complete;
            if is_wall_clock_timeout {
                stderr.push_str("tier-dispatch: attempt timed out\n");
            }
            if let Some(error) = wait_error {
                stderr.push_str(&format!(
                    "tier-dispatch: failed while waiting for attempt: {error}\n"
                ));
            }
            signals.active_group.store(0, Ordering::Release);
            let is_timed_out = is_wall_clock_timeout || is_output_open;
            let exit_code = status.and_then(|status| status.code());
            let result = if signals.is_interrupted.load(Ordering::Acquire) {
                "interrupted"
            } else if is_timed_out {
                "timeout"
            } else if exit_code == Some(0) && !is_wait_failed {
                "success"
            } else if is_retryable_model_error(&stderr) {
                "fallback"
            } else {
                "hard_failure"
            };
            Attempt {
                model: entry.model.clone(),
                thinking: entry.thinking.clone(),
                elapsed_ms: started.elapsed().as_millis(),
                result: result.to_string(),
                stdout,
                stderr,
                exit_code,
                is_timed_out,
                is_wait_failed,
            }
        }
        Err(error) => Attempt {
            model: entry.model.clone(),
            thinking: entry.thinking.clone(),
            elapsed_ms: started.elapsed().as_millis(),
            result: "hard_failure".to_string(),
            stdout: String::new(),
            stderr: format!("failed to spawn {dispatch_bin}: {error}"),
            exit_code: None,
            is_timed_out: false,
            is_wait_failed: false,
        },
    };
    std::fs::remove_dir_all(sandbox).ok();
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

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
            "tier-dispatch-dispatch-test-{tag}-{}-{}",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::SeqCst)
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
    fn child_stdin_is_always_closed() {
        let dir = temp_dir("stdin-closed");
        let script = fake_dispatch_bin(
            &dir,
            "#!/bin/sh\nif read line; then echo received:$line; else echo stdin-closed; fi\n",
        );
        let prompt = write_system_prompt(&dir);
        let entry = ModelEntry {
            model: "primary-model".into(),
            thinking: "low".into(),
        };
        let attempt = run_one(
            script.to_str().unwrap(),
            &entry,
            &prompt,
            "hello",
            Limits::PRODUCTION,
            Instant::now() + Duration::from_secs(5),
        );
        assert_eq!(attempt.stdout, "stdin-closed");
        std::fs::remove_dir_all(&dir).ok();
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
                attempts,
            } => {
                assert_eq!(model_ran, "primary-model");
                assert!(artifact.contains("ran:primary-model"));
                assert_eq!(attempts.len(), 1);
                assert_eq!(attempts[0].result, "success");
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
                attempts,
            } => {
                assert_eq!(model_ran, "fallback-model");
                assert!(artifact.contains("ran:fallback-model"));
                assert_eq!(attempts.len(), 2);
                assert_eq!(attempts[0].result, "fallback");
                assert_eq!(attempts[1].result, "success");
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
            Outcome::HardFailure { attempt, .. } => assert_eq!(attempt.model, "mistyped-model"),
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
            Outcome::TierExhausted { attempts } => {
                assert_eq!(attempts.len(), 2);
                assert_eq!(attempts[0].result, "fallback");
                assert_eq!(attempts[1].result, "exhausted");
            }
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
            Outcome::TierExhausted { attempts } => {
                assert_eq!(attempts.len(), 2);
                assert_eq!(attempts[0].result, "fallback");
                assert_eq!(attempts[1].result, "exhausted");
            }
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
            Outcome::HardFailure {
                attempt,
                previous_attempts,
            } => {
                assert_eq!(attempt.model, "primary-model");
                assert!(previous_attempts.is_empty());
            }
            _ => panic!("expected HardFailure on the primary, never reaching the fallback"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    fn test_limits(attempt: u64, chain: u64) -> Limits {
        Limits {
            attempt: Duration::from_secs(attempt),
            chain: Duration::from_secs(chain),
            grace: Duration::from_millis(500),
        }
    }

    fn is_process_finished(pid: i32) -> bool {
        let output = Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        !output.status.success()
            || String::from_utf8_lossy(&output.stdout)
                .trim()
                .starts_with('Z')
    }

    fn wait_for_process_finished(pid: i32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if is_process_finished(pid) {
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
        is_process_finished(pid)
    }

    #[test]
    fn drain_stops_when_signaled_during_continuous_output() {
        let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let output = drain(
            reader,
            Instant::now() + Duration::from_secs(5),
            Arc::clone(&stop),
        );
        let writer = thread::spawn(move || {
            let data = [b'x'; 8192];
            while writer.write_all(&data).is_ok() {}
        });
        thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        stop.store(true, Ordering::Release);
        let output = output.join().unwrap();
        writer.join().unwrap();
        assert!(matches!(output.outcome, DrainOutcome::Stopped));
        assert!(!output.data.is_empty());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn drain_waits_for_the_post_exit_deadline_when_an_idle_pipe_stays_open() {
        let (reader, _writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let output = drain(
            reader,
            Instant::now() + Duration::from_secs(1),
            Arc::clone(&stop),
        );
        let started = Instant::now();
        stop.store(true, Ordering::Release);
        let output = output.join().unwrap();
        assert!(matches!(output.outcome, DrainOutcome::Stopped));
        assert!(started.elapsed() >= POST_EXIT_DRAIN);
    }

    #[test]
    fn hard_failure_preserves_previous_timeout_diagnostic() {
        let dir = temp_dir("timeout-then-hard-failure");
        let script = fake_dispatch_bin(
            &dir,
            "#!/bin/sh\nif [ \"$3\" = primary-model ]; then sleep 10; fi\necho hard-failure 1>&2\nexit 1\n",
        );
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain_with_limits(
            script.to_str().unwrap(),
            &[
                ModelEntry {
                    model: "primary-model".into(),
                    thinking: "low".into(),
                },
                ModelEntry {
                    model: "fallback-model".into(),
                    thinking: "low".into(),
                },
            ],
            &prompt,
            "hello",
            test_limits(2, 6),
        );
        match outcome {
            Outcome::HardFailure {
                attempt,
                previous_attempts,
            } => {
                assert_eq!(attempt.model, "fallback-model");
                assert_eq!(previous_attempts.len(), 1);
                assert!(previous_attempts[0].is_timed_out);
            }
            _ => panic!("expected the fallback hard failure"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn drain_stops_at_its_deadline_with_continuous_partial_output() {
        let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let writer = thread::spawn(move || {
            let data = [b'x'; 8192];
            while writer.write_all(&data).is_ok() {}
        });
        let started = Instant::now();
        let output = drain(
            reader,
            started + Duration::from_millis(500),
            Arc::new(AtomicBool::new(false)),
        )
        .join()
        .unwrap();
        let elapsed = started.elapsed();
        writer.join().unwrap();
        assert!(!output.data.is_empty());
        assert!(!output.is_complete());
        assert!(elapsed < Duration::from_secs(5));
    }

    #[test]
    fn hung_primary_then_fallback_success() {
        let dir = temp_dir("hung-fallback");
        let script = fake_dispatch_bin(
            &dir,
            "#!/bin/sh\nif [ \"$3\" = primary-model ]; then sleep 10; fi\necho ran:$3\n",
        );
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain_with_limits(
            script.to_str().unwrap(),
            &[
                ModelEntry {
                    model: "primary-model".into(),
                    thinking: "low".into(),
                },
                ModelEntry {
                    model: "fallback-model".into(),
                    thinking: "low".into(),
                },
            ],
            &prompt,
            "hello",
            test_limits(3, 10),
        );
        assert!(
            matches!(outcome, Outcome::Success { model_ran, .. } if model_ran == "fallback-model")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn slow_success_below_limit_is_accepted() {
        let dir = temp_dir("slow-success");
        let script = fake_dispatch_bin(&dir, "#!/bin/sh\nsleep 0.5\necho done\n");
        let prompt = write_system_prompt(&dir);
        let entry = ModelEntry {
            model: "primary-model".into(),
            thinking: "low".into(),
        };
        let outcome = walk_chain_with_limits(
            script.to_str().unwrap(),
            &[entry],
            &prompt,
            "hello",
            test_limits(3, 6),
        );
        assert!(matches!(outcome, Outcome::Success { .. }));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn drains_large_stdout_and_stderr_without_deadlock() {
        let dir = temp_dir("large-pipes");
        let script = fake_dispatch_bin(
            &dir,
            "#!/bin/sh\nyes o | head -c 131072\nyes e | head -c 131072 1>&2\n",
        );
        let prompt = write_system_prompt(&dir);
        let entry = ModelEntry {
            model: "primary-model".into(),
            thinking: "low".into(),
        };
        let attempt = run_one(
            script.to_str().unwrap(),
            &entry,
            &prompt,
            "hello",
            Limits::PRODUCTION,
            Instant::now() + Duration::from_secs(5),
        );
        assert_eq!(attempt.exit_code, Some(0));
        assert!(attempt.stdout.len() > 65_536);
        assert!(attempt.stderr.len() > 65_536);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn timeout_cleans_descendant_reaps_child_and_removes_sandbox() {
        let dir = temp_dir("descendant-cleanup");
        let pid_file = dir.join("descendant.pid");
        let cwd_file = dir.join("cwd");
        let script = fake_dispatch_bin(
            &dir,
            &format!(
                "#!/bin/sh\npwd > {}\nsleep 10 &\necho $! > {}\nwait\n",
                cwd_file.display(),
                pid_file.display()
            ),
        );
        let prompt = write_system_prompt(&dir);
        let entry = ModelEntry {
            model: "primary-model".into(),
            thinking: "low".into(),
        };
        let attempt = run_one(
            script.to_str().unwrap(),
            &entry,
            &prompt,
            "hello",
            test_limits(2, 6),
            Instant::now() + Duration::from_secs(6),
        );
        assert!(attempt.is_timed_out);
        assert!(!Path::new(std::fs::read_to_string(cwd_file).unwrap().trim()).exists());
        let pid = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(wait_for_process_finished(pid));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exiting_child_with_open_descendant_output_returns_promptly_and_cleans_group() {
        let dir = temp_dir("post-exit-descendant-cleanup");
        let pid_file = dir.join("descendant.pid");
        let script = fake_dispatch_bin(
            &dir,
            &format!(
                "#!/bin/sh\nsleep 30 &\necho $! > {}\necho done\nexit 0\n",
                pid_file.display()
            ),
        );
        let prompt = write_system_prompt(&dir);
        let entry = ModelEntry {
            model: "primary-model".into(),
            thinking: "low".into(),
        };
        let started = Instant::now();
        let attempt = run_one(
            script.to_str().unwrap(),
            &entry,
            &prompt,
            "hello",
            test_limits(4, 8),
            Instant::now() + Duration::from_secs(8),
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(attempt.exit_code, Some(0));
        assert!(attempt.stdout.contains("done"));
        assert!(attempt.is_timed_out);
        assert!(attempt.is_wait_failed);
        assert!(
            attempt
                .stderr
                .contains("output descriptor remained open after child exit")
        );
        let pid: i32 = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(wait_for_process_finished(pid));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn total_budget_prevents_later_fallback() {
        let dir = temp_dir("total-budget");
        let script = fake_dispatch_bin(
            &dir,
            "#!/bin/sh\nif [ \"$3\" = primary-model ]; then sleep 10; fi\necho fallback-ran\n",
        );
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain_with_limits(
            script.to_str().unwrap(),
            &[
                ModelEntry {
                    model: "primary-model".into(),
                    thinking: "low".into(),
                },
                ModelEntry {
                    model: "fallback-model".into(),
                    thinking: "low".into(),
                },
            ],
            &prompt,
            "hello",
            test_limits(3, 2),
        );
        assert!(matches!(outcome, Outcome::TierTimedOut { attempts } if attempts.len() == 1));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_attempt_timeout_reports_tier_timeout() {
        let dir = temp_dir("all-timeout");
        let script = fake_dispatch_bin(&dir, "#!/bin/sh\nsleep 10\n");
        let prompt = write_system_prompt(&dir);
        let outcome = walk_chain_with_limits(
            script.to_str().unwrap(),
            &[
                ModelEntry {
                    model: "primary-model".into(),
                    thinking: "low".into(),
                },
                ModelEntry {
                    model: "fallback-model".into(),
                    thinking: "low".into(),
                },
            ],
            &prompt,
            "hello",
            test_limits(2, 6),
        );
        assert!(matches!(outcome, Outcome::TierTimedOut { attempts } if attempts.len() == 2));
        std::fs::remove_dir_all(&dir).ok();
    }
}
