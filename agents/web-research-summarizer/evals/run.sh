#!/bin/bash
# Harness contract, shared with the GEPA(Genetic-Pareto prompt evolution) loop:
#   ./run.sh [candidate-file]            grade every nonholdout case
#   ./run.sh --holdout [candidate-file]  grade the holdout slice
# against the incumbent definition (or the candidate, if given). One
# JSON(JavaScript Object Notation) line per case to stdout —
#   {"id":"c1","score":6,"failure_mode":null}
# — summary to stderr. A candidate replaces the incumbent only per the holdout
# gating rule in rubric.md.
#
# Honesty bound: mechanical checks stop at what this script can anchor — block
# shape, claim/source/date pairing, estimated token bound, an untouched scratch
# directory. Whether each claim answers the objective, whether each cited page
# actually supports its claim, and whether c3 truly ran zero searches need a
# judge reading the run transcript against rubric.md. The mechanical ceiling
# here is therefore 6/10; scores of 7-10 come only from that judge pass. This
# script never fakes a pass.

set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/../../../tools/skill-eval/timing.sh"
CASES="$HERE/cases.jsonl"
AGENT_NAME="web-research-summarizer"
DEF="$HERE/../web-research-summarizer.md"
WANT_HOLDOUT="false"

for arg in "$@"; do
  case "$arg" in
    --holdout) WANT_HOLDOUT="true" ;;
    *) DEF="$arg" ;;
  esac
done

command -v jq >/dev/null 2>&1 || { echo "jq is required" >&2; exit 1; }
[ -f "$DEF" ] || { echo "agent definition not found: $DEF" >&2; exit 1; }
[ -f "$CASES" ] || { echo "cases file not found: $CASES" >&2; exit 1; }
timing_preflight true || exit $?
timing_begin "$AGENT_NAME" "$( [ "$WANT_HOLDOUT" = true ] && printf holdout || printf nonholdout )" "$DEF" || exit $?
total=0
sum=0
catastrophic=0
ungraded=0

emit() {
  id="$1"
  score="$2"
  fm="$3"
  timing_case "$id" "$score" "$fm" "$case_started"
  if [ "$score" -lt 0 ]; then
    ungraded=$((ungraded + 1))
    return
  fi
  total=$((total + 1))
  sum=$((sum + score))
  [ "$score" -eq 0 ] && catastrophic=$((catastrophic + 1))
}

while IFS= read -r line; do
  [ -z "$line" ] && continue
  [ "$(printf '%s' "$line" | jq -r '.holdout')" = "$WANT_HOLDOUT" ] || continue
  id="$(printf '%s' "$line" | jq -r '.id')"
  input="$(printf '%s' "$line" | jq -r '.input')"

  timing_case_begin
  case_started="$(timing_now_ms)"
  scratch="$(mktemp -d)"
  out_file="$(mktemp)"
  if timing_dispatch "$AGENT_NAME" "$DEF" "$scratch" "$input" > "$out_file"; then
    dispatch_status=0
  else
    dispatch_status=$?
  fi
  out="$(cat "$out_file")"
  rm -f "$out_file"
  writes="$(find "$scratch" -type f | wc -l | tr -d ' ')"
  rm -rf "$scratch"
  if [ "$dispatch_status" -ne 0 ]; then
    emit "$id" -1 "$(jq -Rn --arg value "dispatch-failed:$dispatch_status" '$value')"
    continue
  fi

  block="$(printf '%s\n' "$out" | awk '/^```findings/{f=1; next} /^```/{f=0} f')"
  words=0
  [ -n "$block" ] && words="$(printf '%s\n' "$block" | wc -w | tr -d ' ')"
  # tokens ~= words * 4/3 (English prose averages ~0.75 words per token)
  tokens=$((words * 4 / 3))

  if [ "$writes" -gt 0 ]; then
    emit "$id" 0 '"file-writer"'
    continue
  fi

  case "$id" in
    c3)
      if [ -n "$block" ]; then
        emit "$id" 0 '"guessed-objective"'
      elif printf '%s' "$out" | grep -qi 'missing input: objective'; then
        emit "$id" 6 null
      else
        emit "$id" 2 '"wrong-gap-name"'
      fi
      ;;
    c4)
      # file:line citations mean the repo fan-out actually happened, fence or not;
      # bare 'explore' is excluded from the pass grep bc it matches 'explored'
      if [ -n "$block" ]; then
        emit "$id" 0 '"repo-scope-grab"'
      elif printf '%s' "$out" | grep -qE '[A-Za-z0-9_./-]+\.[a-z]{1,5}:[0-9]+'; then
        emit "$id" 0 '"repo-scope-grab"'
      elif printf '%s' "$out" | grep -qiE 'out[- ]of[- ]trigger|declin' && printf '%s' "$out" | grep -q 'Explore'; then
        emit "$id" 6 null
      else
        emit "$id" 3 '"mute-decline"'
      fi
      ;;
    *)
      claims="$(printf '%s\n' "$block" | grep -c '^- claim:')"
      srcs="$(printf '%s\n' "$block" | grep -c '^ *source:.*https*://')"
      dated="$(printf '%s\n' "$block" | grep -c '^ *source:.*20[0-9][0-9]')"
      if [ -z "$block" ]; then
        emit "$id" 0 '"shape-miss"'
      elif [ "$tokens" -gt 2200 ]; then
        emit "$id" 0 '"raw-page-dump"'
      elif [ "$claims" -eq 0 ]; then
        emit "$id" 1 '"shape-miss"'
      elif [ "$srcs" -lt "$claims" ]; then
        emit "$id" 2 '"uncited-claim"'
      elif [ "$dated" -lt "$srcs" ]; then
        emit "$id" 3 '"missing-source-date"'
      elif [ "$tokens" -lt 800 ]; then
        emit "$id" 4 '"under-filled"'
      else
        emit "$id" 6 null
      fi
      ;;
  esac
done < "$CASES"

if [ "$total" -gt 0 ]; then
  mean="$(awk -v s="$sum" -v t="$total" 'BEGIN { printf "%.2f", s / t }')"
else
  mean="0"
fi
slice="nonholdout"
[ "$WANT_HOLDOUT" = "true" ] && slice="holdout"
printf 'slice=%s cases=%d ungraded=%d mean=%s catastrophic=%d (mechanical ceiling 6/10; 7-10 requires the rubric.md judge pass)\n' \
  "$slice" "$total" "$ungraded" "$mean" "$catastrophic" >&2
timing_complete || exit $?
[ "$ungraded" -eq 0 ] || exit 2
