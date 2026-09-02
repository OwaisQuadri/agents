//! Flags over-verbose code comments per docs/comment-style.md's closed whitelist.
//!
//! Whitelist-shape classification ("is this an inexpressible architectural invariant")
//! is judgment, not mechanics, so this first pass enforces the mechanical proxy the
//! owner's review actually objected to: raw verbosity. A non-doc comment block longer
//! than the budget is flagged; doc comments (`///`, `//!`, `/**`) are exempt because
//! the docstring shape is whitelisted and docs/docstring-style.md owns their form.
//!
//! Two modes, matching the two sibling shapes:
//!   comment-check <file>...   ste-check shape: print FAIL lines, exit nonzero
//!   comment-check             warnings-check shape: PreToolUse(Bash) hook payload on
//!                             stdin, deny travels in JSON, always exits 0

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const MAX_COMMENT_LINES: usize = 4;

const SOURCE_EXTENSIONS: &[&str] = &["rs", "ts", "tsx"];

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

const USAGE: &str = "usage: comment-check [file]...  (no arguments: PreToolUse hook payload on stdin)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.is_empty() {
        run_hook();
        return ExitCode::SUCCESS;
    }
    let mut is_failing = false;
    for path in &args {
        for violation in check_path(Path::new(path)) {
            is_failing = true;
            println!("FAIL  {violation}");
        }
    }
    if is_failing {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run_hook() {
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

    let mut violations = Vec::new();
    for path in staged {
        if !is_source_path(&path) {
            continue;
        }
        violations.extend(check_path(&repo_root.join(&path)));
    }
    if violations.is_empty() {
        return;
    }

    let mut reason = String::from(
        "Blocked: a staged source file carries an over-verbose comment block.\n\n\
         Standing rule: docs/comment-style.md — a comment ships only in a whitelisted \
         shape, fewer comments beat more, zero is the default target. A non-doc comment \
         block over the length budget almost never fits a whitelisted shape; delete it \
         and make the code explain itself, or shrink it to the invariant it protects.\n",
    );
    for violation in &violations {
        reason.push_str(&format!("\n{violation}"));
    }

    print!(
        "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\
         \"permissionDecision\":\"deny\",\"permissionDecisionReason\":{}}},\
         \"systemMessage\":{}}}",
        json_quote(&reason),
        json_quote("Blocked a commit: a staged file carries an over-verbose comment block.")
    );
}

fn is_source_path(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| SOURCE_EXTENSIONS.contains(&ext))
}

fn check_path(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let is_rust = path.extension().is_some_and(|ext| ext == "rs");
    long_blocks(&text, is_rust)
        .into_iter()
        .map(|(start, end)| {
            format!(
                "{}:{start}-{end}  non-doc comment block of {} lines (budget {MAX_COMMENT_LINES})",
                path.display(),
                end - start + 1
            )
        })
        .collect()
}

#[derive(PartialEq)]
enum CommentKind {
    Doc,
    Plain,
}

struct CommentSpan {
    start_line: usize,
    end_line: usize,
    kind: CommentKind,
    is_full_line: bool,
}

/// 1-based (start, end) line spans of every non-doc comment block longer than the
/// budget. Consecutive full-line `//` comments merge into one block; a trailing
/// comment after code never merges with the lines below it.
fn long_blocks(text: &str, is_rust: bool) -> Vec<(usize, usize)> {
    let spans = comment_spans(text, is_rust);
    let mut blocks: Vec<CommentSpan> = Vec::new();
    for span in spans {
        if let Some(last) = blocks.last_mut() {
            if last.kind == span.kind
                && last.is_full_line
                && span.is_full_line
                && span.start_line == last.end_line + 1
            {
                last.end_line = span.end_line;
                continue;
            }
        }
        blocks.push(span);
    }
    blocks
        .into_iter()
        .filter(|b| b.kind == CommentKind::Plain)
        .filter(|b| b.end_line - b.start_line + 1 > MAX_COMMENT_LINES)
        .map(|b| (b.start_line, b.end_line))
        .collect()
}

