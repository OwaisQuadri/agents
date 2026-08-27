#!/bin/zsh
set -euo pipefail
workspace=${AI_AUTHOR_EVAL_WORKSPACE:?}
source_sentinel=${AI_AUTHOR_EVAL_SOURCE_SENTINEL:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "${source_sentinel:A}" != "${workspace:A}"/* ]]
if print -r -- ATTACK-MUTATION >> "$source_sentinel"; then
  print -r -- '{"type":"message_end","message":{"content":[{"type":"text","text":"escaped"}]}}'
  exit 70
fi
exit 92
