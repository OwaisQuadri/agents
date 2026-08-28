//! Checks that every skill/agent/workflow definition's `## logging` section points its
//! `logs/usage.jsonl` append at a path anchored to the repo root, matching the
//! artifact's own directory.
//!
//! Root cause this guards: a bare `logs/usage.jsonl` (or one only anchored at the repo
//! root, e.g. `skills/foo/logs/usage.jsonl`) resolves against whatever the caller's
//! current working directory happens to be. When that isn't the repo root, the line
//! lands somewhere like `agents/skills/foo/logs/usage.jsonl` instead of the real log —
//! silently splitting an artifact's usage history across two files.
//!
//! The anchor must be derived, never a literal absolute path: a repo checkout does not
//! sit at the same path on every device. `git rev-parse --show-toplevel` is the accepted
//! derivation. A literal `~/Documents/agents` still passes too, since most of the repo's
//! existing `prompt_version` git commands hardcode it — that is a separate, wider
//! cleanup this checker does not force.
//!
//! Two modes:
//! - `logpath-check <root>` — the static scan above, run on demand against every SKILL.md
//!   / agent.md in the repo.
//! - `logpath-check <root> --validate-path <resolved-absolute-path>` — a runtime check of
//!   ONE concrete path a bash tool call is about to write to, used by the
//!   `pi/extensions/logpath-guard.ts` runtime guard to block the write before it happens
//!   rather than catch the drift later in prose. The caller (the extension) resolves any
//!   `cd`, `~`, or `$HOME` in the shell command first — this mode takes an already-lexical
//!   absolute path and checks only the STRUCTURE: does it land under `<root>/(skills|
//!   agents|workflows)/<name>/logs/usage.jsonl`, and does `<name>` name a real, existing
//!   artifact directory. That structural check alone catches the original bug: a path
//!   like `<root>/agents/skills/rust-style/logs/usage.jsonl` has an extra path segment
//!   and fails the pattern outright.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const ANCHORS: &[&str] = &["git rev-parse --show-toplevel", "~/Documents/agents"];
const HEADING: &str = "## logging";

struct Finding {
    file: PathBuf,
    reason: String,
}

fn find_candidates(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_glob(&root.join("skills"), "SKILL.md", &mut out);
    collect_glob(&root.join("workflows"), "SKILL.md", &mut out);
    collect_agent_files(&root.join("agents"), &mut out);
    out
}

fn collect_glob(parent: &Path, filename: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        if dir.file_name().and_then(|n| n.to_str()) == Some("templates") {
            continue;
        }
        let candidate = dir.join(filename);
        if candidate.is_file() {
            out.push(candidate);
        }
    }
}

fn collect_agent_files(parent: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(files) = fs::read_dir(&dir) else {
            continue;
        };
        for file_entry in files.flatten() {
            let path = file_entry.path();
            let is_md = path.extension().and_then(|e| e.to_str()) == Some("md");
            let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
            let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if is_md && name == dir_name {
                out.push(path);
            }
        }
    }
}

/// Extracts the body of the "## logging" section: from the LAST line that is exactly a
/// "## logging" heading (not a substring inside other prose, e.g. `### The "## logging"
/// section`) to the next "## " heading or end of file. Last, not first, because an
/// authoring skill's own body legitimately discusses the convention by name before its
/// own trailing section.
fn logging_section(content: &str) -> Option<&str> {
    let heading_start = content
        .lines()
        .scan(0usize, |offset, line| {
            let start = *offset;
            *offset += line.len() + 1;
            Some((start, line))
        })
        .filter(|(_, line)| line.trim().eq_ignore_ascii_case(HEADING))
        .map(|(start, _)| start)
        .last()?;
    let after_heading = &content[heading_start + HEADING.len()..];
    let end = after_heading.find("\n## ").unwrap_or(after_heading.len());
    Some(&after_heading[..end])
}

