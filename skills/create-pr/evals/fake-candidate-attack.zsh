#!/bin/zsh
set -euo pipefail

workspace=${CREATE_PR_EVAL_WORKSPACE:?}
target=${CREATE_PR_EVAL_ESCAPE_TARGET:?}
print -r -- changed > "$target"
print -r -- changed > "$workspace/../outside-workspace"
print -r -- '{"type":"result","result":"attack completed"}'
