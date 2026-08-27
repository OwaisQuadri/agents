#!/bin/zsh
set -euo pipefail

workspace=${VOLLEY_EVAL_WORKSPACE:?}
id=${VOLLEY_EVAL_CASE_ID:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$VOLLEY_EVAL_EXPECTED_SKILL_SHA" ]]
args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve'; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]]
for hidden in "$VOLLEY_EVAL_HIDDEN_RUBRIC" "$VOLLEY_EVAL_HIDDEN_CASES" "$VOLLEY_EVAL_HIDDEN_HOLDOUT" "$VOLLEY_EVAL_HIDDEN_SOURCE" "$VOLLEY_EVAL_HIDDEN_HOME"; do
  ! /bin/cat "$hidden" >/dev/null 2>&1
done

case "$id" in
  c1)
    handle=$(./fixture-action.zsh dispatch-command suite)
    response="Dispatched the full test suite.\nin flight: $handle — run the full test suite"
    ;;
  c2)
    branch=$(./fixture-action.zsh foreground branch)
    response="Current branch: $branch\nin flight: none"
    ;;
  c3)
    result=$(./fixture-action.zsh acknowledge job-test-27)
    response="$result\nin flight: none"
    ;;
  c4)
    result=$(./fixture-action.zsh foreground targeted-fix)
    response="$result. I did not fix the 3 unrelated lint errors.\nin flight: none"
    ;;
  c5)
    handle=$(./fixture-action.zsh dispatch-agent migration)
    response="Dispatched the call-site migration.\nin flight: $handle — migrate every call site"
    ;;
  c6)
    handle=$(./fixture-action.zsh dispatch-command unknown-read)
    response="The estimate is unclear, so I dispatched the file read.\nin flight: $handle — read the file and answer"
    ;;
  c7)
    answer=$(./fixture-action.zsh foreground arithmetic)
    response="$answer\nin flight: job-build-11 — build release artifacts\nin flight: job-tests-12 — run integration tests"
    ;;
  *) exit 64 ;;
esac

print -r -- "{\"type\":\"result\",\"status\":\"complete\",\"text\":$(jq -Rn --arg text "$response" '$text')}"
