#!/bin/zsh
set -euo pipefail

workspace=${SESSION_STATS_EVAL_WORKSPACE:?}
audit=${SESSION_STATS_EVAL_AUDIT:?}
print -r -- "session-stats $*" >> "$audit"

json_target=""
out_target=""
is_open=false
while (( $# > 0 )); do
  case "$1" in
    --json) json_target=${2:?}; shift 2 ;;
    --out) out_target=${2:?}; shift 2 ;;
    --open) is_open=true; shift ;;
    *) shift ;;
  esac
done

if [[ -n "$json_target" ]]; then
  print -r -- '[{"src":"claude","project":"-test-project","session":"aaaa-session","model":"claude-test-1","input":110,"output":90,"cacheRead":6000,"cacheCreate":500,"messages":2,"first":"2026-01-01T10:00:00.000Z","last":"2026-01-01T10:30:00.000Z","firstCtx":1300,"lastCtx":5310}]' > "$workspace/compiled.json"
  print -r -- "1 rows written to $workspace/compiled.json"
fi

if [[ -n "$out_target" ]]; then
  print -r -- '<!doctype html><title>Disposable session stats</title><p>claude-test-1: 90 output tokens</p>' > "$workspace/session-stats.html"
  if [[ "$is_open" == true ]]; then
    print -r -- "opened $workspace/session-stats.html"
  else
    print -r -- "wrote $workspace/session-stats.html"
  fi
fi
