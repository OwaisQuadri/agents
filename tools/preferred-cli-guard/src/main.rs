//! PreToolUse(bash) guard for `pi/extensions/preferred-cli-guard.ts`: blocks a bash call
//! that literally invokes `find` or `grep` and explains the `fd`/`rg` idiom instead of
//! rewriting the command (fd/rg skip hidden and `.gitignore`d paths by default; a silent
//! rewrite could quietly change results).
//!
//! `preferred-cli-guard --check <command>` — exit 0 allow (no stdout), exit 1 block
//! (reason on stdout). Add a swap (e.g. `sed`->`sd`) as one more `RULES` entry.

use std::process::ExitCode;

/// One preferred-CLI swap; add an entry here for a new swap.
struct Rule {
    banned: &'static str,
    preferred: &'static str,
    /// Block when `banned` follows a single `|`.
    catch_after_pipe: bool,
    /// Block when `banned` is the program `xargs` launches.
    catch_via_xargs: bool,
    /// (word sequence to match, idiomatic suggestion); first match anywhere wins.
    idioms: &'static [(&'static str, &'static str)],
    fallback_note: &'static str,
    gotcha: Option<&'static str>,
}

const RULES: &[Rule] = &[
    Rule {
        banned: "find",
        preferred: "fd",
        catch_after_pipe: false,
        catch_via_xargs: false,
        idioms: &[
            ("-name", "filtering by extension: `fd -e <ext>`. filtering by name substring: bare `fd <pattern>` (fd matches path substrings/regex by default, no leading '*' needed)"),
            ("-type f", "`fd -t f`"),
            ("-type d", "`fd -t d`"),
        ],
        fallback_note: "run `fd --help` for the flag that expresses this search — fd's flags are not a 1:1 mapping from find's",
        gotcha: Some(
            "fd skips `.gitignore`d and hidden paths by default (find never did) — pass \
             `-H`/`--hidden` and `-I`/`--no-ignore` to reach them.",
        ),
    },
    Rule {
        banned: "grep",
        preferred: "rg",
        catch_after_pipe: true,
        catch_via_xargs: true,
        idioms: &[
            ("-r", "drop it — rg recurses by default, no flag needed"),
            ("-R", "drop it — rg recurses by default, no flag needed"),
            ("-i", "`rg -i` (same flag)"),
        ],
        fallback_note: "run `rg --help` for the flag that expresses this search — most grep flags carry over, some (like -r) are rg's default behavior instead",
        gotcha: Some(
            "rg skips `.gitignore`d and hidden paths by default (grep never did) — pass \
             `--hidden` and `--no-ignore` to reach them.",
        ),
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sep {
    And,
    Or,
    Semi,
    Pipe,
    LParen,
    RParen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Word(String),
    Sep(Sep),
}

/// Tokenizes into words and the shell operators this check cares about. Quote-aware
/// (a quoted region is one word); `#` starts a comment to end of line.
#[allow(unused_assignments)] // final flush!() write is never read again
fn tokenize(command: &str) -> Vec<Piece> {
    let chars: Vec<char> = command.chars().collect();
    let n = chars.len();
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut has_content = false;
    let mut i = 0;

    macro_rules! flush {
        () => {
            if has_content {
                pieces.push(Piece::Word(std::mem::take(&mut current)));
                has_content = false;
            }
        };
    }

    while i < n {
        match chars[i] {
            ' ' | '\t' | '\n' | '\r' => {
                flush!();
                i += 1;
            }
            '#' if !has_content => {
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\'' => {
                has_content = true;
                i += 1;
                while i < n && chars[i] != '\'' {
                    current.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                has_content = true;
                i += 1;
                while i < n && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < n {
                        current.push(chars[i + 1]);
                        i += 2;
                    } else {
                        current.push(chars[i]);
                        i += 1;
                    }
                }
                i += 1;
            }
            '&' if i + 1 < n && chars[i + 1] == '&' => {
                flush!();
                pieces.push(Piece::Sep(Sep::And));
                i += 2;
            }
            '|' if i + 1 < n && chars[i + 1] == '|' => {
                flush!();
                pieces.push(Piece::Sep(Sep::Or));
                i += 2;
            }
            '|' => {
                flush!();
                pieces.push(Piece::Sep(Sep::Pipe));
                i += 1;
            }
            ';' => {
                flush!();
                pieces.push(Piece::Sep(Sep::Semi));
                i += 1;
            }
            '(' => {
                flush!();
                pieces.push(Piece::Sep(Sep::LParen));
                i += 1;
            }
            ')' => {
                flush!();
                pieces.push(Piece::Sep(Sep::RParen));
                i += 1;
            }
            '`' => {
                // Opaque like a quote, not a separator: a Sep on both backticks wrongly
                // reset segment-start after the closer, false-positiving `echo `date`
                // find`. Trade-off: find/grep inside a substitution goes uncaught.
                flush!();
                i += 1;
                while i < n && chars[i] != '`' {
                    i += 1;
                }
                i += 1;
            }
            '$' if i + 1 < n && chars[i + 1] == '(' => {
                flush!();
                pieces.push(Piece::Sep(Sep::LParen));
                i += 2;
            }
            c => {
                has_content = true;
                current.push(c);
                i += 1;
            }
        }
    }
    flush!();
    pieces
}

/// Strips one leading backslash (bash's alias-bypass idiom) before lowercasing, so
/// `\find`/`\grep` still match.
fn command_word(word: &str) -> String {
    word.strip_prefix('\\').unwrap_or(word).to_lowercase()
}

/// `identifier=...` shaped — the shell's env-assignment-prefix syntax.
fn looks_like_assignment(word: &str) -> bool {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    for c in chars {
        if c == '=' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchKind {
    Leading,
    Pipe,
    /// True when a `find` genuinely precedes the `| xargs` launch.
    Xargs(bool),
}

/// Finds the rule violation, if any. `git grep` never matches: `grep` after `git` is
/// never leading, piped, or xargs-launched. The xargs pass runs first so `find ... |
/// xargs grep` reports the xargs reason, not the coincidentally-earlier `find`.
fn find_violation(pieces: &[Piece]) -> Option<(usize, &'static Rule, MatchKind)> {
    for (idx, piece) in pieces.iter().enumerate() {
        let Piece::Word(word) = piece else { continue };
        let lowered = command_word(word);
        if let Some(rule) = RULES
            .iter()
            .find(|r| r.catch_via_xargs && r.banned == lowered)
        {
            if launched_by_xargs(pieces, idx) {
                let via_find = preceded_by_find_pipe(pieces, idx);
                return Some((idx, rule, MatchKind::Xargs(via_find)));
            }
        }
    }

    // `|` never resets segment-start; a piped word only matches via catch_after_pipe.
    let mut at_segment_start = true;
    for (idx, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Sep(Sep::And)
            | Piece::Sep(Sep::Or)
            | Piece::Sep(Sep::Semi)
            | Piece::Sep(Sep::LParen) => {
                at_segment_start = true;
            }
            Piece::Sep(Sep::Pipe) | Piece::Sep(Sep::RParen) => {}
            Piece::Word(word) => {
                let lowered = command_word(word);

                if at_segment_start {
                    if looks_like_assignment(word) {
                        continue;
                    }
                    if let Some(rule) = RULES.iter().find(|r| r.banned == lowered) {
                        return Some((idx, rule, MatchKind::Leading));
                    }
                    at_segment_start = false;
                } else if idx > 0 && pieces[idx - 1] == Piece::Sep(Sep::Pipe) {
                    if let Some(rule) = RULES
                        .iter()
                        .find(|r| r.catch_after_pipe && r.banned == lowered)
                    {
                        return Some((idx, rule, MatchKind::Pipe));
                    }
                }
            }
        }
    }
    None
}

/// True when `idx` is the program `xargs` launches, past its own leading flags.
fn launched_by_xargs(pieces: &[Piece], idx: usize) -> bool {
    let mut j = idx;
    loop {
        if j == 0 {
            return false;
        }
        j -= 1;
        match &pieces[j] {
            Piece::Word(w) if w.starts_with('-') => continue,
            Piece::Word(w) if w.eq_ignore_ascii_case("xargs") => return true,
            _ => return false,
        }
    }
}

/// True when the pipe segment immediately before this xargs launch starts with `find`
/// — the `find ... | xargs grep` shape. Takes the launched command's index, not xargs's.
fn preceded_by_find_pipe(pieces: &[Piece], banned_idx: usize) -> bool {
    let mut j = banned_idx;
    let xargs_idx = loop {
        if j == 0 {
            return false;
        }
        j -= 1;
        match &pieces[j] {
            Piece::Word(w) if w.starts_with('-') => continue,
            Piece::Word(w) if w.eq_ignore_ascii_case("xargs") => break j,
            _ => return false,
        }
    };
    if xargs_idx == 0 || pieces[xargs_idx - 1] != Piece::Sep(Sep::Pipe) {
        return false;
    }
    let mut leftmost_word = None;
    let mut j = xargs_idx as isize - 2;
    while j >= 0 {
        match &pieces[j as usize] {
            Piece::Word(w) => leftmost_word = Some(w.as_str()),
            Piece::Sep(Sep::RParen) => {}
            Piece::Sep(_) => break,
        }
        j -= 1;
    }
    leftmost_word
        .map(|w| command_word(w) == "find")
        .unwrap_or(false)
}

/// True when `trigger`'s words appear as a contiguous run anywhere in `words`.
fn contains_idiom_trigger(words: &[&str], trigger: &str) -> bool {
    let needle: Vec<&str> = trigger.split_whitespace().collect();
    if needle.is_empty() || needle.len() > words.len() {
        return false;
    }
    words.windows(needle.len()).any(|w| w == needle.as_slice())
}

fn build_reason(rule: &Rule, kind: MatchKind, pieces: &[Piece]) -> String {
    let words: Vec<&str> = pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Word(w) => Some(w.as_str()),
            Piece::Sep(_) => None,
        })
        .collect();

    let header = match kind {
        MatchKind::Leading => format!(
            "Blocked `{}` — use `{}` instead.",
            rule.banned, rule.preferred
        ),
        MatchKind::Pipe => format!(
            "Blocked `| {}` — pipe into `{}` instead (it reads stdin the same way).",
            rule.banned, rule.preferred
        ),
        MatchKind::Xargs(true) => format!(
            "Blocked `find | xargs {}` — this whole pipeline collapses to one `{} \
             <pattern> <dir>` call; xargs/find are unnecessary once {} does its own \
             recursive search.",
            rule.banned, rule.preferred, rule.preferred
        ),
        MatchKind::Xargs(false) => format!(
            "Blocked `xargs {}` — use `{}` instead; it reads the file list itself, no \
             xargs launcher needed.",
            rule.banned, rule.preferred
        ),
    };

    let idiom = rule
        .idioms
        .iter()
        .find(|(trigger, _)| contains_idiom_trigger(&words, trigger))
        .map(|(_, suggestion)| format!("Idiom: {suggestion}."))
        .unwrap_or_else(|| rule.fallback_note.to_string());

    match rule.gotcha {
        Some(gotcha) => format!("{header} {idiom} {gotcha}"),
        None => format!("{header} {idiom}"),
    }
}

/// The block reason, or `None` to allow. Never panics; malformed input degrades to
/// allow, matching `pi/extensions/logpath-guard.ts`'s posture.
fn blocked_command(command: &str) -> Option<String> {
    let pieces = tokenize(command);
    let (_, rule, kind) = find_violation(&pieces)?;
    Some(build_reason(rule, kind, &pieces))
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--check") {
        eprintln!("usage: preferred-cli-guard --check <command>");
        return ExitCode::FAILURE;
    }
    let Some(command) = args.next() else {
        eprintln!("usage: preferred-cli-guard --check <command>");
        return ExitCode::FAILURE;
    };

    match blocked_command(&command) {
        Some(reason) => {
            println!("{reason}");
            ExitCode::FAILURE
        }
        None => ExitCode::SUCCESS,
    }
}

#[cfg(test)]
mod tests {
    use super::blocked_command;

    #[test]
    fn leading_find_blocks() {
        assert!(blocked_command("find . -name '*.rs'").is_some());
        assert!(blocked_command("find /").is_some());
    }

    #[test]
    fn leading_grep_blocks() {
        assert!(blocked_command("grep -r pattern src/").is_some());
    }

    #[test]
    fn grep_after_pipe_blocks() {
        assert!(blocked_command("ps aux | grep foo").is_some());
    }

    #[test]
    fn grep_after_chain_operator_blocks() {
        assert!(blocked_command("cmd1 && grep foo bar").is_some());
    }

    #[test]
    fn grep_after_env_assignment_blocks() {
        assert!(blocked_command("FOO=bar grep -i pattern").is_some());
    }

    #[test]
    fn find_piped_into_xargs_grep_blocks() {
        let reason = blocked_command("find . -name '*.txt' | xargs grep pattern").unwrap();
        assert!(reason.contains("xargs"), "{reason}");
        assert!(reason.contains("rg"), "{reason}");
    }

    #[test]
    fn xargs_grep_with_flags_blocks() {
        assert!(blocked_command("xargs -I{} grep {} file").is_some());
    }

    #[test]
    fn xargs_grep_reason_names_the_pipeline_only_when_find_actually_precedes_it() {
        let with_find = blocked_command("find . -name '*.txt' | xargs grep pattern").unwrap();
        assert!(with_find.contains("find | xargs"), "{with_find}");

        let without_find = blocked_command("xargs -I{} grep {} file").unwrap();
        assert!(!without_find.contains("find | xargs"), "{without_find}");
        assert!(without_find.contains("xargs"), "{without_find}");
        assert!(without_find.contains("rg"), "{without_find}");
    }

    #[test]
    fn backslash_prefixed_find_and_grep_still_block() {
        assert!(blocked_command("\\find . -name '*.rs'").is_some());
        assert!(blocked_command("\\grep pattern file").is_some());
    }

    #[test]
    fn word_after_a_closed_backtick_substitution_is_not_a_new_command() {
        assert!(blocked_command("echo `date` find").is_none());
        assert!(blocked_command("echo `date` grep").is_none());
    }

    #[test]
    fn git_grep_is_allowed() {
        assert!(blocked_command("git grep pattern").is_none());
    }

    #[test]
    fn fd_and_rg_themselves_are_allowed() {
        assert!(blocked_command("fd . -e rs").is_none());
        assert!(blocked_command("rg pattern").is_none());
    }

    #[test]
    fn quoted_grep_is_not_a_command_invocation() {
        assert!(blocked_command("echo \"grep foo\"").is_none());
    }

    #[test]
    fn commented_out_grep_is_not_a_command_invocation() {
        assert!(blocked_command("cargo build --release # grep for errors").is_none());
    }

    #[test]
    fn grep_as_a_path_substring_is_not_a_command_invocation() {
        assert!(blocked_command("cat src/grep_utils.rs").is_none());
    }

    #[test]
    fn plain_reading_commands_are_allowed() {
        assert!(blocked_command("gh pr list").is_none());
        assert!(blocked_command("cat file.txt").is_none());
    }

    #[test]
    fn find_reason_names_fd_and_idiom() {
        let reason = blocked_command("find . -name '*.rs'").unwrap();
        assert!(reason.contains("fd"), "{reason}");
        assert!(reason.contains("Idiom"), "{reason}");
        assert!(
            reason.contains("gitignore") || reason.contains(".gitignore"),
            "{reason}"
        );
    }

    #[test]
    fn find_type_f_idiom_is_specific() {
        let reason = blocked_command("find . -type f").unwrap();
        assert!(reason.contains("-t f"), "{reason}");
    }

    #[test]
    fn grep_reason_names_rg_and_gotcha() {
        let reason = blocked_command("grep -r pattern src/").unwrap();
        assert!(reason.contains("rg"), "{reason}");
        assert!(
            reason.contains("recurses by default") || reason.contains("Idiom"),
            "{reason}"
        );
        assert!(reason.contains("hidden"), "{reason}");
    }

    #[test]
    fn find_does_not_trigger_pipe_or_xargs_catch() {
        assert!(blocked_command("something | find").is_none());
    }
}
