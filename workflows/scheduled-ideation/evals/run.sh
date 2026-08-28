#!/bin/bash
# Harness contract, shared with the GEPA(Genetic-Pareto prompt evolution) loop:
#   ./run.sh [candidate-file]            grade every non-holdout case
#   ./run.sh --holdout [candidate-file]  grade the holdout slice
# One JSON(JavaScript Object Notation) line per case to stdout, summary to stderr.
#
# Honesty bound: this workflow only executes inside the Workflow tool, which no shell
# can invoke, so the mechanical layer is static topology checks on the script source —
# guard clauses, caps, context isolation, fan-in fields. The mechanical ceiling is
# 5/10; scores of 6-10 require a judge running the workflow live against rubric.md.
# This script never fakes a pass.

set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
CASES="$HERE/cases.jsonl"
DEF="$HERE/../scheduled-ideation.workflow.js"
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
      if has "if \(!rawCandidates.length\)" && has "no candidates today"; then
        emit "$id" 5 null
      else
        emit "$id" 0 '"zero-candidate-guard-missing"'
      fi
      ;;
    c2)
      if has "MAX_TOOL_RADAR = Math.min" && has "slice\(0, MAX_TOOL_RADAR\)" \
        && has "MAX_DIGEST_CANDIDATES = 10" && has "slice\(0, MAX_DIGEST_CANDIDATES\)"; then
        emit "$id" 5 null
      else
        emit "$id" 0 '"cap-missing"'
      fi
      ;;
    c3)
      if printf '%s' "$src" | grep -A6 'fresh-context filter' | grep -qE 'transcript|conversation history'; then
        emit "$id" 0 '"filter-context-leak"'
      elif has "rawCandidates.map\(\(c, i\)"; then
        emit "$id" 5 null
      else
        emit "$id" 2 '"filter-isolation-unverifiable"'
      fi
      ;;
    c4)
      if has "generateMissing" && has "missingLabels: generateMissing" && has "returned: generateResults.length"; then
        emit "$id" 5 null
      else
        emit "$id" 0 '"silent-partial"'
      fi
      ;;
    c5)
      # Mining runs on the default session model (no agentType override) after two real
      # 2026-08-28 live runs both aborted agentType 'Explore' mid-task against Pi's own
      # session-transcript directory; tool-radar still routes to web-research-summarizer.
      # The mechanical check confirms mining is NOT routed to web-research-summarizer
      # (the wrong tool for a codebase-mining source) and tool-radar still is.
      MINING_BLOCK="$(printf '%s' "$src" | awk '/parallel\(toolRadar\.map/{exit} /parallel\(mining\.map/{p=1} p{print}')"
      if has "agentType: 'web-research-summarizer'" && has "mining.map" && has "toolRadar.map" \
        && ! grep -q "agentType: 'web-research-summarizer'" <<<"$MINING_BLOCK"; then
        emit "$id" 5 null
      else
        emit "$id" 0 '"routing-missing"'
      fi
      ;;
    c6)
      if has "skills/ai-author/SKILL.md" && has "read that file first, then follow its numbered steps" \
        && ! has "Read no more than ten artifacts"; then
        emit "$id" 5 null
      else
        emit "$id" 0 '"procedure-duplicated-inline"'
      fi
      ;;
    c7)
      if has "omit a heading entirely if it has zero candidates" && has "never write an empty section"; then
        emit "$id" 5 null
      else
        emit "$id" 1 '"empty-heading-not-guarded"'
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
