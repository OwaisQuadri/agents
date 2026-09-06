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
[ -f "$DEF" ] || { echo "agent definition not found: $DEF" >&2; exit 1; }
[ -f "$CASES" ] || { echo "cases file not found: $CASES" >&2; exit 1; }

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
  cat > "$FIXTURE/settings.html" <<'EOF'
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Account settings</title>
  </head>
  <body>
    <main>
      <h1>Account settings</h1>
      <p>Changes save automatically.</p>
      <button type="button">Save settings</button>
    </main>
  </body>
</html>
EOF
  cat > "$FIXTURE/README.md" <<'EOF'
# Fixture app

Run checks before sending a change.
EOF
  G add app.py settings.html README.md
  G commit -qm "app: greet and settings view"

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
  G checkout -qb docs-tweak
  printf '%s\n' '# Fixture app' '' 'Run all checks before sending a change.' > "$FIXTURE/README.md"
  G commit -qam "docs: clarify contributor check"

  G checkout -q main
  G checkout -qb copy-tweak
  cat > "$FIXTURE/settings.html" <<'EOF'
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Account settings</title>
  </head>
  <body>
    <main>
      <h1>Account settings</h1>
      <p>Changes save when you select Save.</p>
      <button type="button">Save settings</button>
    </main>
  </body>
</html>
EOF
  G commit -qam "copy: clarify settings save behavior"

  G checkout -q main
  G checkout -qb interface-refactor
  cat > "$FIXTURE/settings.html" <<'EOF'
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Account settings</title>
  </head>
  <body>
    <main class="settings-page">
      <h1>Account settings</h1>
      <p class="settings-save-note">Changes save automatically.</p>
      <button class="settings-save-button" type="button">Save settings</button>
    </main>
  </body>
</html>
EOF
  G commit -qam "refactor: add settings style hooks"
  G checkout -q main
}

setup_case() {
  build_fixture
  if [ "$1" = "c5" ]; then
    cat >> "$FIXTURE/app.py" <<'EOF'


def average(items):
    return sum(items) / len(items)
EOF
  elif [ "$1" = "c6" ]; then
    mkdir -p "$REVIEW_EVIDENCE"
    evidence_root="$(CDPATH= cd -- "$REVIEW_EVIDENCE" && pwd -P)"
    fixture_root="$(CDPATH= cd -- "$FIXTURE" && pwd -P)"
    case "$evidence_root/" in
      "$fixture_root/"*) echo "visual evidence must stay outside the fixture repository" >&2; exit 1 ;;
    esac
    python3 - "$REVIEW_EVIDENCE/before.png" "$REVIEW_EVIDENCE/after.png" <<'PY'
import binascii
import struct
import sys
import zlib

WIDTH = 640
HEIGHT = 360
FONT = {
    " ": ("00000",) * 7,
    ".": ("00000", "00000", "00000", "00000", "00000", "00110", "00110"),
    "A": ("01110", "10001", "10001", "11111", "10001", "10001", "10001"),
    "C": ("01111", "10000", "10000", "10000", "10000", "10000", "01111"),
    "E": ("11111", "10000", "10000", "11110", "10000", "10000", "11111"),
    "G": ("01111", "10000", "10000", "10111", "10001", "10001", "01111"),
    "H": ("10001", "10001", "10001", "11111", "10001", "10001", "10001"),
    "I": ("11111", "00100", "00100", "00100", "00100", "00100", "11111"),
    "L": ("10000", "10000", "10000", "10000", "10000", "10000", "11111"),
    "M": ("10001", "11011", "10101", "10101", "10001", "10001", "10001"),
    "N": ("10001", "11001", "10101", "10011", "10001", "10001", "10001"),
    "O": ("01110", "10001", "10001", "10001", "10001", "10001", "01110"),
    "R": ("11110", "10001", "10001", "11110", "10100", "10010", "10001"),
    "S": ("01111", "10000", "10000", "01110", "00001", "00001", "11110"),
    "T": ("11111", "00100", "00100", "00100", "00100", "00100", "00100"),
    "U": ("10001", "10001", "10001", "10001", "10001", "10001", "01110"),
    "V": ("10001", "10001", "10001", "10001", "10001", "01010", "00100"),
    "W": ("10001", "10001", "10001", "10101", "10101", "10101", "01010"),
    "Y": ("10001", "10001", "01010", "00100", "00100", "00100", "00100"),
}


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", binascii.crc32(kind + data) & 0xFFFFFFFF)


