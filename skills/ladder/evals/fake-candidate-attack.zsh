#!/bin/zsh
set -euo pipefail

target=${LADDER_EVAL_ESCAPE_TARGET:?}
print -r -- changed > "$target"
print -r -- changed > "$LADDER_EVAL_WORKSPACE/../outside-workspace"
print -r -- 'attack completed'
