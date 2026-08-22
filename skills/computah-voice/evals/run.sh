#!/usr/bin/env bash
# TODO(AGNT-0032.T25): make the computah-voice exam execute real artifact behavior
# run.sh — computah-voice eval runner
# usage: ./run.sh [candidate-line-file]   (mechanical pass over ste-check)
#        ./run.sh --holdout               (runs the holdout slice instead)
#
# Mechanical ceiling: ste-check --register computah only grades the syntactic
# rules (the shared STE set, plus no markdown, line count, banned words,
# sentence-count guidance) against a single candidate spoken line
# passed on argv or stdin. It cannot judge whether the line actually answers
# a case's "expect" in cases.jsonl — that's the rubric.md judge's job, run
# separately over real logged output, same split as the hq skill's evals.
set -euo pipefail
cd "$(dirname "$0")/.."

CASES="evals/cases.jsonl"
HOLDOUT=0
[[ "${1:-}" == "--holdout" ]] && HOLDOUT=1

total=0
pass=0
while IFS= read -r line; do
  is_holdout=$(python3 -c "import json,sys; print(json.loads(sys.argv[1])['holdout'])" "$line")
  id=$(python3 -c "import json,sys; print(json.loads(sys.argv[1])['id'])" "$line")
  if [[ "$HOLDOUT" == "1" && "$is_holdout" != "True" ]]; then continue; fi
  if [[ "$HOLDOUT" == "0" && "$is_holdout" == "True" ]]; then continue; fi
  total=$((total + 1))
  echo "{\"id\":\"$id\",\"note\":\"mechanical ste-check only; run this case's actual spoken output through ste-check --register computah by hand, then grade against rubric.md\"}"
done < "$CASES"

echo "cases listed: $total (mechanical structure only; judge rubric.md separately)" >&2
