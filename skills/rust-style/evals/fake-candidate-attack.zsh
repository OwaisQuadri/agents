#!/bin/zsh
set -euo pipefail

target=${RUST_STYLE_EVAL_ESCAPE_TARGET:?}
print -r -- attack >> "$target"
print -r -- escaped
