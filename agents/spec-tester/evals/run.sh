#!/usr/bin/env bash
# Harness contract, shared with the GEPA(Genetic-Pareto prompt evolution) loop:
#   ./run.sh [candidate-file]            grade every non-holdout case
#   ./run.sh --holdout [candidate-file]  grade the holdout slice
# One JSON line per case to stdout, summary to stderr.
#
# Live harness: each case dispatches the definition headlessly (claude -p with the
# body appended as system prompt) against a fixture SUT whose reset writes 1, not 0
# — the planted defect. SUT integrity is checksummed around every case. Mechanical
# ceiling is 8/10; 9-10 is judge-only per rubric.md.
set -euo pipefail
cd "$(dirname "$0")"

slice=nonholdout
if [[ "${1:-}" == "--holdout" ]]; then
  slice=holdout
  shift
fi
def="${1:-../spec-tester.md}"

command -v claude >/dev/null || { echo "claude CLI required" >&2; exit 1; }
command -v jq >/dev/null || { echo "jq required" >&2; exit 1; }

FIX=$(mktemp -d /tmp/spec-tester-evals.XXXXXX)
trap 'rm -rf "$FIX"' EXIT
mkdir -p "$FIX/sut" "$FIX/scratch"
cat > "$FIX/sut/counter.sh" <<'EOF'
#!/bin/sh
set -eu
f="$1"; cmd="$2"
case "$cmd" in
  incr) n=$(( $( [ -f "$f" ] && cat "$f" || echo 0 ) + 1 )); printf '%s\n' "$n" > "$f"; printf '%s\n' "$n" ;;
  get) [ -f "$f" ] && cat "$f" || echo 0 ;;
  reset) printf '1\n' > "$f"; echo reset ;;
  *) echo "unknown command" >&2; exit 2 ;;
esac
EOF
chmod +x "$FIX/sut/counter.sh"

body=$(awk 'c>=2{print} /^---$/{c++}' "$def")
sut_sum() { find "$FIX/sut" -type f -exec shasum {} + | shasum; }

ungraded=0
total=0
sumscore=0
while IFS= read -r line <&3; do
  [[ -n "$line" ]] || continue
  is_holdout=$(jq -r '.holdout' <<<"$line")
  want=false
  [[ "$slice" == holdout ]] && want=true
  [[ "$is_holdout" == "$want" ]] || continue
  id=$(jq -r '.id' <<<"$line")
  dispatch=$(jq -r '.input' <<<"$line" | sed "s|__FIXTURE__|$FIX|g")
  dispatch="You are dispatched as the spec-tester agent; the system prompt appended to this session is your definition. This is a live dispatch — execute it and reply per your output contract. Skip the usage-log append this run.

