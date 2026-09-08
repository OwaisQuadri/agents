#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
source ../../../tools/skill-eval/timing.sh
command -v jq >/dev/null || { echo 'jq required' >&2; exit 1; }
slice=nonholdout
if [ "${1:-}" = --holdout ]; then
  slice=holdout
  shift
fi
def="${1:-../implementer.md}"
[ -f "$def" ] || { echo 'agent definition not found' >&2; exit 1; }
timing_preflight || exit $?
timing_begin implementer "$slice" "$def" || exit $?
case_count=0
ungraded=0
while IFS= read -r case_line; do
  [[ -n "$case_line" ]] || continue
  id=$(jq -r .id <<<"$case_line")
  case_count=$((case_count + 1))
  expect=$(jq -r .expect <<<"$case_line")
  timing_case_begin
  case_started="$(timing_now_ms)"
  fix=$(mktemp -d)
  trap 'rm -rf "$fix"' EXIT
  mkdir -p "$fix/src"
  printf 'package = "acme-sdk"\nversion = "1.2.3"\n' > "$fix/pyproject.toml"
  printf '# fixture\n' > "$fix/src/cache.py"
  input=$(jq -r .input <<<"$case_line" | sed "s|__FIXTURE__|$fix|g")
  prompt="You are dispatched as the implementer agent. Execute the dispatch and reply in the exact output contract.\n\n$input"
  out_file=$(mktemp)
  if timing_dispatch implementer "$def" "$fix" "$(printf '%b' "$prompt")" > "$out_file"; then
    dispatch_status=0
  else
    dispatch_status=$?
  fi
  out=$(cat "$out_file")
  rm -f "$out_file"
  if [ "$dispatch_status" -ne 0 ]; then
    timing_case "$id" -1 "$(jq -Rn --arg value "dispatch-failed:$dispatch_status" '$value')" "$case_started"
    ungraded=$((ungraded + 1))
    rm -rf "$fix"; trap - EXIT
    continue
  fi
  score=0; mode='missing-output'
  if grep -q 'status:' <<<"$out" && grep -q 'version_basis:' <<<"$out" && grep -q 'verification:' <<<"$out"; then
    score=6; mode='shape-present'
  fi
  if [[ "$expect" == *'invalid-dispatch'* ]] && grep -q 'invalid-dispatch' <<<"$out"; then score=10; mode='correct-invalid-dispatch'; fi
  if [[ "$expect" == *'out-of-trigger'* ]] && grep -q 'out-of-trigger' <<<"$out"; then score=10; mode='correct-out-of-trigger'; fi
  timing_case "$id" "$score" "$(jq -Rn --arg m "$mode" '$m')" "$case_started"
  rm -rf "$fix"; trap - EXIT
done < <(if [[ "$slice" == holdout ]]; then jq -c 'select(.holdout == true)' cases.jsonl; else jq -c 'select(.holdout == false)' cases.jsonl; fi)
printf 'slice=%s cases=%d ungraded=%d\n' "$slice" "$((case_count - ungraded))" "$ungraded" >&2
timing_complete || exit $?
[ "$ungraded" -eq 0 ] || exit 2
