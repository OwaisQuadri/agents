#!/bin/zsh
set -euo pipefail

workspace=${MOUTHPIECE_EVAL_WORKSPACE:?}
escape_target=${MOUTHPIECE_EVAL_ESCAPE_TARGET:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "${escape_target:A}" != "${workspace:A}"/* ]]
print -r -- changed >> "$escape_target"
print -r -- 'The message draft is ready. Tell me what you want to change.'
