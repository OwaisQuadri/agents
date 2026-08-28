//! PreToolUse(bash) guard, called from `pi/extensions/preferred-cli-guard.ts`: blocks a
//! bash tool call that literally invokes `find` or `grep` and teaches the agent the
//! idiomatic `fd`/`rg` replacement instead of letting it run.
//!
//! Not a silent rewrite — fd/rg default to skipping `.gitignore`d and hidden paths,
//! which find/grep never did, and fd's pattern is regex-by-default vs find's glob. A
//! transparent rewrite risks quietly changing what the search returns, so this blocks
//! and explains instead.
//!
//! Built as a small `RULES` table (see below), not a find/grep-only special case: the
//! owner named this as the first of an expected series of preferred-CLI swaps, so the
//! next one (`sed`->`sd`, `cat`->`bat`, ...) is one more table entry, not a rewrite of
//! the tokenizer.
//!
//! Contract (matches `tools/logpath-check`'s `--validate-path` mode): `preferred-cli-guard
//! --check <command>` — exit 0 = allow, no stdout. Exit 1 = block, the reason on stdout.
//! `<command>` is passed as a single argv element by the caller (no shell involved), so
//! it needs no escaping in either direction.

use std::process::ExitCode;

/// One preferred-CLI swap. Adding a new swap is one more `Rule` entry — nothing else in
/// this file needs to change.
struct Rule {
    /// The banned program name, exact lowercase token match.
    banned: &'static str,
    /// The tool this steers the agent toward.
    preferred: &'static str,
    /// Also block when `banned` is the word immediately after a single `|`.
    catch_after_pipe: bool,
    /// Also block when `banned` is the word `xargs` launches (after `xargs`'s own
    /// leading `-flags`).
    catch_via_xargs: bool,
    /// `(exact contiguous word sequence to look for, idiomatic suggestion)`. The first
    /// one found anywhere in the command's tokens wins; unmatched falls back to
    /// `fallback_note`. Deliberately NOT a mechanical flag-for-flag translation table —
    /// each entry names the idiomatic way to express the same intent, which is
    /// sometimes a different flag shape entirely.
    idioms: &'static [(&'static str, &'static str)],
    /// Shown when no idiom trigger matches.
    fallback_note: &'static str,
    /// The one behavioral gotcha worth stating every time this rule fires, or `None`
    /// if the swap has no equivalent gotcha.
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

/// Tokenizes a bash command string into words and the shell operators that matter for
/// this check (`&&`, `||`, `;`, `|`, `(`, `)`, backtick/`$(` as subshell opens).
/// Quote-aware: a quoted region is grouped into ONE word (quote characters stripped),
/// never split on internal whitespace, so `echo "grep foo"` produces the word `grep
/// foo`, not the bare word `grep` a naive split-then-trim would produce. `#` starting a
/// fresh word (not inside a quote, not mid-word) begins a comment that runs to the next
/// newline, matching shell comment syntax.
#[allow(unused_assignments)] // the final `flush!()` writes `has_content = false` that
                             // nothing reads afterward — the macro's other call sites do read it, so the assignment
                             // stays shared rather than duplicating flush! logic for a harmless last-write.
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
                // A backtick has no distinct open/close token — both occurrences are
                // the same character, unlike `$(`...`)`. Emitting a Sep on EITHER one
                // (as an earlier version did) folds them into the same Sep::LParen
                // variant, which wrongly resets segment-start after the CLOSING
                // backtick too — `echo \`date\` find` then reads `find` as if it starts
                // a new command, a false positive. Treated as opaque instead: skip to
                // the matching backtick and emit nothing, the same posture as a quote
                // (content not tokenized) rather than a separator. Trade-off: a
                // find/grep genuinely invoked INSIDE a backtick substitution is no
                // longer caught — accepted per the repo's own rule that a checker
                // deterministic about the wrong thing (blocking a false positive) is
                // worse than one that misses a rare edge case (`logpath-guard`'s own
                // comment states this same trade-off).
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

/// Strips a single leading backslash, so `\find`/`\grep` compare equal to `find`/
/// `grep`. Bash treats a backslash before an ordinary character as "take it
/// literally" — `\find` runs the real `find` binary, bypassing any shell alias or
/// function named `find`. That is exactly the well-known idiom for dodging a shell
/// guard, so without this normalization the checker itself would be trivially
/// evadable by the one prefix everyone already reaches for to bypass an alias.
fn command_word(word: &str) -> String {
    word.strip_prefix('\\').unwrap_or(word).to_lowercase()
}

/// `identifier=...` shaped, with no assumption about what follows `=` — the shell's own
/// env-assignment-prefix syntax (`FOO=bar grep ...`).
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
    /// `bool` is true when a `find` word genuinely precedes the `| xargs` launch in
    /// the immediately-prior pipe segment — used to pick the reason wording so a
    /// bare `xargs grep ...` (no `find` anywhere) never claims a `find | xargs`
    /// pipeline that was never there.
    Xargs(bool),
}

