#!/bin/zsh
set -euo pipefail

workspace=${RUST_STYLE_EVAL_WORKSPACE:?}
id=${RUST_STYLE_EVAL_CASE_ID:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
args=" $* "
for fence in --no-session --no-skills --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]]
[[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d " " -f 1)" == "$RUST_STYLE_EVAL_EXPECTED_SKILL_SHA" ]]
for hidden in "$RUST_STYLE_EVAL_HIDDEN_RUBRIC" "$RUST_STYLE_EVAL_HIDDEN_CASES" "$RUST_STYLE_EVAL_HIDDEN_HOLDOUT" "$RUST_STYLE_EVAL_HIDDEN_SOURCE" "$RUST_STYLE_EVAL_HIDDEN_HOME" "$RUST_STYLE_EVAL_HIDDEN_SNAPSHOT"; do
  [[ -z "$hidden" ]] || ! /bin/cat "$hidden" >/dev/null 2>&1
done

case "$id" in
  c1)
    print -rn -- 'use std::num::ParseIntError;

pub fn try_parse_port(input: &str) -> Result<u16, ParseIntError> {
    input.parse()
}

pub fn parse_port(input: &str) -> u16 {
    try_parse_port(input).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_port() {
        assert_eq!(parse_port("8080"), 8080);
        assert_eq!(try_parse_port("443"), Ok(443));
    }

    #[test]
    fn rejects_invalid_port() {
        assert!(try_parse_port("invalid").is_err());
    }
}
' > src/lib.rs
    print -r -- 'Implemented the requested fallible parser and preserved the existing interface.

Rust baseline: applied
Checks: cargo fmt --check: pass; cargo check: pass; cargo test: pass; cargo clippy -- -D warnings: pass
Exceptions: none
' > REPORT.md
    ;;
  c2)
    print -r -- 'Finding: the unsafe block has no necessary reason, safety requirements, or explanation of how the implementation satisfies them. The caller must guarantee that values is nonempty, but the interface does not state or enforce that requirement.

Finding: #[allow(clippy::missing_safety_doc)] suppresses an intentional lint without an auditable reason. Replace it with #[expect(clippy::missing_safety_doc, reason = "...")] only if the exception remains necessary.

Rust baseline: applied
Checks: cargo fmt --check: pass; cargo check: pass; cargo clippy -- -D warnings: pass
Exceptions: unsafe code and lint suppression are review findings, not accepted exceptions
' > REVIEW.md
    ;;
  c3)
    print -r -- 'The rust-style skill does not apply in Claude Code because the active path rule already supplies the Rust baseline. I would not duplicate it or claim that checks ran.'
    ;;
  c4)
    print -r -- 'The rust-style skill does not apply to a source-free ownership explanation. No repository checks ran.'
    ;;
  c5)
    print -r -- 'The rust-style skill does not apply because only TypeScript source is in scope. No Rust checks ran.'
    ;;
  c6)
    print -r -- 'I rejected the unsafe rewrite. A profile and a requirement that no safe implementation can meet would be needed before unsafe pointer arithmetic is justified. The safe source remains unchanged.

Rust baseline: applied
Checks: cargo fmt --check: pass; cargo check: pass; cargo test: pass; cargo clippy -- -D warnings: pass
Exceptions: none
' > REPORT.md
    ;;
  *)
    exit 64
    ;;
esac
