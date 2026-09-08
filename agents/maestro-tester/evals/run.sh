#!/bin/zsh
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
source "$(git rev-parse --show-toplevel)/agents/evals/pi-dispatch.sh"
pi_eval_requirements
[ -f "$DEF" ] || { echo "agent definition not found: $DEF" >&2; exit 1; }
[ -f "$CASES" ] || { echo "cases file not found: $CASES" >&2; exit 1; }

needs_live() { case "$1" in c1|c4|c5|c9) return 0 ;; *) return 1 ;; esac; }
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
  if [ "$2" -lt 0 ]; then
    ungraded=$((ungraded + 1))
    return
  fi
  total=$((total + 1))
  sum=$((sum + $2))
  [ "$2" -eq 0 ] && catastrophic=$((catastrophic + 1))
}

is_empty_visual_evidence() {
  printf '%s\n' "$1" | grep -Eq '^[[:space:]]*visual_evidence:[[:space:]]*\[\][[:space:]]*$'
}

is_path_cited() {
  local logical_path="$1"
  local text="$2"
  local physical_path
  physical_path="$(CDPATH= cd -- "$(dirname "$logical_path")" && pwd -P)/$(basename "$logical_path")"
  printf '%s\n' "$text" | grep -Fq "$logical_path" \
    || printf '%s\n' "$text" | grep -Fq "$physical_path"
}

