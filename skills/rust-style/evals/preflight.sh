#!/bin/zsh
set -euo pipefail
here=${0:A:h}
candidate=${1:-$here/../SKILL.md}
[[ -r $candidate ]] || { print -u2 "candidate not found: $candidate"; exit 1; }
[[ -r $here/../rust-baseline.md ]] || { print -u2 'rust baseline not found'; exit 1; }
jq -e 'has("id") and has("holdout")' "$here/cases.jsonl" >/dev/null