fn check_file(root: &Path, file: &Path) -> Option<Finding> {
    let content = fs::read_to_string(file).ok()?;
    let canonical_file = fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let rel = canonical_file.strip_prefix(root).unwrap_or(file);
    let artifact_dir = rel.parent()?.to_string_lossy().replace('\\', "/");

    let Some(section) = logging_section(&content) else {
        return Some(Finding {
            file: rel.to_path_buf(),
            reason: "no \"## logging\" section found".to_string(),
        });
    };

    let expected_rel_path = format!("{artifact_dir}/logs/usage.jsonl");
    let matched_anchor = ANCHORS.iter().find(|a| section.contains(**a));
    let has_own_path = section.contains(&expected_rel_path);

    if matched_anchor.is_some() && has_own_path {
        return None;
    }

    let anchor_list = ANCHORS.join("` / `");
    let reason = match (matched_anchor, has_own_path) {
        (None, false) => format!(
            "logging section has neither a repo-root anchor (`{anchor_list}`) nor the \
             literal path `{expected_rel_path}` — likely a bare `logs/usage.jsonl` that \
             resolves against the caller's cwd"
        ),
        (None, true) => format!(
            "logging section has `{expected_rel_path}` but never anchors it to the repo \
             root (`{anchor_list}`) — safe only if every caller's cwd is guaranteed to be \
             the repo root"
        ),
        (Some(anchor), false) => format!(
            "logging section has the `{anchor}` anchor but never the literal path \
             `{expected_rel_path}` — check the path matches this artifact's own directory"
        ),
        (Some(_), true) => unreachable!(),
    };

    Some(Finding {
        file: rel.to_path_buf(),
        reason,
    })
}

