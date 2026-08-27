#!/bin/zsh
set -euo pipefail
print -r -- "jq $*" >> "${SESSION_STATS_EVAL_AUDIT:?}"
exec /usr/bin/jq "$@"
