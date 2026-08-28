#!/bin/sh
set -eu
usage() { echo "usage: gh-edge-guard.sh <issue-number> --blocked-by <id,id,...>" >&2; exit 2; }

[ $# -eq 3 ] || usage
issue=$1
flag=$2
raw_ids=$3
[ "$flag" = "--blocked-by" ] || usage

# Normalize once, up front: trim whitespace and reduce an issue URL or a
# '#'-prefixed id down to its bare number, for every id in the list. Both the
# real `gh` call and the round-trip check below use this cleaned list, so a
# natural typo ("5, 12", a pasted issue URL) works the same as a bare "5,12".
ids=$(printf '%s' "$raw_ids" | tr ',' '\n' \
  | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//; s#.*/##; s/^#//' \
  | paste -sd, -)

err=$(mktemp)
out=$(mktemp)
trap 'rm -f "$err" "$out"' EXIT

if ! gh issue edit "$issue" --add-blocked-by "$ids" >"$out" 2>"$err"; then
  # Best-effort: GitHub's cycle-rejection message is plain English, not a
  # structured error code (no extensions.code/type field was found exposed by
  # `gh` for this call) — a future wording change on GitHub's side silently
  # falls through to the generic failure branch below, losing the named "cycle:"
  # reason. Case-insensitive match to survive minor capitalization drift only.
  if grep -qi "would create a cycle" "$err"; then
    echo "cycle: adding blocked-by $ids to #$issue would create a cycle (rejected by GitHub)" >&2
    exit 1
  fi
  echo "gh-edge-guard: gh issue edit failed:" >&2
  cat "$err" >&2
  exit 1
fi

# AGNT-INV-003 round-trip: confirm the edge actually landed as sent, not just that
# gh returned 0.
got=$(gh issue view "$issue" --json blockedBy --jq '[.blockedBy.nodes[].number] | sort | join(",")')
for want_id in $(printf '%s' "$ids" | tr ',' '\n'); do
  case ",$got," in
    *,"$want_id",*) ;;
    *) echo "gh-edge-guard: reported success but #$issue's blockedBy does not include $want_id (got: $got)" >&2; exit 1 ;;
  esac
done

echo "ok: #$issue now blocked-by $ids" >&2
