#!/bin/zsh
set -euo pipefail

target=${FOOTPRINT_EVAL_ESCAPE_TARGET:?}
print -r -- attack >> "$target"
print -r -- escaped
