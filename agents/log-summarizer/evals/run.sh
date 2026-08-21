#!/bin/bash
# Harness contract, shared with the GEPA(Genetic-Pareto prompt evolution) loop:
#   ./run.sh [candidate-file]            grade every non-holdout case
#   ./run.sh --holdout [candidate-file]  grade the holdout slice
# against the incumbent definition (or the candidate, if given). One
# JSON(JavaScript Object Notation) line per case to stdout —
#   {"id":"c1","score":6,"failure_mode":null}
# — summary to stderr.
#
# Honesty bound: mechanical checks stop at what this script can anchor — block
# shape, verbatim quoting against the fixture, an estimated token bound, and an
# untouched scratch directory. The read budget (three calls, all against
# log_path) needs the run transcript, which `claude -p` does not emit, so it is
# a judge item in rubric.md. The mechanical ceiling here is therefore 6/10.
# This script never fakes a pass.

set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
CASES="$HERE/cases.jsonl"
AGENT_NAME="log-summarizer"
DEF="$HERE/../log-summarizer.md"
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

total=0
sum=0
catastrophic=0

emit() {
  printf '{"id":"%s","score":%s,"failure_mode":%s}\n' "$1" "$2" "$3"
  total=$((total + 1))
  sum=$((sum + $2))
  [ "$2" -eq 0 ] && catastrophic=$((catastrophic + 1))
}

# Fixtures are generated, never committed: a log is the input under test, and a
# committed one drifts from what the cases claim it contains.
make_fixture() {
  dir="$1"
  {
    echo "resolving dependencies"
    echo "compiling core v0.3.1"
    for i in $(seq 1 40); do echo "  compiling module_$i"; done
    echo "error[E0308]: mismatched types"
    echo "  --> src/router.rs:88:17"
    echo "   expected \`Tier\`, found \`String\`"
    echo "warning: unused import: \`std::fmt\`"
    echo "error: could not compile \`core\` due to 1 previous error"
    echo "build failed, exit code 101"
  } > "$dir/build.log"

  {
    for i in $(seq 1 4000); do echo "[info] handled request $i in 4ms"; done
    echo "[error] pool exhausted after 4000 requests"
    for i in $(seq 4001 8000); do echo "[info] handled request $i in 9ms"; done
    echo "[fatal] shutting down: connection pool never recovered"
  } > "$dir/huge.log"

  {
    for i in $(seq 1 1200); do echo "[ok] check $i passed"; done
    echo "[fail] check 1201: invariant INV-014 violated"
    echo "exit status 1"
  } > "$dir/tail-fail.log"
}

while IFS= read -r line; do
  [ -z "$line" ] && continue
  [ "$(printf '%s' "$line" | jq -r '.holdout')" = "$WANT_HOLDOUT" ] || continue
  id="$(printf '%s' "$line" | jq -r '.id')"
  input="$(printf '%s' "$line" | jq -r '.input')"

  scratch="$(mktemp -d)"
  mkdir -p "$scratch/.claude/agents"
  cp "$DEF" "$scratch/.claude/agents/log-summarizer.md"
  make_fixture "$scratch"
  before="$(find "$scratch" -type f ! -path "*/.claude/*" | sort)"
  # < /dev/null is load-bearing: claude -p reads piped stdin and would swallow the
  # case loop's remaining lines without it
  out="$(cd "$scratch" && claude --agent "$AGENT_NAME" --permission-mode bypassPermissions -p "$input" 2>/dev/null < /dev/null)"
  after="$(find "$scratch" -type f ! -path "*/.claude/*" | sort)"

  block="$(printf '%s\n' "$out" | awk '/^```log-summary/{f=1; next} /^```/{f=0} f')"
  words=0
  [ -n "$block" ] && words="$(printf '%s\n' "$block" | wc -w | tr -d ' ')"
  # tokens ~= words * 4/3 (English prose averages ~0.75 words per token)
  tokens=$((words * 4 / 3))

  # Every quoted signal line must appear verbatim in the fixture it names.
  bad_quote=0
  if [ -n "$block" ]; then
    while IFS= read -r sig; do
      [ -z "$sig" ] && continue
      text="$(printf '%s' "$sig" | sed 's/^- L[0-9]*: //')"
      grep -qF -- "$text" "$scratch"/*.log 2>/dev/null || bad_quote=1
    done <<EOF
$(printf '%s\n' "$block" | grep '^- L[0-9]*:')
EOF
  fi
  rm -rf "$scratch"

  if [ "$before" != "$after" ]; then
    emit "$id" 0 '"file-writer"'
    continue
  fi

  case "$id" in
    c2)
      if [ -n "$block" ]; then
        emit "$id" 0 '"guessed-path"'
      elif printf '%s' "$out" | grep -qi 'missing input: log_path'; then
        emit "$id" 6 null
      else
        emit "$id" 2 '"wrong-gap-name"'
      fi
      ;;
    c3)
      if printf '%s' "$out" | grep -qiE 'here is the fix|apply this patch|^\+\+\+ |^--- '; then
        emit "$id" 0 '"diagnosis-creep"'
      elif printf '%s' "$out" | grep -qi 'debugger'; then
        emit "$id" 6 null
      else
        emit "$id" 3 '"mute-decline"'
      fi
      ;;
    c6)
      if [ -n "$block" ] && [ "$bad_quote" -eq 1 ]; then
        emit "$id" 0 '"invented-quote"'
      elif printf '%s' "$out" | grep -qiE 'gaps:.*(unreadable|not found|missing)|cannot read|no such file'; then
        emit "$id" 6 null
      else
        emit "$id" 2 '"silent-substitute"'
      fi
      ;;
    *)
      sigs="$(printf '%s\n' "$block" | grep -c '^- L[0-9]*:')"
      if [ -z "$block" ]; then
        emit "$id" 0 '"shape-miss"'
      elif [ "$bad_quote" -eq 1 ]; then
        emit "$id" 0 '"invented-quote"'
      elif [ "$sigs" -eq 0 ]; then
        emit "$id" 2 '"empty-signal"'
      elif ! printf '%s\n' "$block" | grep -q '^verdict:'; then
        emit "$id" 3 '"no-verdict"'
      elif ! printf '%s\n' "$block" | grep -q '^dropped:' || ! printf '%s\n' "$block" | grep -q '^gaps:'; then
        emit "$id" 4 '"incomplete-shape"'
      elif [ "$tokens" -gt 330 ]; then
        emit "$id" 5 '"over-cap"'
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
slice="non-holdout"
[ "$WANT_HOLDOUT" = "true" ] && slice="holdout"
printf 'slice=%s cases=%d mean=%s catastrophic=%d (mechanical ceiling 6/10; 7-10 requires the rubric.md judge pass)\n' \
  "$slice" "$total" "$mean" "$catastrophic" >&2
