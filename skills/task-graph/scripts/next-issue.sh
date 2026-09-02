#!/bin/sh
set -eu
[ $# -eq 0 ] || { echo "usage: next-issue.sh (reads the current repo's GitHub Issues via gh; no arguments)" >&2; exit 2; }

raw=$(gh issue list --state all --json number,title,blockedBy --limit 500)

count=$(printf '%s' "$raw" | jq 'length')
if [ "$count" -eq 500 ]; then
  echo "next-issue.sh: fetched exactly 500 issues (the --limit cap) — the repo may hold" >&2
  echo "more than that and this ranking would silently exclude them; raise --limit in" >&2
  echo "the script before trusting this output on a backlog this large" >&2
  exit 1
fi

repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
owner=${repo%%/*}
name=${repo#*/}
project_number=$(gh api graphql \
  -f query='query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { projectsV2(first: 2) { nodes { number } } } }' \
  -f owner="$owner" -f name="$name" --jq '.data.repository.projectsV2.nodes | if length == 1 then .[0].number else error("expected exactly one project linked to the repo, found " + (length | tostring)) end')

fields_raw=$(gh project item-list "$project_number" --owner "$owner" --format json --limit 500)
item_count=$(printf '%s' "$fields_raw" | jq '.totalCount')
if [ "$item_count" -gt 500 ]; then
  echo "next-issue.sh: the project holds $item_count items but only 500 were fetched" >&2
  echo "(the --limit cap); raise --limit in the script before trusting this output" >&2
  exit 1
fi

report=$(printf '%s' "$raw" | jq -c --argjson fields "$fields_raw" --arg repo "$repo" '
def tdeps($deps; $id; $seen):
  if ($seen | index($id)) then error("cycle: " + (($seen + [$id]) | map(tostring) | join(" -> ")))
  else ($deps[$id | tostring] // []) | map(. as $d | [$d] + tdeps($deps; $d; $seen + [$id])) | add // [] | unique
  end;

# Project single-select values arrive as display names ("In progress", "Urgent");
# normalize to the hyphenated lowercase enum the ranking below compares against.
def norm: if . == null then null else ascii_downcase | gsub(" "; "-") end;

. as $items
| ($items | map(.number)) as $ids
| ($fields.items
   | map(select(.content.type == "Issue" and ((.content.repository // "") | endswith($repo)))
     | {key: (.content.number | tostring),
        value: {status: (.status | norm), priority: (.priority | norm)}})
   | from_entries) as $pf
| ($items | map({
    number,
    status: ($pf[.number | tostring].status),
    priority: ($pf[.number | tostring].priority // "med"),
    blocked_by: [.blockedBy.nodes[].number]
    # blocked_by is repo-local: a cross-repo blockedBy edge (GitHub supports
    # "org/repo#N") would collide with a same-numbered local issue or spuriously
    # error as unknown below. This repo files no cross-repo dependencies today;
    # if that ever changes, this extraction needs to read blockedBy.nodes[].repository
    # too and scope the id comparison accordingly — not built speculatively here.
  })) as $items

| ([$items[] | select(.status == null) | .number]) as $unstatused
| if ($unstatused | length) > 0 then error("missing project Status on issue(s): " + ($unstatused | map(tostring) | join(", "))) else . end

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
