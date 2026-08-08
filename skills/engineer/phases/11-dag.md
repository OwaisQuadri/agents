# phase 11 — dag

JOB: the task DAG(directed acyclic graph) validated, rendered, adversarially reviewed fresh, and approved by the human
IN:  tasks.json, test-cases.md; phase 10 committed
OUT: `.map/<ID>/dag.mmd`; tasks.json final for implementation

## steps

1. invoke /task-graph in task shape. It validates the graph, lands tasks.json, regenerates dag.mmd through `skills/task-graph/scripts/dag-mermaid.sh`, and reports the waves. Show the diagram. Done when the script exits 0 and the .mmd is committed-ready.
2. [FRESH] run the adversarial plan review. Dispatch anchor-verifier with three fields. `work_product_paths` = [tasks.json, dag.mmd, data-structures.md, interfaces.md, test-cases.md]. `verify_command` = the tsort line from phase 09. `rubric` = [acyclic; every wave's siblings have disjoint files; every phase-06/07 delta covered by a task; every task has ≥1 covering test case; statuses all todo]. The dispatch must NOT carry this session's reasoning, the drafts, or any "it's fine" summary. Done when the verdict returns.
3. fix the rubric failures, then re-dispatch until the verdict is pass. These cycles count against `loop_counts.plan`, which is their own counter. Plan-review fixes never count against the cap in phase 13. Done when the verdict is pass.
4. HUMAN GATE B. Present the plan bundle: the chosen pattern in ux.md with its rejected alternates, data-structures.md, interfaces.md, test-cases.md, tasks.json, and the dag.mmd rendering. ux.md is in the bundle because phase 03 PICKS an interaction pattern. An unshown choice reaches implementation without the user ever seeing it. Commit `map(<ID>): phase 11 dag`. Done when the user has seen the bundle, `state.json.gates.B` records their verdict, and the commit exists.

## blame tags

`dag-structure-wrong` `false-parallelism` `plan-review-missed-gap`