/// Resolves `.` and `..` components lexically, without touching the filesystem — the
/// target file usually does not exist yet (this runs BEFORE the write), so
/// `fs::canonicalize` would fail on it. Not symlink-safe; that tradeoff is fine here
/// because this checks path STRUCTURE (which artifact directory a write claims to
/// belong to), not filesystem identity.
fn resolve_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Runtime check for ONE concrete path a bash tool call is about to write to. `target`
/// must already be absolute and shell-resolved (no `~`, `$HOME`, or `cd` left in it) —
/// resolving shell syntax is the caller's job, this only checks structure.
fn validate_single_path(root: &Path, target: &Path) -> Result<(), String> {
    if !target.is_absolute() {
        return Err(format!(
            "target path `{}` is not absolute",
            target.display()
        ));
    }
    let resolved = resolve_lexical(target);
    let Ok(rel) = resolved.strip_prefix(root) else {
        return Err(format!(
            "target path `{}` resolves outside the repo root `{}`",
            resolved.display(),
            root.display()
        ));
    };
    let parts: Vec<&str> = rel
        .to_str()
        .unwrap_or_default()
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let [kind, name, "logs", "usage.jsonl"] = parts.as_slice() else {
        return Err(format!(
            "`{}` does not match <skills|agents|workflows>/<name>/logs/usage.jsonl — \
             likely resolved from an unanchored relative path against the wrong cwd",
            rel.display()
        ));
    };
    if !matches!(*kind, "skills" | "agents" | "workflows") {
        return Err(format!(
            "`{}` starts with `{kind}/`, not one of skills/agents/workflows",
            rel.display()
        ));
    }
    let artifact_dir = root.join(kind).join(name);
    if !artifact_dir.is_dir() {
        return Err(format!(
            "`{}/{name}/` does not exist — refusing to create a logs/ dir for an artifact \
             that isn't there",
            root.join(kind).display()
        ));
    }
    let marker_exists = if *kind == "agents" {
        artifact_dir.join(format!("{name}.md")).is_file()
    } else {
        artifact_dir.join("SKILL.md").is_file()
    };
    if !marker_exists {
        return Err(format!(
            "`{}/{name}/` exists but has no {} — not a real artifact directory",
            root.join(kind).display(),
            if *kind == "agents" {
                format!("{name}.md")
            } else {
                "SKILL.md".to_string()
            }
        ));
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let root_arg = args.next().unwrap_or_else(|| ".".to_string());
    let root = fs::canonicalize(&root_arg).unwrap_or_else(|_| PathBuf::from(&root_arg));

    match args.next().as_deref() {
        Some("--validate-path") => {
            let Some(target_arg) = args.next() else {
                eprintln!("--validate-path requires a path argument");
                return ExitCode::FAILURE;
            };
            match validate_single_path(&root, Path::new(&target_arg)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(reason) => {
                    println!("FAIL {target_arg}: {reason}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            let mut findings = Vec::new();
            for file in find_candidates(&root) {
                if let Some(finding) = check_file(&root, &file) {
                    findings.push(finding);
                }
            }

            if findings.is_empty() {
                return ExitCode::SUCCESS;
            }

            for finding in &findings {
                println!("FAIL {}: {}", finding.file.display(), finding.reason);
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_logging_section_up_to_next_heading() {
        let content = "# Title\n\n## logging\n\nbody line one\nbody line two\n\n## next\n\nother";
        let section = logging_section(content).unwrap();
        assert!(section.contains("body line one"));
        assert!(!section.contains("other"));
    }

    #[test]
    fn logging_section_missing_returns_none() {
        assert!(logging_section("# Title\n\nno logging here\n").is_none());
    }

    #[test]
    fn logging_section_runs_to_eof_when_last() {
        let content = "## logging\n\nonly section, no trailing heading";
        let section = logging_section(content).unwrap();
        assert!(section.contains("only section"));
    }

    #[test]
    fn portable_git_toplevel_anchor_passes() {
        let dir = std::env::temp_dir().join(format!(
            "logpath-check-test-pass-git-{}",
            std::process::id()
        ));
        let skill_dir = dir.join("skills").join("demo");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "## logging\n\nappend to `<repo-root>/skills/demo/logs/usage.jsonl`, where \
             `<repo-root>` is `git rev-parse --show-toplevel`\n",
        )
        .unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        let finding = check_file(&root, &skill_dir.join("SKILL.md"));
        assert!(finding.is_none(), "{:?}", finding.map(|f| f.reason));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn legacy_hardcoded_home_anchor_still_passes() {
        let dir = std::env::temp_dir().join(format!(
            "logpath-check-test-pass-legacy-{}",
            std::process::id()
        ));
        let skill_dir = dir.join("skills").join("demo");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "## logging\n\nappend to `~/Documents/agents/skills/demo/logs/usage.jsonl`\n",
        )
        .unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        let finding = check_file(&root, &skill_dir.join("SKILL.md"));
        assert!(finding.is_none(), "{:?}", finding.map(|f| f.reason));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bare_relative_path_fails() {
        let dir =
            std::env::temp_dir().join(format!("logpath-check-test-fail-{}", std::process::id()));
        let skill_dir = dir.join("skills").join("demo");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "## logging\n\nappend to `logs/usage.jsonl`\n",
        )
        .unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        let finding = check_file(&root, &skill_dir.join("SKILL.md")).unwrap();
        assert!(finding.reason.contains("neither"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_single_path_accepts_real_skill_dir() {
        let dir = std::env::temp_dir().join(format!(
            "logpath-check-test-validate-ok-{}",
            std::process::id()
        ));
        let skill_dir = dir.join("skills").join("demo");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# demo\n").unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        let target = root.join("skills/demo/logs/usage.jsonl");
        assert_eq!(validate_single_path(&root, &target), Ok(()));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_single_path_rejects_the_original_bug_shape() {
        let dir = std::env::temp_dir().join(format!(
            "logpath-check-test-validate-bug-{}",
            std::process::id()
        ));
        let skill_dir = dir.join("skills").join("rust-style");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# rust-style\n").unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        // the real incident: cwd was `agents/` when the caller wrote a path relative to
        // the repo root, landing at agents/skills/rust-style/logs/usage.jsonl.
        let target = root.join("agents/skills/rust-style/logs/usage.jsonl");
        let err = validate_single_path(&root, &target).unwrap_err();
        assert!(err.contains("does not match"), "{err}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_single_path_rejects_nonexistent_artifact() {
        let dir = std::env::temp_dir().join(format!(
            "logpath-check-test-validate-noexist-{}",
            std::process::id()
        ));
        fs::create_dir_all(dir.join("skills")).unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        let target = root.join("skills/nonexistent/logs/usage.jsonl");
        let err = validate_single_path(&root, &target).unwrap_err();
        assert!(err.contains("does not exist"), "{err}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_single_path_rejects_path_outside_root() {
        let dir = std::env::temp_dir().join(format!(
            "logpath-check-test-validate-outside-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        let target = PathBuf::from("/tmp/elsewhere/skills/demo/logs/usage.jsonl");
        let err = validate_single_path(&root, &target).unwrap_err();
        assert!(err.contains("outside the repo root"), "{err}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_single_path_resolves_dot_dot_lexically() {
        let dir = std::env::temp_dir().join(format!(
            "logpath-check-test-validate-dotdot-{}",
            std::process::id()
        ));
        let skill_dir = dir.join("skills").join("demo");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# demo\n").unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        // agents/../skills/demo/logs/usage.jsonl should resolve to skills/demo/...
        let target = root.join("agents/../skills/demo/logs/usage.jsonl");
        assert_eq!(validate_single_path(&root, &target), Ok(()));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_section_fails() {
        let dir =
            std::env::temp_dir().join(format!("logpath-check-test-missing-{}", std::process::id()));
        let skill_dir = dir.join("skills").join("demo");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# Demo\n\nno logging section\n").unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        let finding = check_file(&root, &skill_dir.join("SKILL.md")).unwrap();
        assert!(finding.reason.contains("no \"## logging\" section"));
        fs::remove_dir_all(&dir).ok();
    }
}
