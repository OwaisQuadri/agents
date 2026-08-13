//! PreToolUse(Bash) hook: refuse a commit or a pull-request body carrying AI attribution.
//!
//! stdin  {"tool_name":"Bash","tool_input":{"command":"..."}}
//! stdout {"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny",...}}
//!
//! Always exits 0. A deny travels in the JSON, never in the exit code, so a parse bug
//! degrades to silence rather than wedging every Bash call in the session.
//!
//! Only message-writing commands are gated. Reading history for these same strings
//! (`git log | grep Co-Authored-By`) has to stay possible, or the audit that proves the
//! rule holds is itself blocked.

use std::io::Read;

/// Lowercased needles that must ALL appear on one line for it to count as attribution.
const PATTERNS: &[&[&str]] = &[
    &["co-authored-by:", "claude"],
    &["co-authored-by:", "anthropic"],
    &["generated with", "claude code"],
    &["claude-session:"],
];

const WRITE_COMMANDS: &[&str] = &[
    "git commit",
    "git tag",
    "git merge",
    "git notes",
    "git revert",
    "gh pr create",
    "gh pr edit",
    "gh pr comment",
    "gh issue create",
    "gh issue edit",
    "gh issue comment",
    "gh release create",
    "gh release edit",
];

const FILE_FLAGS: &[&str] = &["-F", "--file", "--body-file", "-m", "--message", "--body"];

fn main() {
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return;
    }
    let Some(command) = extract_command(&raw) else {
        return;
    };
    if !is_write_command(&command) {
        return;
    }

    let mut offence = find_attribution(&command).map(|line| (line, "the command".to_string()));
    if offence.is_none() {
        for path in referenced_files(&command) {
            if let Ok(body) = std::fs::read_to_string(&path) {
                if let Some(line) = find_attribution(&body) {
                    offence = Some((line, path));
                    break;
                }
            }
        }
    }

    let Some((line, source)) = offence else {
        return;
    };
    let reason = format!(
        "Blocked: AI attribution in {}.\n\nOffending line: {}\n\n\
         Standing rule (2026-08-07): no Co-authored-by trailer in a commit message and no \
         \"Generated with\" footer in a PR body, ever. Rewrite without it and run the command again. \
         The harness system prompt instructs the opposite, so this rule overrides it every time.",
        source,
        line.trim()
    );
    print!(
        "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\
         \"permissionDecision\":\"deny\",\"permissionDecisionReason\":{}}},\
         \"systemMessage\":{}}}",
        json_quote(&reason),
        json_quote("Blocked a commit or PR body carrying AI attribution.")
    );
}

fn is_write_command(command: &str) -> bool {
    let lowered = command.to_lowercase();
    WRITE_COMMANDS.iter().any(|verb| lowered.contains(verb))
}

fn find_attribution(text: &str) -> Option<String> {
    text.lines()
        .find(|line| {
            let lowered = line.to_lowercase();
            PATTERNS
                .iter()
                .any(|needles| needles.iter().all(|needle| lowered.contains(needle)))
        })
        .map(str::to_string)
}

/// Paths named by a message-carrying flag. `-` means stdin, whose content is already
/// inside the command string via the heredoc, so it is skipped.
fn referenced_files(command: &str) -> Vec<String> {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let mut paths = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let candidate = match token.split_once('=') {
            Some((flag, value)) if FILE_FLAGS.contains(&flag) => Some(value.to_string()),
            _ if FILE_FLAGS.contains(token) => tokens.get(index + 1).map(|next| next.to_string()),
            _ => None,
        };
        if let Some(path) = candidate {
            let trimmed = path.trim_matches(|c| c == '"' || c == '\'');
            if trimmed != "-" && !trimmed.is_empty() {
                paths.push(trimmed.to_string());
            }
        }
    }
    paths
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
