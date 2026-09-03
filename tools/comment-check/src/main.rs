//! Flags over-verbose code comments per docs/comment-style.md's closed whitelist.
//!
//! Whitelist-shape classification ("is this an inexpressible architectural invariant")
//! is judgment, not mechanics, so this first pass enforces the mechanical proxy the
//! owner's review actually objected to: raw verbosity. A non-doc comment block longer
//! than the budget is flagged; doc comments (`///`, `//!`, `/**`) are exempt because
//! the docstring shape is whitelisted and docs/docstring-style.md owns their form.
//!
//! Three modes:
//!   comment-check <file>...             ste-check shape: print FAIL lines, exit nonzero
//!   comment-check                       warnings-check shape: PreToolUse(Bash) hook
//!                                       payload on stdin, deny travels in JSON, exits 0
//!   comment-check --list-json --lang X  reads stdin as source text, prints every
//!                                       comment span (doc and non-doc) as a JSON
//!                                       array with its own text. Whitelist-shape and
//!                                       docstring-position judgment happen downstream
//!                                       of this: this mode is extraction only, same
//!                                       zero-judgment posture as the other two.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const MAX_COMMENT_LINES: usize = 3;

const SLASH_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "go", "swift", "c", "h", "cc", "cpp",
    "hpp", "java", "kt", "kts", "cs", "zig", "scala", "m", "mm",
];

const HASH_EXTENSIONS: &[&str] =
    &["py", "sh", "zsh", "bash", "rb", "pl", "toml", "yaml", "yml"];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Lang {
    Slash { is_rust: bool },
    Hash { is_python: bool },
}

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

