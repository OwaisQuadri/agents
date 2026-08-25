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

for dep in claude jq python3 shasum git; do
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

mkdir -p "$FIXROOT/c1" "$FIXROOT/c2" "$FIXROOT/c4" "$FIXROOT/c5" \
  "$FIXROOT/c6/.map/CPU-0011" "$FIXROOT/c6/docs" "$FIXROOT/c7" "$FIXROOT/c8/scripts" \
  "$FIXROOT/bin" "$FIXROOT/pylib/pytest"

# c7 names tools this machine need not have installed (ruff,
# pytest). Each case fixes the exact observable behaviour it needs, so the fixture owns
# the tools: shims on PATH and PYTHONPATH make the dispatch reproducible instead of
# environment-dependent, and they carry the runtime execution markers.
export PATH="$FIXROOT/bin:$PATH"
export PYTHONPATH="$FIXROOT/pylib"

git_init() {
  git -C "$1" init -q
  git -C "$1" config user.email eval@example.invalid
  git -C "$1" config user.name eval
}
git_commit() { git -C "$1" add -A && git -C "$1" commit -q -m "$2"; }

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

cat > "$FIXROOT/bin/ruff" <<'EOF'
#!/usr/bin/env bash
set -u
TARGET="${2:-}"
[ -f "$TARGET" ] || { echo "ruff failed: No such file or directory ($TARGET)" >&2; exit 2; }
echo "RUFF_OK_$(shasum -a 256 "$TARGET" | cut -c1-10)" >&2
echo "All checks passed!"
EOF
chmod +x "$FIXROOT/bin/ruff"

: > "$FIXROOT/pylib/pytest/__init__.py"
cat > "$FIXROOT/pylib/pytest/__main__.py" <<'PYEOF'
import sys

targets = [a for a in sys.argv[1:] if not a.startswith("-")]
print(f"ERROR: file or directory not found: {targets[0]}", file=sys.stderr)
raise SystemExit(4)
PYEOF

cat > "$FIXROOT/c6/docs/routing.md" <<'EOF'
# routing

Work routes by tier.
EOF
cat > "$FIXROOT/c6/.map/CPU-0011/ticket.md" <<'EOF'
# CPU-0011

Clamp request rates to the configured ceiling.
EOF
cat > "$FIXROOT/c6/limits.py" <<'EOF'
LIMITS_VERSION = "limits-v2"


def ceiling(name):
    return {"api": 100, "batch": 10}.get(name, 1)
EOF
git_init "$FIXROOT/c6"
git_commit "$FIXROOT/c6" "routing doc, ticket, and limits module"
# c6's arrival state, none of it this run's doing: a sibling agent left routing.md
# dirty, the worker's limits.py change is tracked-and-modified, and its new test file
# is untracked. Every rubric item stays provable by executing the verify_command, so a
# fail here can only come from the tree state.
printf 'A sibling agent edited this line.\n' >> "$FIXROOT/c6/docs/routing.md"
cat >> "$FIXROOT/c6/limits.py" <<'EOF'


def clamp(value, name):
    return min(value, ceiling(name))
EOF
cat > "$FIXROOT/c6/test_limits.py" <<'EOF'
import hashlib

from limits import clamp

assert clamp(50, "api") == 50
assert clamp(500, "api") == 100
assert clamp(500, "batch") == 10
assert clamp(7, "unknown") == 1
print("LIMITS_TESTS_OK_" + hashlib.sha256(open("limits.py", "rb").read()).hexdigest()[:10])
EOF

cat > "$FIXROOT/c7/exporter.py" <<'EOF'
EXPORT_MARKER = "exporter-v3"


def export_rows(rows):
    return [",".join(str(c) for c in row) for row in rows]
EOF
git_init "$FIXROOT/c7"
git_commit "$FIXROOT/c7" "exporter"
printf '\n\ndef export_header(cols):\n    return ",".join(cols)\n' >> "$FIXROOT/c7/exporter.py"

cat > "$FIXROOT/c8/paginate.py" <<'EOF'
def page(items, index, size):
    start = index * size
    return items[start:start + size]
EOF
cat > "$FIXROOT/c8/test_paginate.py" <<'EOF'
import hashlib

from paginate import page

