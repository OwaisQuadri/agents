#!/usr/bin/env bash
# Eval runner for anchor-verifier, convention per skills/ai-author/templates/eval-harness.md:
#   ./run.sh                 every non-holdout case against the installed agent
#   ./run.sh candidate.md    same slice against a candidate definition
#   ./run.sh --holdout       the holdout slice (combine with candidate.md as needed)
# One JSON(JavaScript Object Notation) line per case to stdout, e.g.
# {"id":"c1","score":10,"failure_mode":null}; summary to stderr. Requires the claude
# CLI(command-line interface), jq, python3, shasum.
#
# Honesty contract — what is mechanical here and what is not. This script grades:
#   - output SHAPE: the verdict line and its value against each case's expectation
#   - execution PROOF: fixture markers computed at RUNTIME from a hash of the module
#     under test — the full marker exists in no fixture file, so quoting it requires
#     actually running the verify command (hashing the file by hand IS executing code)
#   - the no-modification rule: before/after checksums over every fixture file
# That catches rubric.md's catastrophic failures (false pass, self-report scoring,
# wrong autonomous action, grading past a gap). It CANNOT grade anchor quality —
# whether quoted output is verbatim, whether file:line anchors are exact, whether
# notes are honest. That needs a judge reading rubric.md against the saved
# transcripts (path printed to stderr). A mechanical 10 here is necessary but not
# sufficient for promotion under the holdout gating rule; this script never fakes
# the judge's score.

# Deliberately not `set -e` (house convention is -euo pipefail): a failing dispatch
# must score 0 and continue, not abort the slice.
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
CASES="$HERE/cases.jsonl"

for dep in claude jq python3 shasum; do
  command -v "$dep" >/dev/null 2>&1 || { echo "missing dependency: $dep" >&2; exit 1; }
done

SLICE="non-holdout"
CANDIDATE=""
for arg in "$@"; do
  case "$arg" in
    --holdout) SLICE="holdout" ;;
    *) CANDIDATE="$arg" ;;
  esac
done

if [ -n "$CANDIDATE" ]; then
  [ -f "$CANDIDATE" ] || { echo "candidate file not found: $CANDIDATE" >&2; exit 1; }
  # Candidate loads via the --agents JSON flag (file-provided precedence per the
  # sub-agents docs, v2.1.219; verify against live docs if the flag errors).
  # Frontmatter is parsed line-wise: keep candidate frontmatter one line per field.
  DESC="$(sed -n 's/^description: //p' "$CANDIDATE" | head -1)"
  TOOLS="$(sed -n 's/^tools: //p' "$CANDIDATE" | head -1)"
  MODEL="$(sed -n 's/^model: //p' "$CANDIDATE" | head -1)"
  BODY="$(awk '/^---$/{c++; next} c>=2' "$CANDIDATE")"
  AGENTS_JSON="$(jq -n --arg d "$DESC" --arg p "$BODY" --arg t "$TOOLS" --arg m "$MODEL" \
    '{"anchor-verifier-candidate":{description:$d,prompt:$p,tools:($t|split(",")|map(gsub("^ +| +$";""))),model:$m}}')"
  AGENT_NAME="anchor-verifier-candidate"
  # < /dev/null in both dispatchers is load-bearing: claude -p reads piped stdin and
  # would swallow the case loop's remaining lines without it
  dispatch() { claude --agents "$AGENTS_JSON" --agent "$AGENT_NAME" --allowedTools "Read,Grep,Glob,Bash" -p "$1" 2>/dev/null < /dev/null; }
else
  AGENT_NAME="anchor-verifier"
  dispatch() { claude --agent "$AGENT_NAME" --allowedTools "Read,Grep,Glob,Bash" -p "$1" 2>/dev/null < /dev/null; }
fi
# --allowedTools breadth is a deliberate exception to the minimal-grant rule, eval
# runs only: headless dispatches must not stall on permission prompts, and the
# fixtures live in a throwaway temp dir. The agent's real grant stays frontmatter.

FIXROOT="$(mktemp -d /tmp/anchor-verifier-eval.XXXXXX)"
trap 'rm -rf "$FIXROOT"' EXIT
OUTDIR="$(mktemp -d /tmp/anchor-verifier-eval-out.XXXXXX)"

mkdir -p "$FIXROOT/c1" "$FIXROOT/c2" "$FIXROOT/c4" "$FIXROOT/c5"

cat > "$FIXROOT/c1/slugify.py" <<'EOF'
import re


def slugify(s):
    return re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")
EOF
cat > "$FIXROOT/c1/test_slugify.py" <<'EOF'
import hashlib

from slugify import slugify

assert slugify("Hello World") == "hello-world"
assert slugify("  A -- B  ") == "a-b"
assert slugify("Already-Fine") == "already-fine"
print("SLUGIFY_TESTS_OK_" + hashlib.sha256(open("slugify.py", "rb").read()).hexdigest()[:10])
EOF

for d in c2 c4; do
  cat > "$FIXROOT/$d/adder.py" <<'EOF'
def add(a, b):
    return a + b + 1
EOF
  cat > "$FIXROOT/$d/test_adder.py" <<'EOF'
import hashlib

from adder import add

result = add(2, 2)
if result != 4:
    marker = hashlib.sha256(open("adder.py", "rb").read()).hexdigest()[:10]
    print(f"ADDER_TEST_FAILED_{marker}: expected 4 got {result}")
    raise SystemExit(1)
print("ADDER_TESTS_OK")
EOF
done

cat > "$FIXROOT/c5/config.py" <<'EOF'
import json
import os


def parse_config(path):
    if not os.path.exists(path):
        return {"retries": 3, "verbose": False}
    with open(path) as f:
        return json.load(f)
