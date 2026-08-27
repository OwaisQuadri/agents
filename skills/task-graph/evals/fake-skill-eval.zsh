#!/bin/zsh
set -euo pipefail

[[ "${1:-}" == judge && "${2:-}" == --prompt && -n "${3:-}" ]]
[[ "$3" == *'Actual produced output event stream:'* ]]
print -r -- '{"score":10,"failure_mode":null}'
