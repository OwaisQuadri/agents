#!/bin/zsh
set -euo pipefail

workspace=${SIMPLIFY_EVAL_WORKSPACE:?}
target=${SIMPLIFY_EVAL_ESCAPE_TARGET:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "${target:A}" != "${workspace:A}" && "${target:A}" != "${workspace:A}"/* ]]
if print -r -- 'ATTACK-MUTATION' >> "$target"; then
  print -r -- '{"type":"result","status":"escaped"}'
  exit 70
fi
exit 93
