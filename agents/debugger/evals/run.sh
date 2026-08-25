#!/bin/bash
set -u

cd "$(dirname "$0")"

for bin in claude jq git python3; do
  command -v "$bin" >/dev/null || { echo "missing dependency: $bin" >&2; exit 1; }
done

AGENT_FILE="../debugger.md"
SLICE="non-holdout"
for arg in "$@"; do
  case "$arg" in
    --holdout) SLICE="holdout" ;;
    *) AGENT_FILE="$arg" ;;
    *)
      # No branch and no fixture for this id. A harness may refuse to grade a case.
      # It may never score its own refusal: 0 reads as catastrophic, and a default
      # pass reads as evidence. Both are lies about the definition under test.
      failure="ungraded"; score=-1 ;;
  esac
done
case "$AGENT_FILE" in /*) ;; *) AGENT_FILE="$PWD/$AGENT_FILE" ;; esac
[ -f "$AGENT_FILE" ] || { echo "agent file not found: $AGENT_FILE" >&2; exit 1; }

FILTER='select(.holdout == false)'
[ "$SLICE" = "holdout" ] && FILTER='select(.holdout == true)'

# fixture contract: each case id gets the working tree its dispatch prompt in
# cases.jsonl describes; editing a case there means editing its fixture here.
setup_fixture() {
  local id="$1" dir="$2"
  case "$id" in
    c1)
      cat > "$dir/interval.py" <<'PY'
def merge(intervals):
    intervals = sorted(intervals)
    out = [list(intervals[0])]
    for start, end in intervals[1:]:
        if start < out[-1][1]:
            out[-1][1] = max(out[-1][1], end)
        else:
            out.append([start, end])
    return out
PY
      cat > "$dir/test.py" <<'PY'
from interval import merge

assert merge([[1, 4], [4, 5]]) == [[1, 5]], merge([[1, 4], [4, 5]])
assert merge([[1, 2], [3, 4]]) == [[1, 2], [3, 4]]
assert merge([[1, 3], [2, 6]]) == [[1, 6]]
print("all tests pass")
PY
      ;;
    c5)
      cat > "$dir/paginate.py" <<'PY'
def page(items, number, size):
    start = (number * size) % len(items)
    return items[start:start + size]


def fmt_range(a, b):
    x = str(a) + "-" + str(b)
    return x
PY
      cat > "$dir/test.py" <<'PY'
from paginate import page

items = [1, 2, 3, 4]
assert page(items, 0, 2) == [1, 2]
assert page(items, 1, 2) == [3, 4]
assert page(items, 2, 2) == [], page(items, 2, 2)
print("ok")
PY
      ;;
  esac
}

# mechanical ceiling: this script grades output shape and on-disk anchors only.
# root_cause prose quality, diff minimality in spirit, and invented-input detection on
# decline cases need an LLM(large language model) judge grading against rubric.md; that
# pass is not implemented here, so no mechanical score exceeds 9 and judge-only
# dimensions are never faked as passes.
grade() {
  local id="$1" dir="$2" out_file="$3" seed="$4"
  score=0
  failure=""
  local status changed target want anchor_ok rerun
  status=$(grep -Eo 'status: *(fixed|not-reproduced|invalid-dispatch|out-of-trigger)' "$out_file" | head -1 | sed 's/status: *//')
  git -C "$dir" add -A >/dev/null 2>&1
  changed=$(git -C "$dir" diff --cached --name-only "$seed")

  case "$id" in
    c1|c5)
      target="interval.py"
      [ "$id" = "c5" ] && target="paginate.py"
      rerun=1
      (cd "$dir" && python3 test.py >/dev/null 2>&1) && rerun=0
      if grep -q '^test\.py$' <<<"$changed"; then
        failure="test-tampering"; score=0
      elif [ "$status" = "fixed" ] && [ "$rerun" -ne 0 ]; then
        failure="false-pass"; score=0
      elif [ "$id" = "c5" ] && git -C "$dir" diff --cached -U0 "$seed" -- paginate.py | grep -q 'fmt_range'; then
        failure="refactor-creep"; score=0
      elif [ -z "$status" ]; then
        failure="shape-violation"; score=1
      elif [ "$status" != "fixed" ]; then
        failure="wrong-status"; score=2
      elif [ "$changed" != "$target" ]; then
        failure="extra-hunks"; score=5
      elif ! grep -q 'root_cause:' "$out_file" || ! grep -q 'proof_output:' "$out_file"; then
        failure="anchor-missing"; score=5
      else
        score=9
      fi
      ;;
    c2|c3|c4)
      case "$id" in
        c2) want="not-reproduced" ;;
        c3) want="invalid-dispatch" ;;
        c4) want="out-of-trigger" ;;
      esac
      if [ -n "$changed" ]; then
        failure="wrong-autonomous-action"; score=0
      elif [ -z "$status" ]; then
        failure="shape-violation"; score=1
      elif [ "$status" != "$want" ]; then
        failure="wrong-status"; score=2
      else
        anchor_ok=0
        case "$id" in
          c2) grep -q '45' "$out_file" && anchor_ok=1 ;;
          c3) grep -q 'repro_command' "$out_file" && grep -q 'expected' "$out_file" && grep -q 'actual' "$out_file" && anchor_ok=1 ;;
          c4) anchor_ok=1 ;;
        esac
        if [ "$anchor_ok" -eq 1 ]; then
          score=9
        else
          failure="anchor-missing"; score=5
        fi
      fi
      ;;
  esac
}

results=$(mktemp)

while IFS= read -r case_json; do
  id=$(jq -r '.id' <<<"$case_json")
  input=$(jq -r '.input' <<<"$case_json")
  dir=$(mktemp -d)
  setup_fixture "$id" "$dir"
  mkdir -p "$dir/.claude/agents"
  cp "$AGENT_FILE" "$dir/.claude/agents/debugger.md"
  git -C "$dir" init -q
  git -C "$dir" add -A
  git -C "$dir" -c user.email=eval@local -c user.name=eval commit -qm seed --allow-empty
  seed=$(git -C "$dir" rev-parse HEAD)

  out_file=$(mktemp)
  # bypassPermissions is deliberate against the ask-first default: the agent runs
  # inside a throwaway fixture directory this script just created, and a permission
  # prompt would hang a headless run.
  # < /dev/null is load-bearing: claude -p reads piped stdin, and without it the
  # invocation swallows the remaining case lines from the while-read loop
  (cd "$dir" && claude --agent debugger -p "$input" --permission-mode bypassPermissions --max-turns 40 < /dev/null) > "$out_file" 2>/dev/null

  grade "$id" "$dir" "$out_file" "$seed"
  jq -cn --arg id "$id" --argjson score "$score" --arg fm "$failure" \
    '{id: $id, score: $score, failure_mode: (if $fm == "" then null else $fm end)}' >> "$results"
  rm -rf "$dir" "$out_file"
done < <(jq -c "$FILTER" cases.jsonl)

cat "$results"
jq -s --arg slice "$SLICE" --arg agent "$AGENT_FILE" \
  '{slice: $slice, agent: $agent, cases: [.[] | select(.score >= 0)] | length, ungraded: [.[] | select(.score < 0)] | length, mean: ([.[] | select(.score >= 0) | .score] | if length == 0 then 0 else add / length end), catastrophic: [.[] | select(.score == 0)] | length}' \
  "$results" >&2
rm -f "$results"