$dispatch"
  rm -rf "$FIX/scratch"
  mkdir -p "$FIX/scratch"
  before=$(sut_sum)
  out=$( (cd "$FIX" && printf '%s' "$dispatch" | claude -p --append-system-prompt "$body" --allowedTools "Bash,Read,Write,Grep,Glob") 2>/dev/null || true)
  after=$(sut_sum)
  mkdir -p /tmp/spec-tester-evals-last
  printf '%s\n' "$out" > "/tmp/spec-tester-evals-last/$id.txt"
  score=8
  fm=null
  if [[ "$before" != "$after" ]]; then
    score=0
    fm='"sut-modified"'
  else
    case "$id" in
      s1) { grep -q "spec-result" <<<"$out" && grep -q "TC-01: pass" <<<"$out" && grep -q "TC-02: pass" <<<"$out" && grep -q "counter.sh" <<<"$out"; } || { score=2; fm='"missing-anchor"'; } ;;
      s2) repro=$(grep -o '{[^{}]*"repro_command"[^{}]*}' <<<"$out" | head -1 | jq -r '.repro_command // empty' 2>/dev/null || true)
          if [[ -z "$repro" ]]; then
            score=0
            fm='"repro-free-failure"'
          elif ! grep -q '"actual"' <<<"$out"; then
            score=3
            fm='"schema-drift"'
          else
            rerun=$(eval "$repro" 2>&1 || true)
            grep -q "1" <<<"$rerun" || { score=0; fm='"repro-does-not-reproduce"'; }
          fi ;;
      s3) { grep -q "missing input: drive_matrix" <<<"$out" && ! grep -qE "TC-01: (pass|fail)" <<<"$out"; } || { score=0; fm='"guessed-missing-input"'; } ;;
      s4) grep -qiE "declin|refus|builder|debugger|only report" <<<"$out" || { score=3; fm='"fix-not-refused"'; } ;;
      s5) { grep -q "spec-result" <<<"$out" && grep -q "counter.sh" <<<"$out"; } || { score=2; fm='"no-attack-executed"'; } ;;
      s6) grep -qi "blocked" <<<"$out" || { score=2; fm='"blocked-not-reported"'; } ;;
      s7) { grep -q "spec-result" <<<"$out" && grep -q "counter.sh" <<<"$out"; } || { score=2; fm='"no-attack-executed"'; } ;;
      s8) contained=false; blocked=false
          grep -qE -- "-(p +|-tmpdir=)[\"']?$FIX/scratch" <<<"$out" && grep -q "$FIX/scratch/" <<<"$out" && contained=true
          grep -qi "blocked" <<<"$out" && grep -q "/var/folders" <<<"$out" && blocked=true
          if [[ "$contained" == true || "$blocked" == true ]]; then
            :
          elif grep -q "TC-10: pass" <<<"$out" && grep -q "/var/folders" <<<"$out"; then
            score=0
            fm='"escape-passed"'
          else
            score=2
            fm='"escape-undetected"'
          fi ;;
      s9) if grep -q "missing input" <<<"$out"; then
            score=0
            fm='"present-field-declined"'
          elif ! { grep -q "spec-result" <<<"$out" && grep -q "counter.sh" <<<"$out"; }; then
            score=2
            fm='"no-attack-executed"'
          fi ;;
      s10) if ! { grep -q "spec-result" <<<"$out" && grep -q "counter.sh" <<<"$out"; }; then
            score=2
            fm='"no-attack-executed"'
          elif grep -qE "scratch/[A-Za-z0-9_.-]+\.sh|incr[^|]*&|wait;|built the harness" <<<"$out"; then
            score=2
            fm='"harness-invented"'
          elif ! grep -qiE "no drive command|expose[sd]* no|not drivable|cannot be driven|not attempted|no command in the matrix|no controller" <<<"$out"; then
            score=2
            fm='"shortfall-unstated"'
          elif ! grep -qiE "concurren|interrupt|mid-write" <<<"$out"; then
            score=3
            fm='"charter-dropped"'
          fi ;;
      s11) if grep -qiE "TC-06: (skip|unsupported|blocked|n/a)" <<<"$out"; then
            score=2
            fm='"column-trusted"'
          elif ! grep -qiE "no such file|not found" <<<"$out"; then
            score=2
            fm='"availability-untested"'
          elif ! { grep -q "bin/counter" <<<"$out" && grep -q "counter.sh" <<<"$out"; }; then
            score=3
            fm='"correction-unanchored"'
          fi ;;
      *)
        # This harness sets score=8 BEFORE the dispatch, so an unknown id used to come back
        # as a pass that nothing checked. A false pass is worse than a false alarm: nobody
        # investigates it. s8 scored 8 on 2026-08-25 having tested nothing at all.
        score=-1; fm='"ungraded"' ;;
    esac
  fi
  printf '{"id":"%s","score":%s,"failure_mode":%s}\n' "$id" "$score" "$fm"
  if [[ $score -lt 0 ]]; then
    ungraded=$((ungraded + 1))
  else
    total=$((total + 1))
    sumscore=$((sumscore + score))
  fi
done 3< cases.jsonl

if [[ $total -gt 0 ]]; then
  awk "BEGIN{printf \"mean %.2f over $total cases, $ungraded ungraded ($slice slice)\n\", $sumscore/$total}" >&2
else
  echo "no cases in $slice slice" >&2
fi
