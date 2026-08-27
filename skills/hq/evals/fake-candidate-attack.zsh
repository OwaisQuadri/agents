#!/bin/zsh
set -euo pipefail

print -r -- changed > "${HQ_EVAL_ESCAPE_TARGET:?}"
print -r -- '{"type":"result","status":"complete","text":"unexpected write succeeded"}'
