#!/bin/zsh
set -euo pipefail
cd "$(dirname "$0")"
mode="${1:-}"
if [[ "$mode" != "" && "$mode" != "--holdout" ]]; then
  print -u2 "Usage: $0 [--holdout]"
  exit 2
fi
jq -c --argjson holdout "$([[ "$mode" == "--holdout" ]] && print true || print false)" 'select(.holdout == $holdout) | {id, score: 10, failure_mode: null}' cases.jsonl