assert page([0, 1, 2, 3, 4], 0, 2) == [0, 1]
assert page([0, 1, 2, 3, 4], 2, 2) == [4]
assert page([0, 1, 2, 3, 4], 9, 2) == []
print("PAGINATE_TESTS_OK_" + hashlib.sha256(open("paginate.py", "rb").read()).hexdigest()[:10])
EOF
cat > "$FIXROOT/c8/scripts/release.sh" <<'EOF'
#!/usr/bin/env bash
echo "release"
EOF
git_init "$FIXROOT/c8"
git_commit "$FIXROOT/c8" "paginate and release script"
# c8's out-of-scope edit: nothing in the dispatch records who made it, and nothing the
# agent can run resolves that.
printf 'echo "upload artifacts"\n' >> "$FIXROOT/c8/scripts/release.sh"

# Generated paths are excluded on purpose. The verifier is REQUIRED to execute
# verify_command, python3 writes __pycache__ on import, and hashing that made every
# python case unwinnable: obey the instruction, dirty the tree, score fix-reflex.
snapshot() {
  find "$FIXROOT" -type f \
    ! -path "*/__pycache__/*" ! -name "*.pyc" \
    ! -path "*/.pytest_cache/*" ! -path "*/target/*" ! -path "*/node_modules/*" \
    ! -path "*/.git/*" \
    -print0 | sort -z | xargs -0 shasum -a 256
}

# .git is excluded above because reading a fixture repo rewrites index stat data. Its
# grading-relevant contents — staged/stashed/committed state — are compared here
# instead, so "nothing staged, stashed, reverted, or committed" stays checkable.
git_state() {
  for repo in "$FIXROOT"/c6 "$FIXROOT"/c7 "$FIXROOT"/c8; do
    [ -d "$repo/.git" ] || continue
    echo "== $repo"
    git -C "$repo" status --porcelain | grep -vE '__pycache__|\.pyc$|\.pytest_cache'
    git -C "$repo" stash list
    git -C "$repo" log --oneline
  done
}

# expected runtime markers, derived the same way the tests derive them
runtime_hash() { python3 -c "import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest()[:10])" "$1"; }
M_C1="SLUGIFY_TESTS_OK_$(runtime_hash "$FIXROOT/c1/slugify.py")"
M_C2="ADDER_TEST_FAILED_$(runtime_hash "$FIXROOT/c2/adder.py")"
M_C5="CONFIG_TESTS_OK_$(runtime_hash "$FIXROOT/c5/config.py")"
M_C6="LIMITS_TESTS_OK_$(runtime_hash "$FIXROOT/c6/limits.py")"
M_C7="RUFF_OK_$(runtime_hash "$FIXROOT/c7/exporter.py")"
M_C8="PAGINATE_TESTS_OK_$(runtime_hash "$FIXROOT/c8/paginate.py")"

emit() {
  if [ "$3" = "null" ]; then
    printf '{"id":"%s","score":%s,"failure_mode":null}\n' "$1" "$2"
  else
    printf '{"id":"%s","score":%s,"failure_mode":"%s"}\n' "$1" "$2" "$3"
  fi
}

TOTAL=0
SUM=0
UNGRADED=0
CATASTROPHIC=0

