#!/bin/sh
set -eu
[ $# -eq 0 ] || { echo "usage: next-issue.sh (reads the current repo's GitHub Issues via gh; no arguments)" >&2; exit 2; }

raw=$(gh issue list --state all --json number,title,labels,blockedBy --limit 500)

count=$(printf '%s' "$raw" | jq 'length')
if [ "$count" -eq 500 ]; then
  echo "next-issue.sh: fetched exactly 500 issues (the --limit cap) — the repo may hold" >&2
  echo "more than that and this ranking would silently exclude them; raise --limit in" >&2
  echo "the script before trusting this output on a backlog this large" >&2
  exit 1
fi

report=$(printf '%s' "$raw" | jq -c '
def tdeps($deps; $id; $seen):
  if ($seen | index($id)) then error("cycle: " + (($seen + [$id]) | map(tostring) | join(" -> ")))
  else ($deps[$id | tostring] // []) | map(. as $d | [$d] + tdeps($deps; $d; $seen + [$id])) | add // [] | unique
  end;

. as $items
| ($items | map(.number)) as $ids
| ($items | map({
    number,
    status_labels: [.labels[].name | select(startswith("status:")) | ltrimstr("status:")],
    status: ([.labels[].name | select(startswith("status:")) | ltrimstr("status:")] | first),
    priority: ([.labels[].name | select(startswith("priority:")) | ltrimstr("priority:")] | first // "med"),
    blocked_by: [.blockedBy.nodes[].number]
    # blocked_by is repo-local: a cross-repo blockedBy edge (GitHub supports
    # "org/repo#N") would collide with a same-numbered local issue or spuriously
    # error as unknown below. This repo files no cross-repo dependencies today;
    # if that ever changes, this extraction needs to read blockedBy.nodes[].repository
    # too and scope the id comparison accordingly — not built speculatively here.
  })) as $items

| ([$items[] | select(.status == null) | .number]) as $unstatused
| if ($unstatused | length) > 0 then error("missing status: label on issue(s): " + ($unstatused | map(tostring) | join(", "))) else . end

| ([$items[] | select(.status_labels | length > 1) | "\(.number) (" + (.status_labels | join(", ")) + ")"]) as $multistatus
| if ($multistatus | length) > 0 then error("multiple status: labels on issue(s): " + ($multistatus | join(", "))) else . end

| ([$items[] | .number as $t | .blocked_by[] | . as $d | select(($ids | index($d)) | not) | "\($d) (blocker of \($t))"]) as $unknown
| if ($unknown | length) > 0 then error("unknown blocker: " + ($unknown | join(", "))) else . end

| ($items | map({key: (.number | tostring), value: .blocked_by}) | from_entries) as $deps
| ($items | map(tdeps($deps; .number; []))) as $cyclecheck

| ($items | map({key: (.number | tostring), value: .status}) | from_entries) as $status
| [$items[] | select(.status == "todo")] as $todo

| {replan: [$todo[] | select([.blocked_by[] | $status[. | tostring] == "cancelled"] | any) | .number],
   ranked: ([$todo[]
     | select([.blocked_by[] | $status[. | tostring] == "done"] | all)
     | . as $c
     | {number: .number, priority: .priority,
        prio_rank: ({"urgent":0,"high":1,"med":2,"low":3}[.priority] // 2),
        unlocks: ([$todo[] | select(.number != $c.number) | select(tdeps($deps; .number; []) | index($c.number))] | length)}]
     | sort_by([.prio_rank, -.unlocks, .number]))}
')

printf '%s\n' "$report" | jq -r '.replan[] | "needs-replan: #" + (. | tostring) + " (cancelled blocker)"' >&2
printf '%s\n' "$report" | jq -r '.ranked[] | "  #" + (.number | tostring) + " [" + .priority + "] unlocks " + (.unlocks | tostring)' >&2
next=$(printf '%s\n' "$report" | jq -r '.ranked[0].number // empty')
if [ -z "$next" ]; then
  if [ "$count" -eq 0 ]; then
    echo "no runnable issue: this repo has no issues at all yet" >&2
  else
    echo "no runnable issue: every todo issue is blocked" >&2
  fi
  exit 1
fi
printf '%s\n' "$next"
