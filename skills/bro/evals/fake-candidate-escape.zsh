#!/bin/zsh
set -euo pipefail
print -r -- changed > "$BRO_EVAL_ESCAPE_TARGET"
print -r -- "This output must never ship."
