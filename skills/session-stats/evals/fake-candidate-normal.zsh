#!/bin/zsh
set -euo pipefail

workspace=${SESSION_STATS_EVAL_WORKSPACE:?}
id=${SESSION_STATS_EVAL_CASE_ID:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
args=" $* "
for fence in --no-session --no-skills --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]]
[[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$SESSION_STATS_EVAL_EXPECTED_SKILL_SHA" ]]
for hidden in "$SESSION_STATS_EVAL_HIDDEN_RUBRIC" "$SESSION_STATS_EVAL_HIDDEN_CASES" "$SESSION_STATS_EVAL_HIDDEN_HOLDOUT" "$SESSION_STATS_EVAL_HIDDEN_SOURCE" "$SESSION_STATS_EVAL_HIDDEN_HOME"; do
  [[ -z "$hidden" ]] || ! /bin/cat "$hidden" >/dev/null 2>&1
done

session-stats --json /tmp/session-stats.json >/dev/null
case "$id" in
  fixture-shape)
    row=$(jq -r '.[0] | "src=\(.src), model=\(.model), input=\(.input), output=\(.output), cacheRead=\(.cacheRead), cacheCreate=\(.cacheCreate), messages=\(.messages), firstCtx=\(.firstCtx), lastCtx=\(.lastCtx)"' "$workspace/compiled.json")
    print -r -- "$row. I ran jq over $workspace/compiled.json."
    ;;
  dedup)
    row=$(jq -r '.[0] | "messages=\(.messages), input=\(.input), output=\(.output)"' "$workspace/compiled.json")
    print -r -- "The repeated m1 request adds no tokens. $row. I ran jq over $workspace/compiled.json."
    ;;
  synthetic-model)
    models=$(jq -r 'map(.model) | join(", ")' "$workspace/compiled.json")
    print -r -- "The compiled models are $models. The <synthetic> record produces no row. I ran jq over $workspace/compiled.json."
    ;;
  analysis-not-transcripts)
    result=$(jq -r 'map(select(.src == "claude" and .model == "claude-test-1" and .first >= "2026-01-01" and .last < "2026-01-02")) | max_by(.output) | "\(.model) used \(.output) output tokens"' "$workspace/compiled.json")
    print -r -- "$result. Filters: source=claude, model=claude-test-1, date=2026-01-01. Compiled JSON: $workspace/compiled.json."
    ;;
  view-on-request)
    session-stats --out /tmp/session-stats.html --open >/dev/null
    print -r -- "Compiled JSON: $workspace/compiled.json. Graph: $workspace/session-stats.html. I used only the disposable synthetic history."
    ;;
  *) exit 64 ;;
esac
