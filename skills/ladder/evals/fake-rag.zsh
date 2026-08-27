#!/bin/zsh
set -euo pipefail

workspace=${LADDER_EVAL_WORKSPACE:?}
case "$LADDER_EVAL_CASE_ID" in
  c1)
    jq -cn --arg path "$workspace/learning/PRIOR-PLAN.md" '{results:[{source_path:$path,excerpt:"Agreed six-step Rust plan: P1 through P6."}]}'
    ;;
  *)
    print -r -- '{"results":[]}'
    ;;
esac