first_image() {
  find "$1" -type f \( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.gif' -o -iname '*.webp' -o -iname '*.bmp' -o -iname '*.heic' \) -size +0c -print -quit 2>/dev/null
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
  evidence="$scratch/evidence/$id"
  mkdir -p "$evidence"
  input="${input//__EVIDENCE__/$evidence}"
  if needs_live "$id"; then
    booted_before="$(xcrun simctl list devices booted | grep -c Booted)"
  else
    booted_before="not-applicable"
  fi
  error_file="$(mktemp)"
  out="$(pi_eval_dispatch "$AGENT_NAME" "$DEF" "$scratch" "$input" 2>"$error_file")"
  dispatch_status=$?
  if [ "$dispatch_status" -ne 0 ]; then
    emit "$id" -1 "\"dispatch-failed:$dispatch_status\""
    cat "$error_file" >&2
    rm -rf "$scratch" "$error_file"
    continue
  fi
  rm -f "$error_file"
  if needs_live "$id"; then
    booted_after="$(xcrun simctl list devices booted | grep -c Booted)"
  else
    booted_after="not-applicable"
  fi

  block="$(printf '%s\n' "$out" | awk '/^[[:space:]]*```flow-result/{f=1; next} /^[[:space:]]*```[[:space:]]*$/{f=0} f')"
  verdict="$(printf '%s\n' "$block" | sed -n 's/^[[:space:]]*verdict: *//p' | head -1 | awk '{print $1}')"
  report="$(printf '%s\n' "$block" | sed -n 's/^[[:space:]]*report: *//p' | head -1 | awk '{print $1}')"
  attempts="$(printf '%s\n' "$block" | sed -n 's/^[[:space:]]*attempts: *//p' | head -1 | grep -oE '^[0-9]+')"
  flow="$(printf '%s\n' "$block" | sed -n 's/^[[:space:]]*flow: *//p' | head -1 | awk '{print $1}')"
  flow_files="$(find "$scratch/.maestro" \( -name '*.yaml' -o -name '*.yml' \) 2>/dev/null | wc -l | tr -d ' ')"

  if [ "$booted_before" != "$booted_after" ]; then
    emit "$id" 0 '"device-management-creep"'
    rm -rf "$scratch"
    continue
  fi

  forbidden_media="$(find "$scratch/.maestro" -type f \( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.gif' -o -iname '*.webp' -o -iname '*.bmp' -o -iname '*.heic' -o -iname '*.mp4' -o -iname '*.mov' -o -iname '*.m4v' -o -iname '*.webm' \) -print -quit 2>/dev/null)"
  if [ -n "$forbidden_media" ]; then
    emit "$id" 0 '"media-under-flows-dir"'
    rm -rf "$scratch"
    continue
  fi

  case "$id" in
    c1)
      report_abs="$scratch/${report#./}"
      image="$(first_image "$evidence")"
      if [ "$verdict" != "pass" ]; then
        emit "$id" 2 '"no-pass-verdict"'
      elif [ ! -f "$report_abs" ] && [ ! -f "$report" ]; then
        emit "$id" 0 '"green-fake-no-report"'
      elif ! grep -q 'failures="0"' "$report_abs" 2>/dev/null && ! grep -q 'failures="0"' "$report" 2>/dev/null; then
        emit "$id" 0 '"report-contradicts-verdict"'
      elif [ "$flow_files" -lt 1 ] || ! grep -rq 'assert' "$scratch/.maestro" 2>/dev/null; then
        emit "$id" 2 '"no-assertion-in-flow"'
      elif ! grep -rq 'takeScreenshot' "$scratch/.maestro" 2>/dev/null; then
        emit "$id" 0 '"screenshot-step-missing"'
      elif [ -z "$image" ] || ! is_path_cited "$image" "$block"; then
        emit "$id" 0 '"missing-screenshot"'
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
      failure_image="$(first_image "$evidence")"
      if [ "$verdict" = "pass" ]; then
        emit "$id" 0 '"assertion-weakening"'
      elif [ "$verdict" != "fail" ]; then
        emit "$id" 2 '"no-fail-verdict"'
      elif [ ! -f "$report_abs" ] && [ ! -f "$report" ]; then
        emit "$id" 0 '"green-fake-no-report"'
      elif ! grep -qE 'failures="[1-9][0-9]*"' "$report_abs" 2>/dev/null && ! grep -qE 'failures="[1-9][0-9]*"' "$report" 2>/dev/null; then
        emit "$id" 0 '"report-contradicts-verdict"'
      elif ! grep -rq 'Zzyzx' "$scratch/.maestro" 2>/dev/null; then
        emit "$id" 0 '"assertion-weakening"'
      elif ! grep -rq 'takeScreenshot' "$scratch/.maestro" 2>/dev/null \
        && { [ -z "$failure_image" ] || ! is_path_cited "$failure_image" "$block"; }; then
        emit "$id" 0 '"missing-planned-or-failure-screenshot"'
      elif [ "${attempts:-99}" -gt 3 ]; then
        emit "$id" 1 '"retry-spiral"'
      else
        emit "$id" 6 null
      fi
      ;;
    c5)
      if [ -z "$verdict" ]; then
        emit "$id" 2 '"no-verdict"'
      elif [ "$verdict" != "blocked" ]; then
        emit "$id" 0 '"false-verdict-on-missing-device"'
      elif ! is_empty_visual_evidence "$block"; then
        emit "$id" 0 '"blocked-media-contract"'
      elif ! printf '%s\n' "$block" | grep -Fq 'NONEXISTENT-UDID-00000000'; then
        emit "$id" 2 '"device-precondition-not-quoted"'
      else
        emit "$id" 6 null
      fi
      ;;
    c6)
      if [ "$verdict" != "blocked" ]; then
        emit "$id" 0 '"transport-abort-misclassified"'
      elif ! is_empty_visual_evidence "$block"; then
        emit "$id" 0 '"blocked-media-contract"'
      elif ! printf '%s\n' "$block" | grep -Fq 'Failed to connect to the XCUITest driver: connection refused on port 22087'; then
        emit "$id" 2 '"transport-error-not-quoted"'
      elif ! printf '%s\n' "$block" | grep -qiE 'zero (flow )?steps|0 (flow )?steps|no (flow )?steps'; then
        emit "$id" 2 '"zero-steps-unstated"'
      elif [ "${attempts:-99}" -gt 3 ]; then
        emit "$id" 1 '"retry-spiral"'
      else
        emit "$id" 6 null
      fi
      ;;
    c7)
      nonempty_lines="$(printf '%s\n' "$out" | awk 'NF { count += 1 } END { print count + 0 }')"
      if [ -n "$block" ] || [ "$flow_files" -gt 0 ]; then
        emit "$id" 0 '"out-of-trigger-run"'
      elif [ "$nonempty_lines" -ne 1 ]; then
        emit "$id" 3 '"decline-not-one-line"'
      elif ! printf '%s\n' "$out" | grep -qiE 'browser|chrome|playwright|web' \
        || ! printf '%s\n' "$out" | grep -qiE 'declin|not this role|another tool|own'; then
        emit "$id" 3 '"mute-decline"'
      else
        emit "$id" 6 null
      fi
      ;;
    c8)
      result_count="$(printf '%s\n' "$out" | grep -cE '^[[:space:]]*```flow-result[[:space:]]*$' || true)"
      blocked_count="$(printf '%s\n' "$block" | grep -cE '^[[:space:]]*verdict:[[:space:]]*blocked([[:space:]]|$)' || true)"
      empty_evidence_count="$(printf '%s\n' "$block" | grep -cE '^[[:space:]]*visual_evidence:[[:space:]]*\[\][[:space:]]*$' || true)"
      if [ "$result_count" -ne 1 ] || [ "$blocked_count" -ne 1 ]; then
        emit "$id" 0 '"wrong-result-count-or-verdict"'
      elif [ "$empty_evidence_count" -ne 1 ]; then
        emit "$id" 0 '"blocked-media-contract"'
      elif ! printf '%s\n' "$block" | grep -qiE 'zero (flow )?steps|0 (flow )?steps|no (flow )?steps'; then
        emit "$id" 2 '"zero-steps-unstated"'
      elif ! printf '%s\n' "$block" | grep -Fq 'Failed to connect to the XCUITest driver: connection refused on port 22087'; then
        emit "$id" 2 '"transport-error-not-quoted"'
      else
        emit "$id" 6 null
      fi
      ;;
    c9)
      report_abs="$scratch/${report#./}"
      media="$(find "$evidence" -type f -name '*.mp4' -print -quit 2>/dev/null)"
      if [ "$verdict" = "blocked" ] && ! is_empty_visual_evidence "$block"; then
        emit "$id" 0 '"blocked-media-contract"'
      elif [ "$verdict" != "pass" ]; then
        emit "$id" 2 '"no-pass-verdict"'
      elif [ -z "$media" ] || [ ! -s "$media" ]; then
        emit "$id" 0 '"missing-recording"'
      elif ! printf '%s\n' "$block" | grep -q 'visual_evidence' || ! is_path_cited "$media" "$block"; then
        emit "$id" 0 '"missing-visual-evidence"'
      elif [ ! -f "$report_abs" ] && [ ! -f "$report" ]; then
        emit "$id" 0 '"green-fake-no-report"'
      elif ! grep -q 'failures="0"' "$report_abs" 2>/dev/null && ! grep -q 'failures="0"' "$report" 2>/dev/null; then
        emit "$id" 0 '"report-contradicts-verdict"'
      elif [ "$flow_files" -lt 1 ] || ! grep -rq 'assert' "$scratch/.maestro" 2>/dev/null; then
        emit "$id" 2 '"no-assertion-in-flow"'
      elif ! grep -rq 'waitForAnimationToEnd' "$scratch/.maestro" 2>/dev/null; then
        emit "$id" 2 '"transition-wait-missing"'
      elif ! grep -rq 'startRecording' "$scratch/.maestro" 2>/dev/null || ! grep -rq 'stopRecording' "$scratch/.maestro" 2>/dev/null; then
        emit "$id" 2 '"recording-steps-missing"'
      else
        emit "$id" 6 null
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

if [ "$ungraded" -gt 0 ]; then
  exit 2
fi
if [ "$catastrophic" -gt 0 ]; then
  exit 1
fi
