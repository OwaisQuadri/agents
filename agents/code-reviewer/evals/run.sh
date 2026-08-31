#!/usr/bin/env bash
# Convention (skills/ai-author/templates/eval-harness.md): ./run.sh [candidate-file]
# runs every non-holdout case against the incumbent definition (or the candidate,
# staged into a throwaway project and dispatched headlessly via
# `claude --agent code-reviewer -p`); --holdout runs the holdout slice. One
# JSON(JavaScript Object Notation) line per case to stdout:
# {"id":"c1","score":8,"failure_mode":"<tag-or-null>"} and a summary to stderr.
#
# Grading honesty: everything below is mechanical — output shape, seeded-defect
# anchors by name, and fixture integrity (zero modifications). It CANNOT verify that
# a proof command truly demonstrates the defect; that anchor pass needs a human or a
# fresh LLM(large language model) judge grading against rubric.md, so mechanically
# passing review cases cap at 8 and the 9-10 band stays judge-only. Nothing here
# fakes a pass.
#
# Setup and grading branches are keyed to the seed case ids in cases.jsonl; a case
# grown from logs or votes needs its branch added here.

set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
CASES="$HERE/cases.jsonl"
DEF="$HERE/../code-reviewer.md"
SLICE="non-holdout"

for arg in "$@"; do
  case "$arg" in
    --holdout) SLICE="holdout" ;;
    *) DEF="$arg" ;;
  esac
done

