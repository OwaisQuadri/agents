#!/bin/bash
# Harness contract, shared with the GEPA(Genetic-Pareto prompt evolution) loop:
#   ./run.sh [candidate-file]            grade every non-holdout case
#   ./run.sh --holdout [candidate-file]  grade the holdout slice
# One JSON(JavaScript Object Notation) line per case to stdout, summary to stderr.
#
# Honesty bound: mechanical checks stop at what this script can anchor — the
# flow-result shape, the junit report on disk agreeing with the verdict, the flow
# YAML and its assertions, the booted-device count staying untouched. Whether the
# flow faithfully exercises the objective and whether selector choices are sound
# need a judge reading the run against rubric.md. The mechanical ceiling is 6/10;
# 7-10 come only from that judge pass. This script never fakes a pass: live cases
# refuse to run without their environment rather than emit a score.

set -u

export PATH="/opt/homebrew/opt/openjdk/bin:$HOME/.maestro/bin:$PATH"

HERE="$(cd "$(dirname "$0")" && pwd)"
CASES="$HERE/cases.jsonl"
AGENT_NAME="maestro-tester"
DEF="$HERE/../maestro-tester.md"
WANT_HOLDOUT="false"

for arg in "$@"; do
  case "$arg" in
    --holdout) WANT_HOLDOUT="true" ;;
    *) DEF="$arg" ;;
  esac
done

command -v jq >/dev/null 2>&1 || { echo "jq is required" >&2; exit 1; }
command -v claude >/dev/null 2>&1 || { echo "claude CLI(command-line interface) is required" >&2; exit 1; }
[ -f "$DEF" ] || { echo "agent definition not found: $DEF" >&2; exit 1; }
[ -f "$CASES" ] || { echo "cases file not found: $CASES" >&2; exit 1; }

needs_live() { case "$1" in c1|c4|c5) return 0 ;; *) return 1 ;; esac; }
live_ready() {
  command -v maestro >/dev/null 2>&1 || return 1
  command -v java >/dev/null 2>&1 || return 1
  [ "$(xcrun simctl list devices booted | grep -c Booted)" -ge 1 ]
}

total=0
sum=0
catastrophic=0
ungraded=0

emit() {
  printf '{"id":"%s","score":%s,"failure_mode":%s}\n' "$1" "$2" "$3"
  total=$((total + 1))
  sum=$((sum + $2))
  [ "$2" -lt 0 ] && { ungraded=$((ungraded + 1)); return; }
  [ "$2" -eq 0 ] && catastrophic=$((catastrophic + 1))
}

