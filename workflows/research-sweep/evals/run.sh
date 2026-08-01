#!/bin/bash
# Harness contract, shared with the GEPA(Genetic-Pareto prompt evolution) loop:
#   ./run.sh [candidate-file]            grade every non-holdout case
#   ./run.sh --holdout [candidate-file]  grade the holdout slice
# One JSON(JavaScript Object Notation) line per case to stdout, summary to stderr.
#
# Honesty bound: this workflow only executes inside Claude Code's Workflow tool,
# which no shell can invoke, so the mechanical layer is static topology checks on
# the script source — guard clauses, caps, context isolation, fan-in fields. The
# mechanical ceiling is 5/10; scores of 6-10 require a judge running the workflow
# live against rubric.md. This script never fakes a pass.

set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
CASES="$HERE/cases.jsonl"
DEF="$HERE/../research-sweep.workflow.js"
WANT_HOLDOUT="false"

for arg in "$@"; do
  case "$arg" in
    --holdout) WANT_HOLDOUT="true" ;;
    *) DEF="$arg" ;;
  esac
done

command -v jq >/dev/null 2>&1 || { echo "jq is required" >&2; exit 1; }
[ -f "$DEF" ] || { echo "workflow script not found: $DEF" >&2; exit 1; }
[ -f "$CASES" ] || { echo "cases file not found: $CASES" >&2; exit 1; }

total=0
sum=0
catastrophic=0

emit() {
  printf '{"id":"%s","score":%s,"failure_mode":%s}\n' "$1" "$2" "$3"
  total=$((total + 1))
  sum=$((sum + $2))
  [ "$2" -eq 0 ] && catastrophic=$((catastrophic + 1))
}

src="$(cat "$DEF")"
has() { printf '%s' "$src" | grep -qE "$1"; }

while IFS= read -r line; do
  [ -z "$line" ] && continue
  [ "$(printf '%s' "$line" | jq -r '.holdout')" = "$WANT_HOLDOUT" ] || continue
  id="$(printf '%s' "$line" | jq -r '.id')"

  case "$id" in
    c1)
      if has "missing input: goal" && has "if \(!GOAL\) return"; then
        emit "$id" 5 null
      else
        emit "$id" 0 '"goal-guard-missing"'
      fi
      ;;
    c2)
      if has "MAX_PLANNED" && has "MAX_FILL" && has "slice\(0, *MAX_PLANNED\)" && has "slice\(0, *MAX_FILL\)"; then
        emit "$id" 5 null
      else
        emit "$id" 0 '"cap-missing"'
      fi
      ;;
    c3)
      if printf '%s' "$src" | grep -A2 'completeness critic' | grep -qE 'transcript|conversation history'; then
        emit "$id" 0 '"critic-context-leak"'
      elif has "findings block: \\\$\{b.label\}"; then
        emit "$id" 5 null
      else
        emit "$id" 2 '"critic-input-unverifiable"'
      fi
      ;;
    c4)
      if has "critic.missing.slice" && has "isSufficient"; then
        emit "$id" 5 null
      else
        emit "$id" 1 '"no-fill-round"'
      fi
      ;;
    c5)
      # invariant: a dead researcher reaches the fan-in guard as null, never as a
      # wrapped truthy block
      if has "missingLabels" && has "returned" && has "expected" \
        && has "text \? \{ label" && has "plan node returned nothing"; then
        emit "$id" 5 null
      else
        emit "$id" 0 '"silent-partial"'
      fi
      ;;
    *)
      emit "$id" 1 '"unknown-case"'
      ;;
  esac
done < "$CASES"

if [ "$total" -gt 0 ]; then
  mean="$(awk -v s="$sum" -v t="$total" 'BEGIN { printf "%.2f", s / t }')"
else
  mean="0"
fi
slice="non-holdout"
[ "$WANT_HOLDOUT" = "true" ] && slice="holdout"
printf 'slice=%s cases=%d mean=%s catastrophic=%d (mechanical ceiling 5/10; 6-10 requires a live judge run per rubric.md)\n' \
  "$slice" "$total" "$mean" "$catastrophic" >&2
