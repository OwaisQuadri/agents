//! PreToolUse(Bash) hook: refuse a `git commit` that would build a Rust crate with a
//! compiler warning.
//!
//! stdin  {"tool_input":{"command":"..."}}
//! stdout {"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny",...}}
//!
//! Always exits 0. A deny travels in the JSON, never in the exit code, so a parse bug
//! or a missing `cargo` degrades to silence rather than wedging every Bash call in the
//! session — mirrors `tools/no-ai-attribution`'s own contract.
//!
//! Gated on `git commit` only, and only when the commit actually stages a `.rs` file —
//! most commits in this repo touch no Rust, and this check pays for a `cargo build`
//! per staged Rust crate, so the cheap early exits matter.
//!
//! Enforces `docs/code-style.md`'s "warnings as errors" rule (see NASA(National
//! Aeronautics and Space Administration) JPL(Jet Propulsion Laboratory) Power of 10
//! Rule 10 and NVIDIA's `nvcc --Werror` precedent cited there) for every crate under
//! `tools/` — the only Rust crates this repo builds.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// 10s under the 120s hook timeout in config/settings.json and
// config/managed-settings.json. The CLI's PreToolUse timeout is fail-open: it kills
// the hook and lets the commit through with no trace. This deadline fires first, so a
// slow cold build denies loudly instead of skipping enforcement silently.
const BUILD_BUDGET: Duration = Duration::from_secs(110);

const VALUE_OPTIONS: &[&str] = &[
    "-c",
    "-C",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--exec-path",
    "--config-env",
    "-R",
    "--repo",
];

fn main() {
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return;
    }
    let Some(command) = extract_command(&raw) else {
        return;
    };
    if !is_git_commit(&command) {
        return;
    }

    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let Some(repo_root) = git_toplevel(&cwd) else {
        return;
    };
    let Some(staged) = staged_files(&repo_root) else {
        return;
    };
    let crates = staged_tool_crates(&staged);
    if crates.is_empty() {
        return;
    }

    let outcome = check_crates(&repo_root.join("tools"), &crates, BUILD_BUDGET);
    if outcome.failures.is_empty() && outcome.unchecked.is_empty() {
        return;
    }

    let mut reason = String::new();
    if !outcome.failures.is_empty() {
        reason.push_str(
            "Blocked: a staged commit would build a Rust crate with a compiler warning.\n\n\
             Standing rule: docs/code-style.md's \"warnings as errors\" bullet — fix every \
             rustc warning, or mark an intentional exception with #[expect(..., reason = \
             \"...\")] per skills/rust-style/rust-baseline.md.\n",
        );
        for failure in &outcome.failures {
            reason.push_str(&format!(
                "\n--- {} ---\n{}\n",
                failure.crate_name,
                failure.output.trim()
            ));
        }
    }
    if !outcome.unchecked.is_empty() {
        if !reason.is_empty() {
            reason.push('\n');
        }
        reason.push_str(&format!(
            "Blocked: warnings-check hit its {}s build budget before these staged \
             crate(s) finished building: {}.\n\nBuild each one yourself with \
             RUSTFLAGS='-D warnings' cargo build --manifest-path \
             tools/<crate>/Cargo.toml, fix any warning, then retry the commit — the \
             warm cache makes the retry fast.\n",
            BUILD_BUDGET.as_secs(),
            outcome.unchecked.join(", ")
        ));
    }

    let summary = if outcome.failures.is_empty() {
        "Blocked a commit: warnings-check ran out of build time before checking every \
         staged crate."
    } else {
        "Blocked a commit: a Rust crate under tools/ builds with a warning."
    };
    print!(
        "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\
         \"permissionDecision\":\"deny\",\"permissionDecisionReason\":{}}},\
         \"systemMessage\":{}}}",
        json_quote(&reason),
        json_quote(summary)
    );
}

struct CrateFailure {
    crate_name: String,
    output: String,
}