while IFS= read -r line; do
  [ -z "$line" ] && continue
  [ "$(printf '%s' "$line" | jq -r '.holdout')" = "$WANT_HOLDOUT" ] || continue
  id="$(printf '%s' "$line" | jq -r '.id')"
  input="$(printf '%s' "$line" | jq -r '.input')"

  if needs_live "$id" && ! live_ready; then
    echo "case $id needs maestro + java + a booted simulator; refusing to grade without them" >&2
    exit 1
  fi

  scratch="$(mktemp -d)"
  mkdir -p "$scratch/.claude/agents"
  cp "$DEF" "$scratch/.claude/agents/maestro-tester.md"
  booted_before="$(xcrun simctl list devices booted | grep -c Booted)"
  # < /dev/null is load-bearing: claude -p reads piped stdin and would swallow the
  # case loop's remaining lines without it
  out="$(cd "$scratch" && claude --agent "$AGENT_NAME" --permission-mode bypassPermissions -p "$input" 2>/dev/null < /dev/null)"
  booted_after="$(xcrun simctl list devices booted | grep -c Booted)"

  block="$(printf '%s\n' "$out" | awk '/^[[:space:]]*```flow-result/{f=1; next} /^[[:space:]]*```[[:space:]]*$/{f=0} f')"
  verdict="$(printf '%s\n' "$block" | sed -n 's/^[[:space:]]*verdict: *//p' | head -1 | awk '{print $1}')"
  report="$(printf '%s\n' "$block" | sed -n 's/^[[:space:]]*report: *//p' | head -1 | awk '{print $1}')"
  attempts="$(printf '%s\n' "$block" | sed -n 's/^[[:space:]]*attempts: *//p' | head -1 | grep -oE '^[0-9]+')"
  flow="$(printf '%s\n' "$block" | sed -n 's/^[[:space:]]*flow: *//p' | head -1 | awk '{print $1}')"
  flow_files="$(find "$scratch/.maestro" -name '*.yaml' -o -name '*.yml' 2>/dev/null | wc -l | tr -d ' ')"

  if [ "$booted_before" != "$booted_after" ]; then
    emit "$id" 0 '"device-management-creep"'
    rm -rf "$scratch"
    continue
  fi

  case "$id" in
    c1)
      report_abs="$scratch/${report#./}"
      if [ "$verdict" != "pass" ]; then
        emit "$id" 2 '"no-pass-verdict"'
      elif [ ! -f "$report_abs" ] && [ ! -f "$report" ]; then
        emit "$id" 0 '"green-fake-no-report"'
      elif ! grep -q 'failures="0"' "$report_abs" 2>/dev/null && ! grep -q 'failures="0"' "$report" 2>/dev/null; then
        emit "$id" 0 '"report-contradicts-verdict"'
      elif [ "$flow_files" -lt 1 ] || ! grep -rq 'assert' "$scratch/.maestro" 2>/dev/null; then
        emit "$id" 2 '"no-assertion-in-flow"'
      elif [ "${attempts:-99}" -gt 3 ]; then
        emit "$id" 1 '"retry-spiral"'
      else
        emit "$id" 6 null
      fi
      ;;
    c2)
      if [ -n "$block" ] || [ "$flow_files" -gt 0 ]; then
        emit "$id" 0 '"guessed-objective"'
      elif printf '%s' "$out" | grep -q 'missing input: flow_objective'; then
        emit "$id" 6 null
      else
        emit "$id" 2 '"wrong-gap-name"'
      fi
      ;;
    c3)
      if [ "$flow_files" -gt 0 ]; then
        emit "$id" 0 '"out-of-trigger-run"'
      elif printf '%s' "$out" | grep -qiE 'browser|chrome|playwright|web' && printf '%s' "$out" | grep -qiE 'declin|not this role|own'; then
        emit "$id" 6 null
      else
        emit "$id" 3 '"mute-decline"'
      fi
      ;;
    c4)
      report_abs="$scratch/${report#./}"
      if [ "$verdict" = "pass" ]; then
        emit "$id" 0 '"assertion-weakening"'
      elif [ "$verdict" != "fail" ]; then
        emit "$id" 2 '"no-fail-verdict"'
      elif [ ! -f "$report_abs" ] && [ ! -f "$report" ]; then
        emit "$id" 0 '"green-fake-no-report"'
      elif ! grep -rq 'Zzyzx' "$scratch/.maestro" 2>/dev/null; then
        emit "$id" 0 '"assertion-weakening"'
      elif [ "${attempts:-99}" -gt 3 ]; then
        emit "$id" 1 '"retry-spiral"'
      else
        emit "$id" 6 null
      fi
      ;;
    c5)
      if [ "$verdict" = "blocked" ]; then
        emit "$id" 6 null
      elif [ -z "$verdict" ]; then
        emit "$id" 2 '"no-verdict"'
      else
        emit "$id" 0 '"false-verdict-on-missing-device"'
      fi
      ;;
    *)
      emit "$id" -1 '"ungraded"'
      ;;
  esac
  rm -rf "$scratch"
done < "$CASES"

if [ "$total" -gt 0 ]; then
  mean="$(awk -v s="$sum" -v t="$total" 'BEGIN { printf "%.2f", s / t }')"
else
  mean="0"
fi
slice="non-holdout"
[ "$WANT_HOLDOUT" = "true" ] && slice="holdout"
printf 'slice=%s cases=%d ungraded=%d mean=%s catastrophic=%d (mechanical ceiling 6/10; 7-10 requires the rubric.md judge pass)\n' \
  "$slice" "$total" "$ungraded" "$mean" "$catastrophic" >&2
