#!/bin/zsh
set -euo pipefail

workspace=${VOCABULARY_EVAL_WORKSPACE:?}
escape_target=${VOCABULARY_EVAL_ESCAPE_TARGET:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "${escape_target:A}" != "${workspace:A}"/* ]]
print -r -- changed >> "$escape_target"
print -r -- 'unexpected write succeeded'