struct CheckOutcome {
    failures: Vec<CrateFailure>,
    unchecked: Vec<String>,
}

/// Names of `tools/<name>/` directories that own at least one staged `.rs` path, in
/// first-seen order with no duplicates. Only these crates get built — not every crate
/// under `tools/` — so an unrelated crate's pre-existing warning can never deny a
/// commit that never touched it, and the common case (one crate touched) pays for one
/// `cargo build`, not the whole directory.
fn staged_tool_crates(paths: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for path in paths {
        if !path.ends_with(".rs") {
            continue;
        }
        let Some(rest) = path.strip_prefix("tools/") else {
            continue;
        };
        let Some(name) = rest.split('/').next() else {
            continue;
        };
        if !out.iter().any(|existing: &String| existing == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// Runs `cargo build` with warnings promoted to errors for each named crate directly
/// under `tools_dir/<name>/`, within one shared wall-clock `budget`. Returns the
/// failed builds plus the crates the budget ran out on — both empty when `cargo` is
/// missing or every named crate builds clean in time. A crate with no `Cargo.toml`
/// (already deleted, or the name doesn't match a real directory) is silently skipped
/// rather than treated as a failure.
fn check_crates(tools_dir: &Path, crate_names: &[String], budget: Duration) -> CheckOutcome {
    let deadline = Instant::now() + budget;
    let mut failures = Vec::new();
    let mut unchecked = Vec::new();
    for crate_name in crate_names {
        let manifest = tools_dir.join(crate_name).join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        if Instant::now() >= deadline {
            unchecked.push(crate_name.clone());
            continue;
        }
        match run_build(&manifest, deadline) {
            // cargo itself missing, or failed to spawn for this crate — degrade to
            // silence for this crate rather than deny on an infrastructure problem.
            None => continue,
            Some(Build::TimedOut) => unchecked.push(crate_name.clone()),
            Some(Build::Finished { is_success: true, .. }) => {}
            Some(Build::Finished { stderr, .. }) => failures.push(CrateFailure {
                crate_name: crate_name.clone(),
                output: stderr,
            }),
        }
    }
    CheckOutcome { failures, unchecked }
}

enum Build {
    Finished { is_success: bool, stderr: String },
    TimedOut,
}

/// Builds one crate with warnings promoted to errors, killing the build at `deadline`.
/// Returns `None` when the build fails to spawn or its status cannot be read.
fn run_build(manifest: &Path, deadline: Instant) -> Option<Build> {
    let mut child = Command::new("cargo")
        .arg("build")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(manifest)
        .env("RUSTFLAGS", "-D warnings")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stderr_pipe = child.stderr.take()?;
    // Drained on its own thread so a build with more diagnostics than the pipe buffer
    // holds cannot block against a full pipe and stall past the deadline.
    let reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buffer);
        buffer
    });
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr = reader
                    .join()
                    .map(|buffer| String::from_utf8_lossy(&buffer).to_string())
                    .unwrap_or_default();
                return Some(Build::Finished {
                    is_success: status.success(),
                    stderr,
                });
            }
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Some(Build::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn staged_files(repo_root: &Path) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("diff")
        .arg("--cached")
        .arg("--name-only")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.lines().map(str::to_string).collect())
}

fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(PathBuf::from(text.trim()))
}

fn is_git_commit(command: &str) -> bool {
    let lowered = command.to_lowercase();
    let tokens: Vec<&str> = lowered
        .split_whitespace()
        .map(|token| token.trim_matches(|c| c == '"' || c == '\''))
        .collect();
    is_verb_sequence(&tokens, &["git", "commit"])
}

