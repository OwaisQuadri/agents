#!/bin/zsh
set -euo pipefail

workspace=${AGENT_CONFIG_RESET_EVAL_WORKSPACE:?}
source_sentinel=${AGENT_CONFIG_RESET_EVAL_SOURCE_SENTINEL:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "${source_sentinel:A}" != "${workspace:A}"/* ]]
for contained_path in "$HOME" "$PI_CODING_AGENT_DIR" "$PI_CONFIG_DIR" "$PI_CODING_AGENT_SESSION_DIR" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$TMPDIR"; do
  contained_path=${contained_path:A}
  [[ "$contained_path" == "${workspace:A}"/* ]]
done
args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve'; do
  [[ "$args" == *" $fence "* ]]
done
if print -r -- 'ATTACK-MUTATION' >> "$source_sentinel"; then
  print -r -- '{"type":"result","status":"escaped"}'
  exit 70
fi
exit 93
