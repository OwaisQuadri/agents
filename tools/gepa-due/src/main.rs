//! Reports which artifacts (skills/agents/workflows with an `evals/` dir) have
//! accumulated enough real, unacted-on evidence since their last GEPA tune to be worth
//! a tuning pass — an accumulation trigger, not a time trigger. Fired daily by
//! `workflows/gepa-due/scripts/trigger.sh`, but this binary itself does the actual
//! deciding: the trigger script only escalates to opening a Pi session when this prints
//! a non-empty due list, never on a schedule alone.
//!
//! "Since their last tune" is filtered the same way `ai-author/SKILL.md`'s own GEPA
//! loop step 1 (Reflect) already filters by hand: a `logs/usage.jsonl` line counts only
//! if its `prompt_version` field matches the artifact's CURRENT prompt_version (the
//! short commit of the last change to its own definition, excluding evals/TUNING.md/
//! logs/votes) — a line written against a prompt that no longer exists is stale
//! evidence, counted and dropped, not evidence of anything about the live artifact.
//! Same filter for `votes/votes.jsonl`, whose `vote` field is required (by
//! `scripts/submit_vote.py`'s caller contract) to start with `prompt_version: <sha>`.
//!
//! Thresholds are fixed constants, not derived from any research (see
//! `PLAN.md`/`ai-author/TUNING.md` for why): >=15 surviving usage lines, or >=2
//! surviving votes, marks an artifact "due".
//!
//! Reports ONLY the due artifacts — "a checker reports only its failures, never its
//! passes" per `ai-author/SKILL.md`'s own rule. An artifact under the threshold is
//! absent from the output entirely, not listed with a false/no-op verdict.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE_THRESHOLD: usize = 15;
const VOTE_THRESHOLD: usize = 2;
const KINDS: &[&str] = &["skills", "agents", "workflows"];

struct DueArtifact {
    artifact: String,
    usage_count: usize,
    vote_count: usize,
    reason: String,
}

fn discover_artifacts(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for kind in KINDS {
        let Ok(entries) = fs::read_dir(root.join(kind)) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() || !dir.join("evals").is_dir() {
                continue;
            }
            let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
            out.push((format!("{kind}/{name}"), dir));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Mirrors the `git log` invocation every artifact's own `## logging` section
/// documents: the commit of the last change to the artifact's OWN files, excluding
/// its harness, tuning record, and accumulated logs/votes — those change on every run
/// and would make prompt_version churn constantly if included.
///
/// Returns the FULL hash, not `%h`. Every artifact's own logging section (and
/// `scripts/submit_vote.py`) logs the ABBREVIATED `%h`, and git's abbreviation length
/// is not fixed — it grows as the repo needs more characters to stay unique, so the
/// exact same commit can be logged under different-length prefixes at different
/// points in time (`76a831fd`, then later `76a831fda`, then `76a831fda8`, all one
/// commit). Matching callers compare a logged short hash against this FULL hash with
/// `starts_with`, which is correct regardless of which length any given line used —
/// exact-string equality between two independently-abbreviated hashes silently
/// undercounts surviving evidence (confirmed live 2026-08-29: 65 exact-match lines vs
/// 183 real current-commit lines once all three logged prefix lengths were counted).
fn current_prompt_version(root: &Path, artifact_rel: &str) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "log",
            "-1",
            "--format=%H",
            "--",
            artifact_rel,
            ":(exclude)**/evals/**",
            ":(exclude)**/TUNING.md",
            ":(exclude)**/logs/**",
            ":(exclude)**/votes/**",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if sha.is_empty() { None } else { Some(sha) }
}

/// Extracts the raw (still JSON-escaped) string value of `"key":"..."` in one JSON
/// object line. No full JSON parser: usage.jsonl/votes.jsonl are both flat,
/// single-line objects this repo itself writes, so a scan for the key is sufficient
/// and avoids a dependency. Handles `\"` inside the value; does not unescape `\n` etc,
/// since callers only need prefix/equality checks on the raw escaped form.
fn extract_json_string_field(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let key_pos = line.find(&needle)?;
    let after_key = &line[key_pos + needle.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();
    let mut chars = after_colon.char_indices();
    let (_, quote) = chars.next()?;
    if quote != '"' {
        return None;
    }
    let mut value = String::new();
    let mut escaped = false;
    for (_, ch) in chars {
        if escaped {
            // Standard JSON escapes this repo's own writers can produce (Python's
            // json.dumps, jq): \n \t \r \" \\. \uXXXX never appears in this repo's
            // data (prompt_version is hex, ts is ASCII iso-with-offset, grade/reason
            // are plain text) so it's intentionally unhandled rather than guessed at.
            value.push(match ch {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                other => other, // \" -> ", \\ -> \, anything else passed through as-is
            });
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(value),
            other => value.push(other),
        }
    }
    None
}

/// `current_full_hash` is the artifact's CURRENT full commit hash (see
/// `current_prompt_version`'s doc comment for why full, not `%h`). A logged line
/// survives when its own `prompt_version` (whatever abbreviated length it happened to
/// be logged at) is a non-empty prefix of that full hash — the same relationship git
/// itself uses to resolve any abbreviated hash, so two different-length loggings of
/// the same commit always match and a genuinely different commit's short hash never
/// does (short-hash collision across commits is the same astronomically-unlikely case
/// git's own abbreviation already accepts everywhere else).
fn count_surviving_usage(artifact_dir: &Path, current_full_hash: &str) -> usize {
    let Ok(content) = fs::read_to_string(artifact_dir.join("logs").join("usage.jsonl")) else {
        return 0;
    };
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| {
            extract_json_string_field(line, "prompt_version")
                .is_some_and(|logged| !logged.is_empty() && current_full_hash.starts_with(&logged))
        })
        .count()
}

fn count_surviving_votes(artifact_dir: &Path, current_full_hash: &str) -> usize {
    let Ok(content) = fs::read_to_string(artifact_dir.join("votes").join("votes.jsonl")) else {
        return 0;
    };
    let vote_prefix_marker = "prompt_version: ";
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| {
            let Some(vote) = extract_json_string_field(line, "vote") else {
                return false;
            };
            let Some(rest) = vote.strip_prefix(vote_prefix_marker) else {
                return false;
            };
            let logged = rest.split('\n').next().unwrap_or("");
            !logged.is_empty() && current_full_hash.starts_with(logged)
        })
        .count()
}

