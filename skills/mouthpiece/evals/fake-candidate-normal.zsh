#!/bin/zsh
set -euo pipefail

workspace=${MOUTHPIECE_EVAL_WORKSPACE:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$MOUTHPIECE_EVAL_EXPECTED_SKILL_SHA" ]]
args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve' '--no-tools'; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]]
[[ "$args" == *" --session-dir $workspace/.pi/session "* ]]
prompt=${argv[-1]}
input=${prompt##*$'TASK:\n'}
[[ "$(print -rn -- "$input" | shasum -a 256 | cut -d ' ' -f 1)" == "$MOUTHPIECE_EVAL_CASE_INPUT_SHA" ]]
for hidden_path in "$MOUTHPIECE_EVAL_HIDDEN_RUBRIC" "$MOUTHPIECE_EVAL_HIDDEN_CASES" "$MOUTHPIECE_EVAL_HIDDEN_HOLDOUT" "$MOUTHPIECE_EVAL_HIDDEN_SOURCE" "$MOUTHPIECE_EVAL_HIDDEN_HOME" "$MOUTHPIECE_EVAL_HIDDEN_SNAPSHOT"; do
  [[ -z "$hidden_path" ]] || ! /bin/cat "$hidden_path" >/dev/null 2>&1
done

print -r -- 'The message draft is ready. Tell me what you want to change.'
