#!/bin/zsh
set -euo pipefail

workspace=${BYLINE_EVAL_WORKSPACE:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$BYLINE_EVAL_EXPECTED_SKILL_SHA" ]]
args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve' '--no-tools'; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]]
[[ "$args" == *" --session-dir $workspace/.pi/session "* ]]
for hidden_path in "$BYLINE_EVAL_HIDDEN_RUBRIC" "$BYLINE_EVAL_HIDDEN_CASES" "$BYLINE_EVAL_HIDDEN_SOURCE" "$BYLINE_EVAL_HIDDEN_HOME" "$BYLINE_EVAL_HIDDEN_SNAPSHOT"; do
  [[ -z "$hidden_path" ]] || ! /bin/cat "$hidden_path" >/dev/null 2>&1
done

case "$BYLINE_EVAL_CASE_ID" in
  b1) print -r -- 'The caching layer drops resolver latency from 340ms to 21ms on bench/resolve.ts.' ;;
  b2) print -r -- 'Refactor the parser to support nested groups.' ;;
  b3) print -r -- 'The tool reads your config file and builds an index of every entry. It writes the result to disk, then reports the total count.' ;;
  b4) print -r -- 'The retry logic needs work.' ;;
  b5) print -r -- 'Fixed a bug in the exporter. Users reported it on 2026-07-14 and it affected 3 of the 12 export formats.' ;;
  b6) print -r -- 'Offline edits now work in this mode. Existing users need no rollout changes.' ;;
  b7) print -r -- "This removes the old cache and reduces latency. The exact reduction is not known."
    ;;
  b8) print -r -- 'The migration runs in two phases: the first copies rows, and the second flips the read path. Both are idempotent.' ;;
  b9) print -r -- 'Unsupported claims: the benchmark shows that p50 went from 340ms to 210ms, not a 10x improvement. It does not prove that the endpoint eliminates all timeouts.' ;;
  b10) print -r -- 'The draft does not state what the tool does.' ;;
  b11) print -r -- 'Predictability matters more than raw speed. The scheduler admits at most 8 jobs per window.' ;;
  b12) print -r -- 'The default is 3 attempts.' ;;
  b13) print -r -- 'The inverted index built at startup makes this fast.' ;;
  b14) print -r -- 'The API is stable. Callers should migrate from /v1/export before version 3.0 removes the route.' ;;
  b15) print -r -- 'The draft does not identify the changed item. Add the missing object before this edit.' ;;
  hidden) print -r -- 'The authorized response is clear.' ;;
  *) exit 64 ;;
esac
