#!/bin/bash
# Harness contract, shared with the GEPA(Genetic-Pareto prompt evolution) loop:
#   ./run.sh [candidate-file]            grade every non-holdout case
#   ./run.sh --holdout [candidate-file]  grade the holdout slice
# One JSON(JavaScript Object Notation) line per case to stdout, summary to stderr.
#
# Honesty bound: the classifier is executable, so its cases run live through
# scan.sh --classify; digest voice, gate discipline, dispatch isolation, and
# triage judgment are not shell-checkable. The mechanical ceiling is 6/10;
# 7-10 requires a judge grading a live /hq run against rubric.md. This script
# never fakes a pass.

set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
CASES="$HERE/cases.jsonl"
DEF="$HERE/../scripts/scan.sh"
SKILL="$HERE/../SKILL.md"
HEARTBEAT="$HERE/../scripts/heartbeat.sh"
WANT_HOLDOUT="false"

for arg in "$@"; do
  case "$arg" in
    --holdout) WANT_HOLDOUT="true" ;;
    *) DEF="$arg" ;;
  esac
done

command -v jq >/dev/null 2>&1 || { echo "jq is required" >&2; exit 1; }
[ -f "$DEF" ] || { echo "classifier not found: $DEF" >&2; exit 1; }
[ -f "$CASES" ] || { echo "cases file not found: $CASES" >&2; exit 1; }

bash -n "$DEF" || { echo "syntax error in $DEF" >&2; exit 1; }
[ -f "$HEARTBEAT" ] && { bash -n "$HEARTBEAT" || { echo "syntax error in $HEARTBEAT" >&2; exit 1; }; }
if [ -f "$SKILL" ]; then
  for needle in "^JOB:" "cannot speak into" "reset-spec.md" "^## logging" "kind:\"merge\""; do
    grep -qE "$needle" "$SKILL" || { echo "SKILL.md missing required section: $needle" >&2; exit 1; }
  done
fi

total=0
sum=0
catastrophic=0

emit() {
  printf '{"id":"%s","score":%s,"failure_mode":%s}\n' "$1" "$2" "$3"
  total=$((total + 1))
  sum=$((sum + $2))
  [ "$2" -eq 0 ] && catastrophic=$((catastrophic + 1))
}

while IFS= read -r line; do
  [ -z "$line" ] && continue
  [ "$(printf '%s' "$line" | jq -r '.holdout')" = "$WANT_HOLDOUT" ] || continue
  id="$(printf '%s' "$line" | jq -r '.id')"

  tmpd="$(mktemp -d)"
  prev_json="$(printf '%s' "$line" | jq -c '.input.prev')"
  if [ "$prev_json" = '"-"' ]; then
    prev_arg="-"
  else
    printf '%s' "$prev_json" > "$tmpd/prev.json"
    prev_arg="$tmpd/prev.json"
  fi
  printf '%s' "$line" | jq -c '.input.curr' > "$tmpd/curr.json"

  if ! out="$(/bin/bash "$DEF" --classify "$prev_arg" "$tmpd/curr.json" 2>"$tmpd/err")"; then
    emit "$id" 0 '"classifier-crashed"'
    rm -rf "$tmpd"
    continue
  fi
  rm -rf "$tmpd"

  if [ -z "$out" ]; then
    got_anom=""
    got_routine=""
  else
    got_anom="$(printf '%s' "$out" | jq -r '[.anomalies[].kind] | sort | join(",")')"
    got_routine="$(printf '%s' "$out" | jq -r '[.routine[].kind] | sort | join(",")')"
  fi

  case "$id" in
    c1) want_anom=""; want_routine="" ;;
    c2) want_anom="launchd_down"; want_routine="" ;;
    c3) want_anom="job_state_changed"; want_routine="" ;;
    c4) want_anom=""; want_routine="workspace_updated" ;;
    c5) want_anom="job_stuck"; want_routine="" ;;
    c6) want_anom=""; want_routine="session_ended" ;;
    c7) want_anom=""; want_routine="" ;;
    *) emit "$id" 1 '"unknown-case"'; continue ;;
  esac

  if [ "$got_anom" = "$want_anom" ] && [ "$got_routine" = "$want_routine" ]; then
    emit "$id" 6 null
  elif [ -n "$want_anom" ] && [ -z "$got_anom" ]; then
    emit "$id" 0 '"false-quiet"'
  elif [ -z "$want_anom" ] && [ -n "$got_anom" ]; then
    emit "$id" 0 '"false-alarm"'
  else
    emit "$id" 2 '"routine-mismatch"'
  fi
done < "$CASES"

if [ "$total" -gt 0 ]; then
  mean="$(awk -v s="$sum" -v t="$total" 'BEGIN { printf "%.2f", s / t }')"
else
  mean="0"
fi
slice="non-holdout"
[ "$WANT_HOLDOUT" = "true" ] && slice="holdout"
printf 'slice=%s cases=%d mean=%s catastrophic=%d (mechanical ceiling 6/10; 7-10 requires a live judge run per rubric.md)\n' \
  "$slice" "$total" "$mean" "$catastrophic" >&2
