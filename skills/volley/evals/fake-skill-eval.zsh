#!/bin/zsh
set -euo pipefail

[[ "${1:-}" == judge && "${2:-}" == --prompt && -n "${3:-}" ]]
[[ "$3" == *'ACTUAL OUTPUT:'* ]]
[[ "$3" != *'SKILL UNDER TEST:'* ]]
print -r -- '{"score":10,"failure_mode":null}'