command -v claude >/dev/null 2>&1 || { echo "claude CLI(command-line interface) not found on PATH" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 not found on PATH" >&2; exit 1; }
command -v cargo >/dev/null 2>&1 || { echo "cargo not found on PATH" >&2; exit 1; }
[ -f "$DEF" ] || { echo "agent definition not found: $DEF" >&2; exit 1; }
[ -f "$CASES" ] || { echo "cases file not found: $CASES" >&2; exit 1; }

# c6 dispatches the agent against a live dispatch-baseline binary so grading can
# see the exact --out command it chose for its baseline stamp, not just the
# markdown reply. Build once, prepend to PATH for every dispatch subshell.
TOOLDIR="$(cd "$HERE/../../../tools/dispatch-baseline" && pwd)"
cargo build --release --manifest-path "$TOOLDIR/Cargo.toml" -q \
  || { echo "failed to build dispatch-baseline" >&2; exit 1; }
TOOLBIN="$TOOLDIR/target/release"

FIXROOT="/tmp/code-reviewer-evals"
FIXTURE="$FIXROOT/fixture-repo"

G() { git -C "$FIXTURE" -c user.email=eval@local -c user.name=eval "$@"; }

build_fixture() {
  rm -rf "$FIXROOT"
  mkdir -p "$FIXROOT"
  git init -q -b main "$FIXTURE"
  cat > "$FIXTURE/app.py" <<'EOF'
def greet(name):
    return "hello " + name
EOF
  G add app.py
  G commit -qm "app: greet"

  G checkout -qb feature-auth
  cat > "$FIXTURE/login.py" <<'EOF'
import sqlite3


def find_user(db, username):
    query = "SELECT * FROM users WHERE name = '%s'" % username
    return db.execute(query).fetchall()
EOF
  G add login.py
  G commit -qm "auth: user lookup"

  G checkout -q main
  G checkout -qb copy-tweak
  cat > "$FIXTURE/app.py" <<'EOF'
def greet(name):
    return "hello, " + name
EOF
  G commit -qam "copy: comma in greeting"
  G checkout -q main
}

setup_case() {
  build_fixture
  if [ "$1" = "c5" ]; then
    cat >> "$FIXTURE/app.py" <<'EOF'


def average(items):
    return sum(items) / len(items)
EOF
  fi
}

fixture_state() {
  G status --porcelain
  G for-each-ref
  G rev-parse HEAD
  G symbolic-ref -q HEAD || true
  shasum "$FIXTURE"/*.py 2>/dev/null
}

WORKDIR="$(mktemp -d)"
mkdir -p "$WORKDIR/.claude/agents"
cp "$DEF" "$WORKDIR/.claude/agents/code-reviewer.md"
trap 'rm -rf "$WORKDIR"' EXIT

dispatch() {
  # < /dev/null is load-bearing: claude -p reads piped stdin and would swallow the
  # case loop's remaining lines without it
  ( cd "$WORKDIR" && PATH="$TOOLBIN:$PATH" claude --agent code-reviewer -p "$1" --allowedTools "Read,Grep,Glob,Bash" 2>/dev/null < /dev/null )
}

dispatch_json() {
  # same dispatch, but stream-json + verbose so the raw tool-call transcript
  # (including the exact Bash command the agent ran for dispatch-baseline
  # stamp) survives past the final markdown reply
  ( cd "$WORKDIR" && PATH="$TOOLBIN:$PATH" claude --agent code-reviewer -p "$1" --allowedTools "Read,Grep,Glob,Bash" --output-format stream-json --verbose 2>/dev/null < /dev/null )
}

extract_stream() {
  # $1 = raw stream-json, $2 = out markdown (assistant text, same shape the
  # other cases grade), $3 = out Bash commands (one per line)
  python3 - "$1" "$2" "$3" <<'PY'
import json, sys

raw_path, text_path, cmds_path = sys.argv[1:4]
texts, cmds = [], []
with open(raw_path) as f:
    for line in f:
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        if obj.get("type") != "assistant":
            continue
        for block in obj.get("message", {}).get("content", []):
            if block.get("type") == "text":
                texts.append(block["text"])
            elif block.get("type") == "tool_use" and block.get("name") == "Bash":
                cmds.append(block.get("input", {}).get("command", "").replace("\n", " "))
open(text_path, "w").write("\n".join(texts) + "\n")
open(cmds_path, "w").write("\n".join(cmds) + "\n")
PY
}

section() {
  awk -v h="## $1" '$0==h{f=1;next} /^## /{f=0} f' "$2"
}

section_is_none() {
  local body
  body="$(section "$1" "$2" | grep -v '^[[:space:]]*$' || true)"
  [ -z "$body" ] && return 0
  printf '%s' "$body" | grep -qiE '^[-* ]*none'
}

grade_case() {
  local id="$1" out="$2"
  SCORE=1
  FM='"shape-violation"'
  case "$id" in
    c1)
      grep -q '^status:[[:space:]]*reviewed' "$out" \
        && grep -q '^range:.*git' "$out" \
        && grep -q '^## Critical' "$out" \
        && grep -q '^## Warnings' "$out" \
        && grep -q '^## Suggestions' "$out" || return 0
      if section_is_none Critical "$out"; then
        SCORE=0; FM='"false-pass"'; return 0
      fi
      if section Critical "$out" | grep -q 'login\.py' \
        && section Critical "$out" | grep -q 'proof:'; then
        SCORE=8; FM=null
      else
        SCORE=2; FM='"missed-seeded-defect"'
      fi
      ;;
    c2)
      if grep -qi 'invalid-dispatch' "$out"; then
        if grep -qE '^## (Critical|Warnings|Suggestions)' "$out"; then
          SCORE=0; FM='"hedged-invalid-dispatch"'
        elif grep -qi 'repo_path' "$out"; then SCORE=8; FM=null; else SCORE=4; FM='"gap-not-named"'; fi
      elif grep -q '^## Critical' "$out"; then
        SCORE=0; FM='"guessed-missing-input"'
      fi
      ;;
    c3)
      if grep -qi 'invalid-dispatch' "$out"; then
        if grep -qE '^## (Critical|Warnings|Suggestions)' "$out"; then
          SCORE=0; FM='"hedged-invalid-dispatch"'
        elif grep -qiE 'trigger|declin' "$out"; then SCORE=8; FM=null; else SCORE=4; FM='"gap-not-named"'; fi
      elif grep -q '^## Critical' "$out"; then
        SCORE=3; FM='"improvised-review"'
      fi
      ;;
    c4)
      grep -q '^status:[[:space:]]*reviewed' "$out" \
        && grep -q '^## Critical' "$out" || return 0
      if section_is_none Critical "$out"; then
        SCORE=8; FM=null
      else
        SCORE=2; FM='"severity-inflation"'
      fi
      ;;
    c5)
      grep -q '^status:[[:space:]]*reviewed' "$out" \
        && grep -q '^range:.*git' "$out" || return 0
      if section_is_none Critical "$out" && section_is_none Warnings "$out"; then
        SCORE=0; FM='"false-pass"'; return 0
      fi
      if { section Critical "$out"; section Warnings "$out"; } | grep -qE 'app\.py|average'; then
        SCORE=8; FM=null
      else
        SCORE=2; FM='"missed-seeded-defect"'
      fi
      ;;
    c6)
      local cmds="$WORKDIR/cmds-$id.txt" stamp_calls stamp_line
      [ -f "$cmds" ] || { SCORE=0; FM='"no-transcript"'; return 0; }
      stamp_calls="$(grep -c 'dispatch-baseline stamp' "$cmds" || true)"
      if [ "$stamp_calls" -eq 0 ]; then
        SCORE=0; FM='"baseline-not-stamped"'; return 0
      fi
      if [ "$stamp_calls" -gt 1 ]; then
        SCORE=2; FM='"baseline-stamp-retry"'; return 0
      fi
      stamp_line="$(grep 'dispatch-baseline stamp' "$cmds")"
      if printf '%s' "$stamp_line" | grep -qE 'mktemp[^-]*/[^ ]+\.X{4,}' \
        && ! printf '%s' "$stamp_line" | grep -q 'mktemp -u'; then
        SCORE=2; FM='"fragile-mktemp-form"'; return 0
      fi
      grep -q '^status:[[:space:]]*reviewed' "$out" \
        && grep -q '^range:.*git' "$out" \
        && grep -q '^## Critical' "$out" \
        && grep -q '^## Warnings' "$out" \
        && grep -q '^## Suggestions' "$out" || return 0
      if section_is_none Critical "$out"; then
        SCORE=0; FM='"false-pass"'; return 0
      fi
      if section Critical "$out" | grep -q 'login\.py' \
        && section Critical "$out" | grep -q 'proof:'; then
        SCORE=8; FM=null
      else
        SCORE=2; FM='"missed-seeded-defect"'
      fi
      ;;
    *)
      SCORE=0; FM='"ungraded-case"'
      ;;
  esac
  return 0
}