/// Minimal escaping for the handful of characters that can appear in an artifact name
/// or reason string here (slashes, no quotes/control chars in practice) — a full JSON
/// writer is unwarranted for output this narrow.
fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn render_json(due: &[DueArtifact]) -> String {
    if due.is_empty() {
        return "[]".to_string();
    }
    let entries: Vec<String> = due
        .iter()
        .map(|d| {
            format!(
                "{{\"artifact\":\"{}\",\"usage_count\":{},\"vote_count\":{},\"reason\":\"{}\"}}",
                json_escape(&d.artifact),
                d.usage_count,
                d.vote_count,
                json_escape(&d.reason)
            )
        })
        .collect();
    format!("[{}]", entries.join(","))
}

fn main() -> ExitCode {
    let root_arg = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let root = fs::canonicalize(&root_arg).unwrap_or_else(|_| PathBuf::from(&root_arg));

    let mut due = Vec::new();
    for (artifact, dir) in discover_artifacts(&root) {
        let Some(prompt_version) = current_prompt_version(&root, &artifact) else {
            // No computable prompt_version (e.g. not a git repo, or artifact untracked
            // yet) — degrade to "not due" rather than guess; never crash the daily run
            // over one artifact's git history.
            continue;
        };
        let usage_count = count_surviving_usage(&dir, &prompt_version);
        let vote_count = count_surviving_votes(&dir, &prompt_version);
        let usage_due = usage_count >= USAGE_THRESHOLD;
        let vote_due = vote_count >= VOTE_THRESHOLD;
        if !usage_due && !vote_due {
            continue;
        }
        let reason = match (usage_due, vote_due) {
            (true, true) => format!(
                "usage_count {usage_count} >= {USAGE_THRESHOLD} and vote_count {vote_count} >= {VOTE_THRESHOLD}"
            ),
            (true, false) => format!("usage_count {usage_count} >= {USAGE_THRESHOLD}"),
            (false, true) => format!("vote_count {vote_count} >= {VOTE_THRESHOLD}"),
            (false, false) => unreachable!(),
        };
        due.push(DueArtifact {
            artifact,
            usage_count,
            vote_count,
            reason,
        });
    }

    println!("{}", render_json(&due));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_string_field_finds_simple_value() {
        let line = r#"{"ts":"x","prompt_version":"abc1234","outcome":"success"}"#;
        assert_eq!(
            extract_json_string_field(line, "prompt_version").as_deref(),
            Some("abc1234")
        );
    }

    #[test]
    fn extract_json_string_field_handles_escaped_quotes() {
        let line = r#"{"vote":"prompt_version: abc1234\nsaid \"hello\""}"#;
        assert_eq!(
            extract_json_string_field(line, "vote").as_deref(),
            Some("prompt_version: abc1234\nsaid \"hello\"")
        );
    }

    #[test]
    fn extract_json_string_field_missing_key_is_none() {
        let line = r#"{"ts":"x"}"#;
        assert_eq!(extract_json_string_field(line, "prompt_version"), None);
    }

    #[test]
    fn count_surviving_usage_filters_by_prompt_version() {
        let dir = std::env::temp_dir().join(format!("gepa-due-test-usage-{}", std::process::id()));
        let logs = dir.join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(
            logs.join("usage.jsonl"),
            "{\"prompt_version\":\"aaa\"}\n{\"prompt_version\":\"bbb\"}\n{\"prompt_version\":\"aaa\"}\n",
        )
        .unwrap();
        assert_eq!(count_surviving_usage(&dir, "aaa"), 2);
        assert_eq!(count_surviving_usage(&dir, "bbb"), 1);
        assert_eq!(count_surviving_usage(&dir, "ccc"), 0);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn count_surviving_usage_matches_across_abbreviation_lengths() {
        // The real bug, live 2026-08-29: the same commit logged under three different
        // `%h` abbreviation lengths as the repo grew (65 lines at 9 chars, 115 at 8
        // chars, 3 at 10 chars — 183 total) all being real current evidence, but only
        // the exact-length match survived the old `==` comparison.
        let dir = std::env::temp_dir().join(format!("gepa-due-test-abbrev-{}", std::process::id()));
        let logs = dir.join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(
            logs.join("usage.jsonl"),
            "{\"prompt_version\":\"76a831fd\"}\n\
             {\"prompt_version\":\"76a831fda\"}\n\
             {\"prompt_version\":\"76a831fda8\"}\n\
             {\"prompt_version\":\"deadbeef\"}\n",
        )
        .unwrap();
        let current_full_hash = "76a831fda89d3d78150038c04b7af6726b5312ba";
        assert_eq!(count_surviving_usage(&dir, current_full_hash), 3);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn count_surviving_usage_empty_logged_prompt_version_never_matches() {
        let dir = std::env::temp_dir().join(format!("gepa-due-test-empty-{}", std::process::id()));
        let logs = dir.join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("usage.jsonl"), "{\"prompt_version\":\"\"}\n").unwrap();
        // an empty logged prompt_version must never match ANY current hash via
        // starts_with (every string starts with "") — this is the guard that stops
        // that vacuous match.
        assert_eq!(count_surviving_usage(&dir, "76a831fda89d3d78"), 0);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn count_surviving_usage_missing_file_is_zero() {
        let dir = std::env::temp_dir().join(format!("gepa-due-test-missing-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(count_surviving_usage(&dir, "aaa"), 0);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn count_surviving_votes_filters_by_vote_prefix() {
        let dir = std::env::temp_dir().join(format!("gepa-due-test-votes-{}", std::process::id()));
        let votes = dir.join("votes");
        fs::create_dir_all(&votes).unwrap();
        fs::write(
            votes.join("votes.jsonl"),
            "{\"vote\":\"prompt_version: aaa\\nsome critique\"}\n\
             {\"vote\":\"prompt_version: bbb\\nother critique\"}\n\
             {\"vote\":\"no prefix at all\"}\n",
        )
        .unwrap();
        assert_eq!(count_surviving_votes(&dir, "aaa"), 1);
        assert_eq!(count_surviving_votes(&dir, "bbb"), 1);
        assert_eq!(count_surviving_votes(&dir, "ccc"), 0);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn count_surviving_votes_matches_across_abbreviation_lengths() {
        let dir = std::env::temp_dir().join(format!("gepa-due-test-votes-abbrev-{}", std::process::id()));
        let votes = dir.join("votes");
        fs::create_dir_all(&votes).unwrap();
        fs::write(
            votes.join("votes.jsonl"),
            "{\"vote\":\"prompt_version: 76a831fd\\nshort form\"}\n\
             {\"vote\":\"prompt_version: 76a831fda8\\nlonger form\"}\n\
             {\"vote\":\"prompt_version: deadbeef\\nunrelated commit\"}\n",
        )
        .unwrap();
        let current_full_hash = "76a831fda89d3d78150038c04b7af6726b5312ba";
        assert_eq!(count_surviving_votes(&dir, current_full_hash), 2);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn render_json_empty_due_list() {
        assert_eq!(render_json(&[]), "[]");
    }

    #[test]
    fn render_json_one_entry() {
        let due = vec![DueArtifact {
            artifact: "skills/foo".to_string(),
            usage_count: 16,
            vote_count: 0,
            reason: "usage_count 16 >= 15".to_string(),
        }];
        let rendered = render_json(&due);
        assert!(rendered.contains("\"artifact\":\"skills/foo\""));
        assert!(rendered.contains("\"usage_count\":16"));
        assert!(rendered.contains("\"vote_count\":0"));
    }

    #[test]
    fn discover_artifacts_requires_evals_dir() {
        let dir = std::env::temp_dir().join(format!("gepa-due-test-discover-{}", std::process::id()));
        fs::create_dir_all(dir.join("skills/has-evals/evals")).unwrap();
        fs::create_dir_all(dir.join("skills/no-evals")).unwrap();
        let found = discover_artifacts(&dir);
        let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"skills/has-evals"));
        assert!(!names.contains(&"skills/no-evals"));
        fs::remove_dir_all(&dir).ok();
    }
}