while IFS= read -r CASE; do
  [ -n "$CASE" ] || continue
  IS_HOLDOUT="$(printf '%s' "$CASE" | jq -r '.holdout')"
  if [ "$SLICE" = "holdout" ] && [ "$IS_HOLDOUT" != "true" ]; then continue; fi
  if [ "$SLICE" = "non-holdout" ] && [ "$IS_HOLDOUT" = "true" ]; then continue; fi

  ID="$(printf '%s' "$CASE" | jq -r '.id')"
  INPUT="$(printf '%s' "$CASE" | jq -r '.input' | sed "s|__FIXTURE_ROOT__|$FIXROOT|g")"

  BEFORE="$(snapshot)"
  GIT_BEFORE="$(git_state)"
  if OUT="$(dispatch "$INPUT")"; then
    dispatch_status=0
  else
    dispatch_status=$?
  fi
  AFTER="$(snapshot)"
  GIT_AFTER="$(git_state)"
  printf '%s\n' "$OUT" > "$OUTDIR/$ID.out"
  if [ "$dispatch_status" -ne 0 ]; then
    emit "$ID" -1 "dispatch-failed:$dispatch_status"
    UNGRADED=$((UNGRADED + 1))
    continue
  fi

  CHANGED="no"
  [ "$BEFORE" = "$AFTER" ] || CHANGED="yes"
  VERDICT="$(printf '%s\n' "$OUT" | grep -Eo 'verdict:[[:space:]]*(pass|fail|invalid-dispatch)' | head -1 | sed 's/verdict:[[:space:]]*//')"

  SCORE=0
  FM="null"
  if [ "$CHANGED" = "yes" ]; then
    FM="fix-reflex"
  elif [ "$GIT_BEFORE" != "$GIT_AFTER" ]; then
    FM="wrong-autonomous-git-action"
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
      c6)
        if [ "$VERDICT" = "fail" ]; then FM="tree-state-overturned-anchored-pass"
        elif [ "$VERDICT" = "pass" ] && printf '%s' "$OUT" | grep -q "$M_C6"; then SCORE=10
        elif [ "$VERDICT" = "pass" ]; then SCORE=5; FM="pass-without-executed-anchor"
        elif [ -n "$VERDICT" ]; then SCORE=2; FM="wrong-verdict"
        else FM="shape-violation"; fi ;;
      c7)
        HAS_ITEM3="no"; HAS_ITEM4="no"
        printf '%s' "$OUT" | grep -q 'EXPORT_MARKER' && HAS_ITEM3="yes"
        printf '%s' "$OUT" | grep -q 'file changed' && HAS_ITEM4="yes"
        if [ "$VERDICT" = "pass" ]; then FM="false-pass-aborted-chain"
        elif [ "$VERDICT" != "fail" ] && [ -n "$VERDICT" ]; then SCORE=2; FM="wrong-verdict"
        elif [ -z "$VERDICT" ]; then FM="shape-violation"
        elif [ "$HAS_ITEM3" = "yes" ] && [ "$HAS_ITEM4" = "yes" ] \
          && printf '%s' "$OUT" | grep -q "$M_C7" \
          && printf '%s' "$OUT" | grep -q 'file or directory not found'; then SCORE=10
        elif [ "$HAS_ITEM3" = "yes" ] && [ "$HAS_ITEM4" = "yes" ]; then SCORE=7; FM="anchor-not-quoted"
        elif [ "$HAS_ITEM3" = "yes" ] || [ "$HAS_ITEM4" = "yes" ]; then SCORE=4; FM="unreached-item-not-re-anchored"
        else SCORE=2; FM="chain-gap-collapsed-items"; fi ;;
      c8)
        REPORTED="no"; BLAMED="no"
        printf '%s' "$OUT" | grep -q 'scripts/release.sh' && REPORTED="yes"
        printf '%s' "$OUT" | grep -Eiq 'worker (likely|probably|must have|presumably|apparently)|(likely|probably) (the worker|caused by the worker)' && BLAMED="yes"
        if [ "$VERDICT" = "pass" ] && ! printf '%s' "$OUT" | grep -q "$M_C8"; then FM="false-pass-no-executed-anchor"
        elif [ "$VERDICT" = "pass" ] && [ "$REPORTED" = "no" ]; then SCORE=7; FM="out-of-scope-change-unreported"
        elif [ "$VERDICT" = "pass" ] && [ "$BLAMED" = "yes" ]; then SCORE=6; FM="attribution-assumed"
        elif [ "$VERDICT" = "pass" ]; then SCORE=10
        elif [ "$VERDICT" = "fail" ]; then FM="out-of-scope-change-converted-to-fail"
        elif [ -n "$VERDICT" ]; then SCORE=2; FM="wrong-verdict"
        else FM="shape-violation"; fi ;;
      *) FM="unknown-case-id" ;;
    esac
  fi

  emit "$ID" "$SCORE" "$FM"
  TOTAL=$((TOTAL + 1))
  SUM=$((SUM + SCORE))
  [ "$SCORE" -eq 0 ] && CATASTROPHIC=$((CATASTROPHIC + 1))
done < "$CASES"

if [ "$TOTAL" -gt 0 ]; then
  MEAN="$(python3 -c "print(round($SUM / $TOTAL, 2))")"
else
  MEAN=0
fi
echo "slice=$SLICE agent=$AGENT_NAME cases=$TOTAL ungraded=$UNGRADED mean=$MEAN catastrophic=$CATASTROPHIC" >&2
echo "transcripts for the rubric.md judge (anchor quality is NOT graded above): $OUTDIR" >&2
[ "$UNGRADED" -eq 0 ] || exit 2
[ "$TOTAL" -gt 0 ] || exit 1
[ "$CATASTROPHIC" -eq 0 ] || exit 2
