# phase 09 — dependencies

JOB: every task's deps filled with real result-edges, acyclic, with no hidden shared-file edges
IN:  tasks.json; phase 08 committed
OUT: tasks.json with deps filled

## steps

1. fill each task's `deps` with the ids whose RESULT it truly needs. Apply the fake-edge test from workflow-author: typed order is not a dependency. Done when every edge answers "needs the result" with yes.
2. the hidden-resource rule: any two tasks that share a file get an edge in the natural order, even with no data dependency. Parallel same-file edits are the documented failure mode, per docs/agents-fleet-research.md. Done when no two dep-free siblings share a file.
3. verify that the graph is acyclic: `jq -r '.tasks[] | .deps[] as $d | "\($d) \(.id)"' tasks.json | tsort >/dev/null`. The authoritative check runs inside dag-mermaid.sh in phase 11, because the macOS `tsort` warns without failing. Done when no cycle exists.
4. commit `map(<ID>): phase 09 dependencies`.

## blame tags

`missing-edge` `false-edge` `same-file-parallelism`