def image(path, helper, badge):
    pixels = bytearray((244, 246, 248) * WIDTH * HEIGHT)

    def pixel(x, y, color):
        if 0 <= x < WIDTH and 0 <= y < HEIGHT:
            offset = (y * WIDTH + x) * 3
            pixels[offset:offset + 3] = bytes(color)

    def rectangle(x, y, width, height, color):
        for row in range(y, y + height):
            for column in range(x, x + width):
                pixel(column, row, color)

    def circle(cx, cy, radius, color):
        for y in range(cy - radius, cy + radius + 1):
            for x in range(cx - radius, cx + radius + 1):
                if (x - cx) ** 2 + (y - cy) ** 2 <= radius ** 2:
                    pixel(x, y, color)

    def text(x, y, value, scale, color):
        cursor = x
        for character in value.upper():
            glyph = FONT[character]
            for row, bits in enumerate(glyph):
                for column, bit in enumerate(bits):
                    if bit == "1":
                        rectangle(cursor + column * scale, y + row * scale, scale, scale, color)
            cursor += 6 * scale

    rectangle(70, 45, 500, 270, (201, 210, 220))
    rectangle(72, 47, 496, 266, (255, 255, 255))
    text(105, 85, "Account settings", 4, (23, 33, 43))
    text(105, 155, helper, 2, (52, 73, 94))
    rectangle(105, 245, 145, 45, (23, 105, 224))
    text(137, 258, "Save", 3, (255, 255, 255))
    if badge == "A":
        circle(515, 235, 31, (23, 105, 224))
        text(503, 221, "A", 4, (255, 255, 255))
    else:
        rectangle(420, 214, 125, 47, (242, 184, 75))
        text(429, 227, "Manual", 3, (61, 43, 0))

    rows = b"".join(b"\x00" + bytes(pixels[y * WIDTH * 3:(y + 1) * WIDTH * 3]) for y in range(HEIGHT))
    data = b"\x89PNG\r\n\x1a\n"
    data += chunk(b"IHDR", struct.pack(">IIBBBBB", WIDTH, HEIGHT, 8, 2, 0, 0, 0))
    data += chunk(b"IDAT", zlib.compress(rows, 9))
    data += chunk(b"IEND", b"")
    with open(path, "wb") as output:
        output.write(data)
    if not data.startswith(b"\x89PNG\r\n\x1a\n") or zlib.decompress(rows and zlib.compress(rows, 9)) != rows:
        raise SystemExit("invalid PNG")


image(sys.argv[1], "Changes save automatically.", "A")
image(sys.argv[2], "Changes save when you select Save.", "Manual")
PY
    printf '%s\n' \
      "{\"path\":\"$REVIEW_EVIDENCE/before.png\",\"media_type\":\"image\",\"label\":\"Settings before copy change\",\"alt\":\"Account settings interface before the copy change.\"}" \
      "{\"path\":\"$REVIEW_EVIDENCE/after.png\",\"media_type\":\"image\",\"label\":\"Settings after copy change\",\"alt\":\"Account settings interface after the copy change.\"}" \
      > "$REVIEW_EVIDENCE/manifest.jsonl"
  fi
}

fixture_state() {
  local id="${1:-}"
  G status --porcelain
  G for-each-ref
  G rev-parse HEAD
  G symbolic-ref -q HEAD || true
  find "$FIXTURE" -path "$FIXTURE/.git" -prune -o -type f -exec shasum {} + 2>/dev/null | sort
  if [ "$id" = "c6" ]; then
    find "$REVIEW_EVIDENCE" -type f -exec shasum {} + 2>/dev/null | sort
  fi
}

WORKDIR="$(mktemp -d)"
REVIEW_EVIDENCE="$WORKDIR/evidence/c6"
mkdir -p "$WORKDIR/.claude/agents"
cp "$DEF" "$WORKDIR/.claude/agents/code-reviewer.md"
trap 'rm -rf "$WORKDIR"' EXIT