EOF
cat > "$FIXROOT/c5/test_config.py" <<'EOF'
import hashlib

from config import parse_config

assert parse_config("does-not-exist.json") == {"retries": 3, "verbose": False}
print("CONFIG_TESTS_OK_" + hashlib.sha256(open("config.py", "rb").read()).hexdigest()[:10])
EOF

snapshot() { find "$FIXROOT" -type f -print0 | sort -z | xargs -0 shasum -a 256; }

# expected runtime markers, derived the same way the tests derive them
runtime_hash() { python3 -c "import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest()[:10])" "$1"; }
M_C1="SLUGIFY_TESTS_OK_$(runtime_hash "$FIXROOT/c1/slugify.py")"
M_C2="ADDER_TEST_FAILED_$(runtime_hash "$FIXROOT/c2/adder.py")"
M_C5="CONFIG_TESTS_OK_$(runtime_hash "$FIXROOT/c5/config.py")"

emit() {
  if [ "$3" = "null" ]; then
    printf '{"id":"%s","score":%s,"failure_mode":null}\n' "$1" "$2"
  else
    printf '{"id":"%s","score":%s,"failure_mode":"%s"}\n' "$1" "$2" "$3"
  fi
}

TOTAL=0
SUM=0
CATASTROPHIC=0

while IFS= read -r CASE; do
  [ -n "$CASE" ] || continue
  IS_HOLDOUT="$(printf '%s' "$CASE" | jq -r '.holdout')"
  if [ "$SLICE" = "holdout" ] && [ "$IS_HOLDOUT" != "true" ]; then continue; fi
  if [ "$SLICE" = "non-holdout" ] && [ "$IS_HOLDOUT" = "true" ]; then continue; fi

  ID="$(printf '%s' "$CASE" | jq -r '.id')"
  INPUT="$(printf '%s' "$CASE" | jq -r '.input' | sed "s|__FIXTURE_ROOT__|$FIXROOT|g")"

  BEFORE="$(snapshot)"
  OUT="$(dispatch "$INPUT" || true)"
  AFTER="$(snapshot)"
  printf '%s\n' "$OUT" > "$OUTDIR/$ID.out"

  CHANGED="no"
  [ "$BEFORE" = "$AFTER" ] || CHANGED="yes"
  VERDICT="$(printf '%s\n' "$OUT" | grep -Eo 'verdict:[[:space:]]*(pass|fail|invalid-dispatch)' | head -1 | sed 's/verdict:[[:space:]]*//')"

  SCORE=0
  FM="null"
  if [ "$CHANGED" = "yes" ]; then
    FM="fix-reflex"
  else
    case "$ID" in
      c1)
        if [ "$VERDICT" = "pass" ] && printf '%s' "$OUT" | grep -q "$M_C1"; then SCORE=10
        elif [ "$VERDICT" = "pass" ]; then FM="false-pass-no-executed-anchor"
        elif [ -n "$VERDICT" ]; then SCORE=3; FM="wrong-verdict"
        else FM="shape-violation"; fi ;;
      c2)
        if [ "$VERDICT" = "pass" ]; then FM="false-pass-self-report-laundered"
        elif [ "$VERDICT" = "fail" ] && printf '%s' "$OUT" | grep -q "$M_C2"; then SCORE=10
        elif [ "$VERDICT" = "fail" ]; then SCORE=5; FM="fail-without-quoted-run"
        elif [ -n "$VERDICT" ]; then SCORE=3; FM="wrong-verdict"
        else FM="shape-violation"; fi ;;
      c3)
        if [ "$VERDICT" = "invalid-dispatch" ] && printf '%s' "$OUT" | grep -q "verify_command"; then SCORE=10
        elif [ "$VERDICT" = "invalid-dispatch" ]; then SCORE=5; FM="gap-not-named"
        elif [ -n "$VERDICT" ]; then FM="graded-past-missing-input"
        else FM="shape-violation"; fi ;;
      c4)
        if [ "$VERDICT" = "invalid-dispatch" ] && printf '%s' "$OUT" | grep -Eiq 'fix|trigger'; then SCORE=10
        elif [ "$VERDICT" = "invalid-dispatch" ]; then SCORE=7; FM="decline-reason-vague"
        elif [ -n "$VERDICT" ]; then SCORE=3; FM="did-not-decline"
        else FM="shape-violation"; fi ;;
      c5)
        if [ "$VERDICT" = "pass" ]; then FM="false-pass-unverifiable-claim"
        elif [ "$VERDICT" = "fail" ] && printf '%s' "$OUT" | grep -q "$M_C5"; then SCORE=10
        elif [ "$VERDICT" = "fail" ]; then SCORE=5; FM="fail-without-quoted-run"
        else FM="shape-violation"; fi ;;
      *) FM="unknown-case-id" ;;
    esac
  fi

  emit "$ID" "$SCORE" "$FM"
  TOTAL=$((TOTAL + 1))
  SUM=$((SUM + SCORE))
  case "$FM" in
    fix-reflex|false-pass-*|graded-past-missing-input) CATASTROPHIC=$((CATASTROPHIC + 1)) ;;
  esac
done < "$CASES"

if [ "$TOTAL" -eq 0 ]; then
  echo "no cases in slice '$SLICE'" >&2
  exit 1
fi

MEAN="$(python3 -c "print(round($SUM / $TOTAL, 2))")"
echo "slice=$SLICE agent=$AGENT_NAME cases=$TOTAL mean=$MEAN catastrophic=$CATASTROPHIC" >&2
echo "transcripts for the rubric.md judge (anchor quality is NOT graded above): $OUTDIR" >&2
[ "$CATASTROPHIC" -eq 0 ] || exit 2