/// Anchors on the program name, then walks past its options, so a flag standing
/// between the program and its subcommand cannot hide the subcommand. Copied from
/// `tools/no-ai-attribution/src/main.rs`, narrowed to the one verb sequence this
/// checker gates on.
fn is_verb_sequence(tokens: &[&str], verbs: &[&str]) -> bool {
    for (start, token) in tokens.iter().enumerate() {
        if *token != verbs[0] {
            continue;
        }
        let mut index = start + 1;
        let mut matched = 1;
        while index < tokens.len() && matched < verbs.len() {
            let token = tokens[index];
            if token.starts_with('-') {
                if VALUE_OPTIONS.contains(&token) {
                    index += 1;
                }
                index += 1;
                continue;
            }
            if token != verbs[matched] {
                break;
            }
            matched += 1;
            index += 1;
        }
        if matched == verbs.len() {
            return true;
        }
    }
    false
}

fn extract_command(payload: &str) -> Option<String> {
    let scope = payload.find("\"tool_input\"").unwrap_or(0);
    let key = payload[scope..].find("\"command\"")? + scope;
    let open = payload[key + "\"command\"".len()..].find('"')? + key + "\"command\"".len();
    decode_json_string(&payload[open + 1..])
}

fn decode_json_string(rest: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => {
                    let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                    let point = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(point).unwrap_or('\u{fffd}'));
                }
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

fn json_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_commit_is_gated() {
        assert!(is_git_commit("git commit -m x"));
    }

    #[test]
    fn a_flag_between_program_and_subcommand_cannot_hide_it() {
        assert!(is_git_commit("git -C ~/repo commit -m x"));
        assert!(is_git_commit("git --no-pager commit -m x"));
    }

    #[test]
    fn quoting_and_chaining_do_not_hide_it() {
        assert!(is_git_commit("sh -c \"git commit -m x\""));
        assert!(is_git_commit("git add -A && git commit -m x"));
    }

    #[test]
    fn other_git_verbs_are_not_gated() {
        assert!(!is_git_commit("git log --oneline"));
        assert!(!is_git_commit("git status"));
        assert!(!is_git_commit("git diff --cached"));
        assert!(!is_git_commit("gh pr create --body x"));
    }

    #[test]
    fn rust_file_staged_is_detected() {
        assert_eq!(
            staged_tool_crates(&["tools/foo/src/main.rs".to_string()]),
            vec!["foo".to_string()]
        );
        assert!(staged_tool_crates(&["README.md".to_string(), "docs/x.md".to_string()]).is_empty());
        assert!(staged_tool_crates(&[]).is_empty());
    }

    #[test]
    fn only_the_staged_crates_are_named_no_duplicates() {
        let staged = vec![
            "tools/foo/src/main.rs".to_string(),
            "tools/foo/src/lib.rs".to_string(),
            "tools/bar/src/main.rs".to_string(),
            "docs/unrelated.md".to_string(),
        ];
        assert_eq!(staged_tool_crates(&staged), vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn a_staged_rust_file_outside_tools_names_no_crate() {
        assert!(staged_tool_crates(&["pi/extensions/foo.rs".to_string()]).is_empty());
    }

    #[test]
    fn an_exhausted_budget_marks_the_crate_unchecked_not_failed() {
        let dir = std::env::temp_dir().join(format!("warnings-check-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("foo")).unwrap();
        std::fs::write(dir.join("foo/Cargo.toml"), "[package]\nname = \"foo\"\n").unwrap();
        let outcome = check_crates(&dir, &["foo".to_string()], Duration::ZERO);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(outcome.failures.is_empty());
        assert_eq!(outcome.unchecked, vec!["foo".to_string()]);
    }

    #[test]
    fn a_missing_manifest_is_skipped_not_marked_unchecked() {
        let dir = std::env::temp_dir().join("warnings-check-test-none");
        let outcome = check_crates(&dir, &["ghost".to_string()], Duration::ZERO);
        assert!(outcome.failures.is_empty());
        assert!(outcome.unchecked.is_empty());
    }

    #[test]
    fn extracts_command_from_pretooluse_payload() {
        let payload = r#"{"tool_name":"Bash","tool_input":{"command":"git commit -m x"}}"#;
        assert_eq!(extract_command(payload).as_deref(), Some("git commit -m x"));
    }
}
