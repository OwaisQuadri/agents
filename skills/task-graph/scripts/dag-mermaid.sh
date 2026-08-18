#!/bin/sh
set -eu
[ $# -eq 1 ] || { echo "usage: dag-mermaid.sh <tasks.json|roadmap.json>" >&2; exit 2; }
jq -r '
def depth($deps; $id; $seen):
  if ($seen | index($id)) then error("cycle: " + (($seen + [$id]) | join(" -> ")))
  elif ($deps | has($id) | not) then error("unknown dep: " + $id)
  else $deps[$id]
  | if length == 0 then 0
    else ([.[] | depth($deps; .; $seen + [$id])] | max) + 1
    end
  end;
(.tasks != null) as $is_tasks
| (.tasks // .tickets) as $items
| ($items | map(.id)) as $ids
| ($ids | group_by(.) | map(select(length > 1) | .[0])) as $dups
| if ($dups | length) > 0 then error("duplicate id: " + ($dups | join(" "))) else . end
| ($ids | map(gsub("[^A-Za-z0-9]"; "_")) | group_by(.) | map(select(length > 1) | .[0])) as $sdups
| if ($sdups | length) > 0 then error("id collision after sanitize: " + ($sdups | join(" "))) else . end
| ($items | map(select(.status as $s | (["todo","in progress","resolved","cancelled","done"] | index($s)) | not) | .id)) as $bad
| if ($bad | length) > 0 then error("status outside enum: " + ($bad | join(" "))) else . end
| ($items | map(select(.status == "cancelled") | .id)) as $dead
| ([$items[] | select(.status != "cancelled") | .id as $id | (.deps // [])[]
    | select(. as $d | ($dead | index($d)) != null) | "\($id)->\(.)"]) as $orphans
| if ($orphans | length) > 0 then error("depends on a cancelled item: " + ($orphans | join(" "))) else . end
| ($items | map({key: .id, value: (.deps // [])}) | from_entries) as $deps
| ($items | map(. + {wave: (depth($deps; .id; []) + 1)})) as $waved
| ([$waved | map(select(.status != "cancelled")) | group_by(.wave)[]
    | . as $w
    | range(0; length) as $i
    | range($i + 1; length) as $j
    | select(($w[$i].files // []) | any(. as $f | (($w[$j].files // []) | index($f)) != null))
    | "\($w[$i].id)+\($w[$j].id)"]) as $overlap
| if $is_tasks and (($overlap | length) > 0) then error("shared file inside one wave: " + ($overlap | join(" "))) else . end
| ["flowchart TD",
   "  classDef todo fill:#eeeeee,stroke:#999999",
   "  classDef inprogress fill:#fff3cd,stroke:#b8860b",
   "  classDef resolved fill:#cce5ff,stroke:#004085",
   "  classDef done fill:#d4edda,stroke:#155724",
   "  classDef cancelled fill:#f8d7da,stroke:#721c24,stroke-dasharray: 5 5"]
  + ([$waved | group_by(.wave)[]
      | ["  subgraph wave\(.[0].wave) [\"wave \(.[0].wave) — parallel\"]"]
        + [.[] | "    \(.id | gsub("[^A-Za-z0-9]"; "_"))[\"\(.id) \(.short | gsub("\""; ""))\"]:::\(.status | gsub(" "; ""))"]
        + ["  end"]
     ] | add)
  + [$waved[] | .id as $tid | (.deps // [])[] as $d | "  \($d | gsub("[^A-Za-z0-9]"; "_")) --> \($tid | gsub("[^A-Za-z0-9]"; "_"))"]
| .[]
' "$1"
