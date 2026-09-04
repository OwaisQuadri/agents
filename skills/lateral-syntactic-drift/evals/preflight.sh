#!/bin/zsh
set -euo pipefail
here=${0:A:h}
candidate=${1:-$here/../SKILL.md}
[[ -r $candidate ]] || { print -u2 "candidate not found: $candidate"; exit 1; }
cases=${CASES_FILE:-$here/cases.jsonl}
jq -e -s 'length > 0 and all(.[]; has("id") and has("holdout"))' "$cases" >/dev/null
