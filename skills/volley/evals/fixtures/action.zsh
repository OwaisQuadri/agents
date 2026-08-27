#!/bin/zsh
set -euo pipefail

mode=${1:?usage: fixture-action <foreground|dispatch-command|dispatch-agent|acknowledge> [step-or-handle]}
value=${2:-}
scenario=${VOLLEY_EVAL_SCENARIO:-scenario.json}
actions=${VOLLEY_EVAL_ACTIONS:-.harness/actions.jsonl}
[[ -f "$scenario" ]]
case_id=$(jq -er '.case' "$scenario")
step=$(jq -er '.step' "$scenario")
mkdir -p "${actions:h}"

case "$mode" in
  foreground)
    [[ -z "$value" || "$value" == "$step" ]]
    jq -cn --arg mode "$mode" --arg step "$step" --arg case_id "$case_id" '{mode:$mode,step:$step,case:$case_id}' >> "$actions"
    jq -er '.foreground_result' "$scenario"
    ;;
  dispatch-command|dispatch-agent)
    [[ -z "$value" || "$value" == "$step" ]]
    handle="job-$case_id-$step"
    jq -cn --arg mode "$mode" --arg step "$step" --arg handle "$handle" --arg case_id "$case_id" '{mode:$mode,step:$step,handle:$handle,case:$case_id}' >> "$actions"
    print -r -- "$handle"
    ;;
  acknowledge)
    expected=$(jq -er '.notification.handle' "$scenario")
    [[ "$value" == "$expected" ]]
    jq -cn --arg mode "$mode" --arg handle "$value" --arg case_id "$case_id" '{mode:$mode,handle:$handle,case:$case_id}' >> "$actions"
    jq -er '.notification.result' "$scenario"
    ;;
  *)
    print -u2 -r -- "unknown fixture action: $mode"
    exit 2
    ;;
esac
