# phase 11 — dag

JOB: the task DAG(directed acyclic graph) validated, rendered, adversarially reviewed fresh, and approved by the human
IN:  tasks.json, test-cases.md; phase 10 committed
OUT: `.map/<ID>/dag.mmd`; tasks.json final for implementation

## steps

1. invoke /task-graph (task shape): it validates, lands tasks.json, regenerates dag.mmd via `skills/task-graph/scripts/dag-mermaid.sh`, and reports the waves. Show the diagram. Done when the script exits 0 and the .mmd is committed-ready.
2. [FRESH] adversarial plan review — dispatch anchor-verifier with `work_product_paths` = [tasks.json, dag.mmd, data-structures.md, interfaces.md, test-cases.md], `verify_command` = the tsort line from phase 09, `rubric` = [acyclic; every wave's siblings have disjoint files; every phase-06/07 delta covered by a task; every task has ≥1 covering test case; statuses all todo]. The dispatch must NOT carry this session's reasoning, drafts, or any "it's fine" summary. Done when the verdict returns.
3. fix rubric failures and re-dispatch until pass — cycles here count against `loop_counts.plan` (their own counter; conflating plan-review fixes with implementation walk-backs would burn 13's cap). Done when the verdict is pass.
4. HUMAN GATE B: present the plan bundle — data-structures.md, interfaces.md, test-cases.md, tasks.json, the dag.mmd rendering. Commit `map(<ID>): phase 11 dag`. Done when the user has seen the bundle and the commit exists.

## blame tags

`dag-structure-wrong` `false-parallelism` `plan-review-missed-gap`
