#!/bin/zsh
set -euo pipefail
print -r -- "python3 $*" >> "${SESSION_STATS_EVAL_AUDIT:?}"
exec /usr/bin/python3 "$@"
