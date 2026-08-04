#!/bin/sh
set -eu
[ $# -eq 1 ] || { echo "usage: next-ticket.sh <roadmap.json>" >&2; exit 2; }
report=$(jq -c '
def tdeps($deps; $id; $seen):
  if ($seen | index($id)) then error("cycle: " + (($seen + [$id]) | join(" -> ")))
  else ($deps[$id] // []) | map(. as $d | [$d] + tdeps($deps; $d; $seen + [$id])) | add // [] | unique
  end;
.tickets as $items
| ($items | map(.id)) as $ids
| ([$items[] | .id as $t | (.deps // [])[] | . as $d | select(($ids | index($d)) | not) | "\($d) (dep of \($t))"]) as $unknown
| if ($unknown | length) > 0 then error("unknown dep: " + ($unknown | join(", "))) else . end
| ($items | map({key: .id, value: (.deps // [])}) | from_entries) as $deps
| ($items | map(tdeps($deps; .id; []))) as $cyclecheck
| ($items | map({key: .id, value: .status}) | from_entries) as $status
| [$items[] | select(.status == "todo")] as $todo
| {replan: [$todo[] | select([.deps[] | $status[.] == "cancelled"] | any) | .id],
   ranked: ([$todo[]
     | select([.deps[] | $status[.] == "done"] | all)
     | . as $c
     | {id: .id, unlocks: ([$todo[] | select(.id != $c.id) | select(tdeps($deps; .id; []) | index($c.id))] | length)}]
     | sort_by([-.unlocks, .id]))}
' "$1")
printf '%s\n' "$report" | jq -r '.replan[] | "needs-replan: " + . + " (cancelled dep)"' >&2
printf '%s\n' "$report" | jq -r '.ranked[] | "  " + .id + " unlocks " + (.unlocks | tostring)' >&2
next=$(printf '%s\n' "$report" | jq -r '.ranked[0].id // empty')
[ -n "$next" ] || { echo "no runnable ticket: every todo ticket is blocked" >&2; exit 1; }
printf '%s\n' "$next"
