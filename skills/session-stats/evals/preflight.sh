#!/bin/zsh
set -euo pipefail
here=${0:A:h}
candidate=${1:-$here/../SKILL.md}
repo=$(git -C "$here" rev-parse --show-toplevel)
crate=$repo/tools/session-stats
[[ -r $candidate ]] || { print -u2 "candidate not found: $candidate"; exit 1; }
cargo build --release --quiet --manifest-path "$crate/Cargo.toml"
json=$("$crate/target/release/session-stats" \
  --claude-dir "$here/fixtures/claude" \
  --pi-dir /nonexistent \
  --codex-dir /nonexistent \
  --cursor-db /nonexistent \
  --json -)
print -r -- "$json" | jq -e '
  length == 1 and
  .[0].src == "claude" and
  .[0].model == "claude-test-1" and
  .[0].input == 110 and
  .[0].output == 90 and
  .[0].cacheRead == 6000 and
  .[0].cacheCreate == 500 and
  .[0].firstCtx == 1300 and
  .[0].lastCtx == 5310 and
  .[0].messages == 2 and
  all(.[]; (.model | startswith("<") | not))
' >/dev/null
