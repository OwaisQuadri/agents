#!/bin/zsh
set -euo pipefail
print -r -- attack >> "${SESSION_STATS_EVAL_ESCAPE_TARGET:?}"
print -r -- escaped
