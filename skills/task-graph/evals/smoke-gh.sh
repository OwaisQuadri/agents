#!/usr/bin/env bash
# Live smoke test for next-issue.sh / gh-edge-guard.sh against the real GitHub repo
# via gh. NOT run by run.sh's default path (would spam the real issue tracker on
# every eval run) — invoke explicitly: skills/task-graph/evals/smoke-gh.sh
#
# Assertions check RELATIVE ranking/flags among the scratch issues this script
# creates, never the global next-issue.sh pick — the real repo backlog is live
# alongside these, so a scratch issue can never be guaranteed the global #1 slot
# (an existing lower-numbered same-priority issue always wins a tie).
set -euo pipefail
cd "$(dirname "$0")"

created=()
cleanup() {
  if [[ ${#created[@]} -gt 0 ]]; then
    for n in "${created[@]}"; do
      gh issue delete "$n" --yes >/dev/null 2>&1 || true
    done
  fi
}
trap cleanup EXIT

mk() {
  # mk <title> <status> [gh-issue-create extra args...]
  local title=$1 status=$2
  shift 2
  local out n
  out=$(gh issue create --title "$title" --body "smoke-gh scratch, auto-deleted" "$@")
  n=${out##*/}
  ../scripts/gh-issue-field.sh "$n" Status "$status" >/dev/null 2>&1
  ../scripts/gh-issue-field.sh "$n" Priority urgent >/dev/null 2>&1
  echo "$n"
}

echo "smoke-gh: base/mid/leaf/side tie-break + unlocks ranking" >&2
A=$(mk "smoke-gh A" done); created+=("$A")
gh issue close "$A" --reason completed >/dev/null
B=$(mk "smoke-gh B" todo --blocked-by "$A"); created+=("$B")
C=$(mk "smoke-gh C" todo --blocked-by "$B"); created+=("$C")
D=$(mk "smoke-gh D" todo --blocked-by "$A"); created+=("$D")

diag=$(../scripts/next-issue.sh 2>&1 >/dev/null || true)
b_line=$(grep -n "^  #$B " <<<"$diag" | cut -d: -f1)
d_line=$(grep -n "^  #$D " <<<"$diag" | cut -d: -f1)
grep -q "^  #$B \[urgent\] unlocks 1" <<<"$diag" || { echo "smoke-gh: #$B wrong unlocks count" >&2; echo "$diag" >&2; exit 1; }
grep -q "^  #$D \[urgent\] unlocks 0" <<<"$diag" || { echo "smoke-gh: #$D wrong unlocks count" >&2; echo "$diag" >&2; exit 1; }
[[ -n "$b_line" && -n "$d_line" && "$b_line" -lt "$d_line" ]] || { echo "smoke-gh: #$B (more unlocks) did not rank before #$D" >&2; exit 1; }

echo "smoke-gh: cancelled-blocker replan flag" >&2
gh issue close "$B" --reason "not planned" >/dev/null
../scripts/gh-issue-field.sh "$B" Status cancelled >/dev/null 2>&1
err=$(../scripts/next-issue.sh 2>&1 >/dev/null || true)
grep -q "needs-replan: #$C" <<<"$err" || { echo "smoke-gh: replan warning missing for #$C" >&2; exit 1; }
grep -q "^  #$C " <<<"$err" && { echo "smoke-gh: replanned #$C wrongly appeared in ranked list" >&2; exit 1; }

echo "smoke-gh: gh-edge-guard.sh happy path + whitespace-normalized id list" >&2
E=$(mk "smoke-gh E" todo); created+=("$E")
F=$(mk "smoke-gh F" todo); created+=("$F")
G=$(mk "smoke-gh G" todo); created+=("$G")
../scripts/gh-edge-guard.sh "$F" --blocked-by "$E, $G" >/dev/null 2>&1
got=$(gh issue view "$F" --json blockedBy --jq '[.blockedBy.nodes[].number] | sort | join(",")')
want=$(printf '%s\n%s' "$E" "$G" | sort -n | paste -sd, -)
[[ "$got" == "$want" ]] || { echo "smoke-gh: edge-guard happy path didn't land, got $got want $want" >&2; exit 1; }

echo "smoke-gh: gh-edge-guard.sh refuses a cycle, zero mutation, named-reason branch fires" >&2
before=$(gh issue view "$E" --json blockedBy --jq '[.blockedBy.nodes[].number]')
cycle_err=$(../scripts/gh-edge-guard.sh "$E" --blocked-by "$F" 2>&1 >/dev/null || true)
grep -q "^cycle:" <<<"$cycle_err" || { echo "smoke-gh: cycle refusal did not hit the named-reason branch (got: $cycle_err)" >&2; exit 1; }
after=$(gh issue view "$E" --json blockedBy --jq '[.blockedBy.nodes[].number]')
[[ "$before" == "$after" ]] || { echo "smoke-gh: refused edge still mutated state" >&2; exit 1; }

echo "smoke-gh: next-issue.sh errors clearly on an issue missing its project Status" >&2
out=$(gh issue create --title "smoke-gh H" --body "smoke-gh scratch, auto-deleted")
H=${out##*/}; created+=("$H")
missing_err=$(../scripts/next-issue.sh 2>&1 >/dev/null || true)
grep -q "missing project Status on issue(s): $H" <<<"$missing_err" || { echo "smoke-gh: missing-status error missing or wrong issue named (got: $missing_err)" >&2; exit 1; }
../scripts/gh-issue-field.sh "$H" Status done >/dev/null 2>&1

echo "smoke-gh: all cases pass" >&2