const USAGE: &str = "usage: comment-check [file]...  |  comment-check --list-json --lang EXT  (no arguments: PreToolUse hook payload on stdin)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--list-json") {
        return run_list_json(&args);
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

/// `--list-json --lang EXT`: reads source text from stdin, prints a JSON array of every
/// comment span (doc and non-doc) with its own text. Extraction only — no verbosity or
/// shape judgment — mirroring the other two modes' own zero-judgment posture.
fn run_list_json(args: &[String]) -> ExitCode {
    let Some(lang_index) = args.iter().position(|a| a == "--lang") else {
        eprintln!("--list-json requires --lang EXT\n{USAGE}");
        return ExitCode::FAILURE;
    };
    let Some(ext) = args.get(lang_index + 1) else {
        eprintln!("--lang requires a value\n{USAGE}");
        return ExitCode::FAILURE;
    };
    let Some(lang) = lang_for_extension(&format!("x.{ext}")) else {
        eprintln!("unrecognized --lang extension: {ext}");
        return ExitCode::FAILURE;
    };
    let mut text = String::new();
    if std::io::stdin().read_to_string(&mut text).is_err() {
        eprintln!("failed to read stdin");
        return ExitCode::FAILURE;
    }
    println!("{}", spans_to_json(&text, lang));
    ExitCode::SUCCESS
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
    lang_for_extension(path).is_some()
}

fn lang_for_extension(path: &str) -> Option<Lang> {
    let (_, ext) = path.rsplit_once('.')?;
    if SLASH_EXTENSIONS.contains(&ext) {
        return Some(Lang::Slash { is_rust: ext == "rs" });
    }
    if HASH_EXTENSIONS.contains(&ext) {
        return Some(Lang::Hash { is_python: ext == "py" });
    }
    None
}

/// An extensionless file is classified by its shebang, so hook scripts and other
/// bare executables stay inside the budget too.
fn lang_for_shebang(text: &str) -> Option<Lang> {
    let first = text.lines().next()?;
    if !first.starts_with("#!") {
        return None;
    }
    if first.contains("python") {
        return Some(Lang::Hash { is_python: true });
    }
    if ["sh", "zsh", "bash", "ruby", "perl"].iter().any(|name| first.contains(name)) {
        return Some(Lang::Hash { is_python: false });
    }
    None
}

fn check_path(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let lang = match path.to_str().and_then(lang_for_extension).or_else(|| lang_for_shebang(&text)) {
        Some(lang) => lang,
        None => return Vec::new(),
    };
    long_blocks(&text, lang)
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
/// budget. Consecutive full-line comments merge into one block; a trailing
/// comment after code never merges with the lines below it.
fn long_blocks(text: &str, lang: Lang) -> Vec<(usize, usize)> {
    merged_spans(text, lang)
        .into_iter()
        .filter(|b| b.kind == CommentKind::Plain)
        .filter(|b| b.end_line - b.start_line + 1 > MAX_COMMENT_LINES)
        .map(|b| (b.start_line, b.end_line))
        .collect()
}

/// Every comment block (doc and non-doc), consecutive full-line comments merged into
/// one span the same way `long_blocks` merges them — shared so `--list-json` and the
/// length check see identical block boundaries.
fn merged_spans(text: &str, lang: Lang) -> Vec<CommentSpan> {
    let spans = match lang {
        Lang::Slash { is_rust } => comment_spans(text, is_rust),
        Lang::Hash { is_python } => hash_comment_spans(text, is_python),
    };
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
}

/// Renders every comment block in `text` (doc and non-doc) as a JSON array of
/// `{start_line, end_line, kind, text}`, `text` being the exact source lines (with
/// their own leading comment markers) the span covers.
fn spans_to_json(text: &str, lang: Lang) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::from("[");
    for (i, span) in merged_spans(text, lang).into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let kind = match span.kind {
            CommentKind::Doc => "doc",
            CommentKind::Plain => "plain",
        };
        let span_text = lines
            .get(span.start_line - 1..span.end_line)
            .map(|slice| slice.join("\n"))
            .unwrap_or_default();
        out.push_str(&format!(
            r#"{{"start_line":{},"end_line":{},"kind":"{kind}","text":{}}}"#,
            span.start_line,
            span.end_line,
            json_quote(&span_text)
        ));
    }
    out.push(']');
    out
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
                let third = chars.get(i + 2).copied();
                let fourth = chars.get(i + 3).copied();
                let is_doc = if is_rust {
                    third == Some('!') || (third == Some('/') && fourth != Some('/'))
                } else {
                    third == Some('!') || third == Some('/')
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

/// Lexes hash-comment languages: `#` opens a comment only at line start or after
/// whitespace, so `$#`, `${#x}`, and mid-word hashes stay code. Python's
/// triple-quoted strings and both quote styles are skipped; a shebang is exempt.
fn hash_comment_spans(text: &str, is_python: bool) -> Vec<CommentSpan> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let mut is_line_blank_so_far = true;
    let mut prev_is_boundary = true;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                line += 1;
                is_line_blank_so_far = true;
                prev_is_boundary = true;
                i += 1;
            }
            '#' if prev_is_boundary => {
                let is_shebang = line == 1 && chars.get(i + 1) == Some(&'!');
                let is_full_line = is_line_blank_so_far;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                spans.push(CommentSpan {
                    start_line: line,
                    end_line: line,
                    kind: if is_shebang { CommentKind::Doc } else { CommentKind::Plain },
                    is_full_line,
                });
            }
            '"' | '\'' => {
                let quote = c;
                is_line_blank_so_far = false;
                prev_is_boundary = false;
                let is_triple = is_python
                    && chars.get(i + 1) == Some(&quote)
                    && chars.get(i + 2) == Some(&quote);
                if is_triple {
                    i += 3;
                    while i < chars.len() {
                        if chars[i] == '\n' {
                            line += 1;
                        } else if chars[i] == '\\' {
                            i += 1;
                        } else if chars[i] == quote
                            && chars.get(i + 1) == Some(&quote)
                            && chars.get(i + 2) == Some(&quote)
                        {
                            i += 3;
                            break;
                        }
                        i += 1;
                    }
                } else {
                    i += 1;
                    while i < chars.len() {
                        match chars[i] {
                            q if q == quote => {
                                i += 1;
                                break;
                            }
                            '\\' if quote == '"' || is_python => i += 2,
                            '\n' => {
                                line += 1;
                                i += 1;
                            }
                            _ => i += 1,
                        }
                    }
                }
            }
            _ => {
                if !c.is_whitespace() {
                    is_line_blank_so_far = false;
                }
                prev_is_boundary = c.is_whitespace();
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

    const RUST: Lang = Lang::Slash { is_rust: true };
    const TS: Lang = Lang::Slash { is_rust: false };
    const PYTHON: Lang = Lang::Hash { is_python: true };
    const SHELL: Lang = Lang::Hash { is_python: false };

    fn rust_blocks(text: &str) -> Vec<(usize, usize)> {
        long_blocks(text, RUST)
    }

    #[test]
    fn short_comment_blocks_pass() {
        let text = "// one\n// two\n// three\nfn main() {}\n";
        assert!(rust_blocks(text).is_empty());
    }

    #[test]
    fn a_four_line_run_of_line_comments_is_flagged() {
        let text = "// one\n// two\n// three\n// four\nfn main() {}\n";
        assert_eq!(rust_blocks(text), vec![(1, 4)]);
    }

    #[test]
    fn doc_comments_are_exempt() {
        let text = "//! a\n//! b\n//! c\n//! d\n//! e\n//! f\n/// g\n/// h\n/// i\n/// j\n/// k\nfn main() {}\n";
        assert!(rust_blocks(text).is_empty());
        let jsdoc = "/**\n * a\n * b\n * c\n * d\n * e\n */\nexport function f() {}\n";
        assert!(long_blocks(jsdoc, TS).is_empty());
    }

    #[test]
    fn a_long_block_comment_is_flagged() {
        let text = "fn main() {}\n/* a\nb\nc\nd\ne */\n";
        assert_eq!(rust_blocks(text), vec![(2, 6)]);
        assert_eq!(long_blocks(text, TS), vec![(2, 6)]);
    }

    #[test]
    fn a_blank_line_splits_two_short_runs() {
        let text = "// one\n// two\n// three\n\n// four\n// five\nfn main() {}\n";
        assert!(rust_blocks(text).is_empty());
    }

    #[test]
    fn a_trailing_comment_does_not_merge_with_the_run_below() {
        let text = "let x = 1; // trailing\n// a\n// b\n// c\nfn main() {}\n";
        assert!(rust_blocks(text).is_empty());
    }

    #[test]
    fn comment_markers_inside_strings_are_not_comments() {
        let text = "let a = \"// one\";\nlet b = \"// two\";\nlet c = \"// three\";\nlet d = \"// four\";\nlet e = \"// five\";\n";
        assert!(rust_blocks(text).is_empty());
        let raw = "let a = r#\"// one\n// two\n// three\n// four\n// five\"#;\n";
        assert!(rust_blocks(raw).is_empty());
        let template = "const a = `// one\n// two\n// three\n// four\n// five`;\n";
        assert!(long_blocks(template, TS).is_empty());
    }

    #[test]
    fn a_lifetime_quote_does_not_swallow_the_rest_of_the_file() {
        let text = "fn f<'a>(x: &'a str) {}\n// a\n// b\n// c\n// d\n// e\n";
        assert_eq!(rust_blocks(text), vec![(2, 6)]);
    }

    #[test]
    fn triple_slash_is_doc_in_every_slash_language() {
        let text = "/// a\n/// b\n/// c\n/// d\n/// e\nexport {}\n";
        assert!(long_blocks(text, TS).is_empty());
    }

    #[test]
    fn a_four_line_hash_run_is_flagged() {
        let text = "# one\n# two\n# three\n# four\nx = 1\n";
        assert_eq!(long_blocks(text, PYTHON), vec![(1, 4)]);
        assert!(long_blocks("# one\n# two\n# three\nx = 1\n", SHELL).is_empty());
    }

    #[test]
    fn hash_inside_strings_and_words_is_not_a_comment() {
        let text = "a = \"# one\"\nb = \"# two\"\nc = \"# three\"\nd = \"# four\"\n";
        assert!(long_blocks(text, PYTHON).is_empty());
        let doc = "def f():\n    \"\"\"\n    # a\n    # b\n    # c\n    # d\n    \"\"\"\n";
        assert!(long_blocks(doc, PYTHON).is_empty());
        let shell = "echo $#\nn=${#arr}\ncase x#y in esac\nz=1\n";
        assert!(long_blocks(shell, SHELL).is_empty());
    }

    #[test]
    fn a_shebang_does_not_merge_with_the_run_below() {
        let text = "#!/bin/zsh\n# a\n# b\n# c\nx=1\n";
        assert!(long_blocks(text, SHELL).is_empty());
        let text = "#!/bin/zsh\n# a\n# b\n# c\n# d\nx=1\n";
        assert_eq!(long_blocks(text, SHELL), vec![(2, 5)]);
    }

    #[test]
    fn source_extensions_span_both_comment_families() {
        assert!(is_source_path("tools/foo/src/main.rs"));
        assert!(is_source_path("pi/extensions/guard.ts"));
        assert!(is_source_path("web/app.tsx"));
        assert!(is_source_path("install.sh"));
        assert!(is_source_path("scripts/run.py"));
        assert!(is_source_path("config/settings.toml"));
        assert!(!is_source_path("docs/comment-style.md"));
        assert!(!is_source_path("README"));
    }

    #[test]
    fn extensionless_files_are_classified_by_shebang() {
        assert_eq!(lang_for_shebang("#!/bin/zsh\n# a\n"), Some(SHELL));
        assert_eq!(lang_for_shebang("#!/usr/bin/env python3\n"), Some(PYTHON));
        assert_eq!(lang_for_shebang("plain text\n"), None);
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

    #[test]
    fn list_json_includes_doc_and_plain_spans_with_their_own_text() {
        let text = "/// docstring\nfn public_fn() {}\n\n// plain comment\n// second line\nfn private_helper() {}\n";
        let json = spans_to_json(text, RUST);
        assert_eq!(
            json,
            r#"[{"start_line":1,"end_line":1,"kind":"doc","text":"/// docstring"},{"start_line":4,"end_line":5,"kind":"plain","text":"// plain comment\n// second line"}]"#
        );
    }

    #[test]
    fn list_json_never_filters_by_length_unlike_long_blocks() {
        // a run over MAX_COMMENT_LINES: long_blocks flags it, --list-json still lists
        // it verbatim -- length filtering is long_blocks's job alone, list-json is pure
        // extraction for a downstream judge to classify.
        let text = "// one\n// two\n// three\n// four\nfn main() {}\n";
        assert_eq!(rust_blocks(text), vec![(1, 4)]);
        let json = spans_to_json(text, RUST);
        assert!(json.contains(r#""start_line":1,"end_line":4,"kind":"plain"#));
    }

    #[test]
    fn list_json_escapes_quotes_and_newlines_in_span_text() {
        let text = "// says \"hi\"\nfn main() {}\n";
        let json = spans_to_json(text, RUST);
        assert_eq!(
            json,
            r#"[{"start_line":1,"end_line":1,"kind":"plain","text":"// says \"hi\""}]"#
        );
    }

    #[test]
    fn list_json_on_empty_input_is_an_empty_array() {
        assert_eq!(spans_to_json("", RUST), "[]");
        assert_eq!(spans_to_json("fn main() {}\n", RUST), "[]");
    }

    #[test]
    fn list_json_works_across_hash_and_slash_languages() {
        let py = "# a note\ndef f():\n    pass\n";
        assert_eq!(
            spans_to_json(py, PYTHON),
            r##"[{"start_line":1,"end_line":1,"kind":"plain","text":"# a note"}]"##
        );
    }
}
