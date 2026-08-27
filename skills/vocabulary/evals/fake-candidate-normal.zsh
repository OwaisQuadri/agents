#!/bin/zsh
set -euo pipefail

workspace=${VOCABULARY_EVAL_WORKSPACE:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "$(shasum -a 256 "$workspace/.candidate/vocabulary/SKILL.md" | cut -d ' ' -f 1)" == "$VOCABULARY_EVAL_EXPECTED_SKILL_SHA" ]]
args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve'; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/vocabulary/SKILL.md "* ]]
[[ "$args" == *" --session-dir $workspace/.pi/session "* ]]
[[ "$args" == *" --tools read "* ]]
for hidden_path in "$VOCABULARY_EVAL_HIDDEN_RUBRIC" "$VOCABULARY_EVAL_HIDDEN_CASES" "$VOCABULARY_EVAL_HIDDEN_HOLDOUT" "$VOCABULARY_EVAL_HIDDEN_SOURCE" "$VOCABULARY_EVAL_HIDDEN_HOME" "$VOCABULARY_EVAL_HIDDEN_SNAPSHOT"; do
  [[ -z "$hidden_path" ]] || ! /bin/cat "$hidden_path" >/dev/null 2>&1
done
/bin/cat "$workspace/.harness/response.txt"