/// Finds the rule violation in `pieces`. `git grep` is excluded with no special-case
/// code: `grep` sitting right after `git` is never the leading word of its command
/// segment, never immediately after a `|`, and never launched by `xargs` — none of the
/// three trigger positions apply, so it simply never matches.
///
/// Two passes, not one left-to-right scan: an `xargs`-launched match is the most
/// specific and actionable shape (it names the whole pipeline as collapsible, see
/// `MatchKind::Xargs`'s reason template), so it is checked FIRST across the whole
/// command — otherwise `find . -name '*.txt' | xargs grep pattern` would report the
/// coincidentally-earlier `find` (also a genuine leading-command match) and never
/// surface the more useful "drop the whole pipeline" reason for the `xargs grep` part.
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

    // `|` deliberately does NOT reset segment-start below: a piped command is only a
    // trigger position when its rule opts in via `catch_after_pipe` (checked
    // explicitly), never via the generic "leading word of a command" rule — otherwise
    // `something | find` would wrongly match Leading even though find's
    // `catch_after_pipe` is false.
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

/// True when, walking backward from `idx`, every word is a `-flag` until an `xargs`
/// word is hit, with no separator in between — i.e. `idx` is the program `xargs`
/// launches, past its own leading flags (`xargs -I{} grep ...`, `xargs grep ...`).
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

/// True when the `xargs` word that launches `banned_idx` (walking back past `xargs`'s
/// own `-flag`s, same walk as `launched_by_xargs`) is itself immediately preceded by a
/// `|`, and the pipe segment before THAT starts with a genuine `find` word — the shape
/// `find ... | xargs grep ...`. Scoped to the one immediately-prior segment (stops at
/// any separator, including an earlier `|`) so a `find` sitting further back in an
/// unrelated part of a longer chain is never credited to this xargs launch.
///
/// Takes `banned_idx` (the launched command's own index, e.g. `grep`'s), NOT `xargs`'s
/// index — `launched_by_xargs` already confirmed a match starting from `banned_idx`;
/// re-deriving `xargs`'s index here from the same starting point, rather than plumbing
/// it through as a second return value, keeps `launched_by_xargs`'s signature a plain
/// bool the way `find_violation`'s other two branches use their match functions.
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

/// True when `trigger` (a space-separated word sequence, e.g. `"-type f"`) appears as a
/// contiguous, exact-word-match run anywhere in `words`.
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

/// Pure decision function: the reason to block, or `None` to allow. Never panics on
/// malformed input — an unparseable command falls through to `None` (allow), the same
/// degrade-to-allow posture `pi/extensions/logpath-guard.ts` and `hooks/rag-recall`
/// already use for a guard's own failure.
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

        // Regression: this shape has no `find` anywhere, so the reason must not claim
        // one — a fresh-context code review caught the earlier version always saying
        // "find | xargs grep" even when the command never ran find.
        let without_find = blocked_command("xargs -I{} grep {} file").unwrap();
        assert!(!without_find.contains("find | xargs"), "{without_find}");
        assert!(without_find.contains("xargs"), "{without_find}");
        assert!(without_find.contains("rg"), "{without_find}");
    }

    #[test]
    fn backslash_prefixed_find_and_grep_still_block() {
        // Regression: `\find`/`\grep` is the standard bash idiom for bypassing a shell
        // alias or function of the same name — without normalizing it, the guard itself
        // was trivially evadable by the one prefix everyone already reaches for.
        assert!(blocked_command("\\find . -name '*.rs'").is_some());
        assert!(blocked_command("\\grep pattern file").is_some());
    }

    #[test]
    fn word_after_a_closed_backtick_substitution_is_not_a_new_command() {
        // Regression: a backtick has no distinct open/close token, so an earlier
        // version folded both onto the same separator kind and wrongly treated the
        // word right after the CLOSING backtick as the start of a new command segment
        // — `echo `date` find` was blocked even though it never runs `find` at all.
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
        // find has catch_after_pipe=false, catch_via_xargs=false — only leading-command
        // find should ever block; a `find` sitting after a pipe/xargs is not a shape
        // this checker claims to handle (piped `find` output isn't itself a search).
        assert!(blocked_command("something | find").is_none());
    }
}
