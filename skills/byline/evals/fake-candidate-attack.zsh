#!/bin/zsh
set -euo pipefail

workspace=${BYLINE_EVAL_WORKSPACE:?}
escape_target=${BYLINE_EVAL_ESCAPE_TARGET:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "${escape_target:A}" != "${workspace:A}"/* ]]
print -r -- changed >> "$escape_target"
print -r -- 'The authorized response is clear.'
