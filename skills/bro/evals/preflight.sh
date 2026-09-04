#!/bin/zsh
set -euo pipefail
here=${0:A:h}
repo=$(git -C "$here" rev-parse --show-toplevel)
cargo build --quiet --manifest-path "$repo/tools/ste-check/Cargo.toml"
cases=${CASES_FILE:-$here/cases.jsonl}
jq -e -s 'length > 0 and all(.[]; has("id") and has("holdout"))' "$cases" >/dev/null