TOTAL=0
N=0
CATS=0

while IFS=$'\t' read -r id input; do
  [ -z "$id" ] && continue
  setup_case "$id"
  PRE="$(fixture_state)"
  OUTFILE="$WORKDIR/out-$id.txt"
  if [ "$id" = "c6" ]; then
    RAWFILE="$WORKDIR/raw-$id.jsonl"
    dispatch_json "$input" > "$RAWFILE"
    extract_stream "$RAWFILE" "$OUTFILE" "$WORKDIR/cmds-$id.txt"
  else
    dispatch "$input" > "$OUTFILE"
  fi
  POST="$(fixture_state)"
  if [ "$PRE" != "$POST" ]; then
    SCORE=0
    FM='"modified-files"'
  else
    grade_case "$id" "$OUTFILE"
  fi
  printf '{"id":"%s","score":%s,"failure_mode":%s}\n' "$id" "$SCORE" "$FM"
  N=$((N + 1))
  TOTAL=$((TOTAL + SCORE))
  if [ "$SCORE" -eq 0 ]; then CATS=$((CATS + 1)); fi
done < <(python3 - "$CASES" "$SLICE" <<'PY'
import json
import sys

path, slice_ = sys.argv[1], sys.argv[2]
for line in open(path):
    line = line.strip()
    if not line:
        continue
    case = json.loads(line)
    if (slice_ == "holdout") != bool(case.get("holdout", False)):
        continue
    print(case["id"] + "\t" + case["input"].replace("\t", " ").replace("\n", " "))
PY
)

if [ "$N" -gt 0 ]; then
  MEAN="$(awk -v t="$TOTAL" -v n="$N" 'BEGIN{printf "%.1f", t/n}')"
else
  MEAN=0
fi
echo "slice=$SLICE cases=$N mean=$MEAN catastrophic=$CATS definition=$DEF" >&2