dispatch() {
  # < /dev/null is load-bearing: claude -p reads piped stdin and would swallow the
  # case loop's remaining lines without it
  ( cd "$WORKDIR" && claude --agent code-reviewer -p "$1" --allowedTools "Read,Grep,Glob,Bash" 2>/dev/null < /dev/null )
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

is_path_cited() {
  local logical_path="$1"
  local text="$2"
  local physical_path
  physical_path="$(CDPATH= cd -- "$(dirname "$logical_path")" && pwd -P)/$(basename "$logical_path")"
  printf '%s\n' "$text" | grep -Fq "$logical_path" \
    || printf '%s\n' "$text" | grep -Fq "$physical_path"
}

visual_evidence_block() {
  awk '
    {
      lower = tolower($0)
      if (mode == "") {
        if (lower ~ /^[[:space:]]*"?visual_evidence"?[[:space:]]*:/) {
          mode = "field"
          field_indent = match($0, /[^[:space:]]/) - 1
          print
        } else if (lower ~ /^##[[:space:]]+visual[_ -]?evidence[[:space:]]*$/) {
          mode = "heading"
          print
        }
        next
      }
      if (mode == "heading") {
        if ($0 ~ /^##[[:space:]]+/) exit
        print
        next
      }
      content_index = match($0, /[^[:space:]]/)
      if (content_index == 0) {
        print
        next
      }
      indent = content_index - 1
      if (indent <= field_indent \
          && ($0 ~ /^[[:space:]]*```[[:space:]]*$/ \
            || $0 ~ /^[[:space:]]*"?[A-Za-z0-9_-]+"?[[:space:]]*:/ \
            || $0 ~ /^[[:space:]]*##[[:space:]]+/)) exit
      print
    }
  '
}

has_badge_observation() {
  local image="$1"
  awk -v image="$image" '
    {
      line = tolower($0)
      if (image == "before") {
        explicit_a = line ~ /(^|[^[:alnum:]])letter[[:space:]]+[[:punct:]]*a([^[:alnum:]]|$)/ \
          || line ~ /(^|[^[:alnum:]])(labeled|labelled|marked)([[:space:]]+with)?[[:space:]]+[[:punct:]]*a([^[:alnum:]]|$)/ \
          || line ~ /(^|[^[:alnum:]])reading[[:space:]]+[[:punct:]]*a([^[:alnum:]]|$)/ \
          || line ~ /(^|[^[:alnum:]])a[[:space:]]*-[[:space:]]*letter([^[:alnum:]]|$)/
        shape = line ~ /(^|[^[:alnum:]])(circle|circular|round)([^[:alnum:]]|$)/
        if (line ~ /(^|[^[:alnum:]])blue([^[:alnum:]]|$)/ \
            && shape && explicit_a \
            && line ~ /(^|[^[:alnum:]])badge([^[:alnum:]]|$)/) found = 1
      } else if (line ~ /(^|[^[:alnum:]])(amber|orange|gold)([^[:alnum:]]|$)/ \
          && line ~ /(^|[^[:alnum:]])manual([^[:alnum:]]|$)/ \
          && line ~ /(^|[^[:alnum:]])badge([^[:alnum:]]|$)/) {
        found = 1
      }
    }
    END { exit found ? 0 : 1 }
  '
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
    c4|c7)
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
      before="$REVIEW_EVIDENCE/before.png"
      after="$REVIEW_EVIDENCE/after.png"
      if [ ! -f "$before" ] || [ ! -f "$after" ]; then
        SCORE=0; FM='"missing-visual-media"'; return 0
      fi
      visual_evidence="$(visual_evidence_block < "$out")"
      if [ -z "$visual_evidence" ] \
        || ! is_path_cited "$before" "$visual_evidence" \
        || ! is_path_cited "$after" "$visual_evidence"; then
        SCORE=0; FM='"missing-visual-evidence"'; return 0
      fi
      if ! has_badge_observation before < "$out" \
        || ! has_badge_observation after < "$out" \
        || ! grep -Fq 'Changes save automatically.' "$out" \
        || ! grep -Fq 'Changes save when you select Save.' "$out"; then
        SCORE=0; FM='"visual-paths-without-inspection"'; return 0
      fi
      if grep -q '^status:[[:space:]]*reviewed' "$out"; then
        SCORE=8; FM=null
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
  input="${input//__VISUAL_MANIFEST__/$REVIEW_EVIDENCE/manifest.jsonl}"
  PRE="$(fixture_state "$id")"
  OUTFILE="$WORKDIR/out-$id.txt"
  dispatch "$input" > "$OUTFILE"
  POST="$(fixture_state "$id")"
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
