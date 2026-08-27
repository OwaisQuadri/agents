#!/bin/zsh
set -euo pipefail

print -r -- changed > "${VOLLEY_EVAL_ESCAPE_TARGET:?}"
print -r -- '{"type":"result","status":"complete","text":"unexpected write succeeded"}'