/// Lexes the source just enough to find comments without being fooled by comment
/// markers inside string literals: plain strings in both languages, raw strings and
/// char/lifetime quotes in Rust, template literals in TypeScript.
fn comment_spans(text: &str, is_rust: bool) -> Vec<CommentSpan> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let mut is_line_blank_so_far = true;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '\n' => {
                line += 1;
                is_line_blank_so_far = true;
                i += 1;
            }
            '/' if next == Some('/') => {
                let is_doc = if is_rust {
                    let third = chars.get(i + 2).copied();
                    let fourth = chars.get(i + 3).copied();
                    third == Some('!') || (third == Some('/') && fourth != Some('/'))
                } else {
                    false
                };
                let is_full_line = is_line_blank_so_far;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                spans.push(CommentSpan {
                    start_line: line,
                    end_line: line,
                    kind: if is_doc { CommentKind::Doc } else { CommentKind::Plain },
                    is_full_line,
                });
            }
            '/' if next == Some('*') => {
                let third = chars.get(i + 2).copied();
                let fourth = chars.get(i + 3).copied();
                let is_doc = third == Some('!')
                    || (third == Some('*') && fourth != Some('*') && fourth != Some('/'));
                let is_full_line = is_line_blank_so_far;
                let start_line = line;
                i += 2;
                let mut depth = 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '\n' {
                        line += 1;
                    } else if is_rust && chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 1;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 1;
                    }
                    i += 1;
                }
                spans.push(CommentSpan {
                    start_line,
                    end_line: line,
                    kind: if is_doc { CommentKind::Doc } else { CommentKind::Plain },
                    is_full_line,
                });
                is_line_blank_so_far = false;
            }
            '"' => {
                is_line_blank_so_far = false;
                i += 1;
                while i < chars.len() {
                    match chars[i] {
                        '"' => {
                            i += 1;
                            break;
                        }
                        '\\' => i += 2,
                        '\n' => {
                            line += 1;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            }
            'r' if is_rust && matches!(next, Some('"') | Some('#')) => {
                let mut hashes = 0;
                let mut j = i + 1;
                while chars.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if chars.get(j) != Some(&'"') {
                    is_line_blank_so_far = false;
                    i += 1;
                    continue;
                }
                is_line_blank_so_far = false;
                i = j + 1;
                while i < chars.len() {
                    if chars[i] == '\n' {
                        line += 1;
                    } else if chars[i] == '"'
                        && (1..=hashes).all(|k| chars.get(i + k) == Some(&'#'))
                    {
                        i += hashes + 1;
                        break;
                    }
                    i += 1;
                }
            }
            '\'' if is_rust => {
                is_line_blank_so_far = false;
                if next == Some('\\') {
                    i += 2;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if chars.get(i + 2) == Some(&'\'') {
                    i += 3;
                } else {
                    // a lifetime, not a char literal: consume only the quote
                    i += 1;
                }
            }
            '\'' | '`' if !is_rust => {
                let quote = c;
                is_line_blank_so_far = false;
                i += 1;
                while i < chars.len() {
                    match chars[i] {
                        q if q == quote => {
                            i += 1;
                            break;
                        }
                        '\\' => i += 2,
                        '\n' => {
                            line += 1;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            }
            _ => {
                if !c.is_whitespace() {
                    is_line_blank_so_far = false;
                }
                i += 1;
            }
        }
    }
    spans
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
/// `tools/warnings-check/src/main.rs`, gating on the same one verb sequence.
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

    fn rust_blocks(text: &str) -> Vec<(usize, usize)> {
        long_blocks(text, true)
    }

    #[test]
    fn short_comment_blocks_pass() {
        let text = "// one\n// two\n// three\n// four\nfn main() {}\n";
        assert!(rust_blocks(text).is_empty());
    }

    #[test]
    fn a_five_line_run_of_line_comments_is_flagged() {
        let text = "// one\n// two\n// three\n// four\n// five\nfn main() {}\n";
        assert_eq!(rust_blocks(text), vec![(1, 5)]);
    }

    #[test]
    fn doc_comments_are_exempt() {
        let text = "//! a\n//! b\n//! c\n//! d\n//! e\n//! f\n/// g\n/// h\n/// i\n/// j\n/// k\nfn main() {}\n";
        assert!(rust_blocks(text).is_empty());
        let jsdoc = "/**\n * a\n * b\n * c\n * d\n * e\n */\nexport function f() {}\n";
        assert!(long_blocks(jsdoc, false).is_empty());
    }

    #[test]
    fn a_long_block_comment_is_flagged() {
        let text = "fn main() {}\n/* a\nb\nc\nd\ne */\n";
        assert_eq!(rust_blocks(text), vec![(2, 6)]);
        assert_eq!(long_blocks(text, false), vec![(2, 6)]);
    }

    #[test]
    fn a_blank_line_splits_two_short_runs() {
        let text = "// one\n// two\n// three\n\n// four\n// five\nfn main() {}\n";
        assert!(rust_blocks(text).is_empty());
    }

    #[test]
    fn a_trailing_comment_does_not_merge_with_the_run_below() {
        let text = "let x = 1; // trailing\n// a\n// b\n// c\n// d\nfn main() {}\n";
        assert!(rust_blocks(text).is_empty());
    }

    #[test]
    fn comment_markers_inside_strings_are_not_comments() {
        let text = "let a = \"// one\";\nlet b = \"// two\";\nlet c = \"// three\";\nlet d = \"// four\";\nlet e = \"// five\";\n";
        assert!(rust_blocks(text).is_empty());
        let raw = "let a = r#\"// one\n// two\n// three\n// four\n// five\"#;\n";
        assert!(rust_blocks(raw).is_empty());
        let template = "const a = `// one\n// two\n// three\n// four\n// five`;\n";
        assert!(long_blocks(template, false).is_empty());
    }

    #[test]
    fn a_lifetime_quote_does_not_swallow_the_rest_of_the_file() {
        let text = "fn f<'a>(x: &'a str) {}\n// a\n// b\n// c\n// d\n// e\n";
        assert_eq!(rust_blocks(text), vec![(2, 6)]);
    }

    #[test]
    fn typescript_line_comments_have_no_doc_form() {
        let text = "/// a\n/// b\n/// c\n/// d\n/// e\nexport {}\n";
        assert_eq!(long_blocks(text, false), vec![(1, 5)]);
    }

    #[test]
    fn only_source_extensions_are_scanned() {
        assert!(is_source_path("tools/foo/src/main.rs"));
        assert!(is_source_path("pi/extensions/guard.ts"));
        assert!(is_source_path("web/app.tsx"));
        assert!(!is_source_path("docs/comment-style.md"));
        assert!(!is_source_path("install.sh"));
    }

    #[test]
    fn commit_gating_matches_the_sibling_checkers() {
        assert!(is_git_commit("git commit -m x"));
        assert!(is_git_commit("git -C ~/repo commit -m x"));
        assert!(is_git_commit("git add -A && git commit -m x"));
        assert!(!is_git_commit("git log --oneline"));
        assert!(!is_git_commit("gh pr create --body x"));
    }

    #[test]
    fn extracts_command_from_pretooluse_payload() {
        let payload = r#"{"tool_name":"Bash","tool_input":{"command":"git commit -m x"}}"#;
        assert_eq!(extract_command(payload).as_deref(), Some("git commit -m x"));
    }
}
