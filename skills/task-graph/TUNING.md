# task-graph: tuning record

The GEPA loop's inputs and outputs for this skill. `SKILL.md` never loads it. Open it when you
tune this skill, and not when you build a graph.

## accepted mutations

- 2026-08-10, injection became bidirectional. The owner asked this at the live CPU-0026 gate D:
  "do we consider the dependencies created for the existing roadmap when we inject tickets into it? like does the make it right ticket block any existing tickets on the roadmap and is that updated automatically?"
  It did not, and it was not. Step 3 only ever computed the new item's dependencies. Nothing
  here, and nothing in phase 22 of the engineer map, ever revisited an existing item. The graph
  stays valid and acyclic, which is why the gap survived. `next-ticket.sh` ranks by transitive
  dependents, so every newly filed ticket scored 0 unlocks and sorted last. The live case shows
  the cost. A barge-in-redesign ticket reshapes a filed echo-suppression ticket, and it would
  land with no edge. The picker would then send someone to tune a multiplier against a design
  that a later ticket replaces. The harness had certified the old behaviour, because `t3` is an
  injection case whose expect says nothing about edges. The fence bars the mutation-proposer
  from writing that case. A fenced author appended two instead: `t7`, the regression, and `t8`,
  a held-out negative. `t8` catches a skill that reports reverse edges only where it finds some.
  Against defect stubs, the one-directional defect scores 9 on `t3` and 2 on `t7`.
- 2026-08-05, shared-file wave rejection scoped to the tasks shape. Filing tickets into
  conversation mode's roadmap hit it on pairs of done and cancelled tickets. The live roadmap
  already violated it while it carried honest result-dependencies only. That is evidence the
  rule never fit the serial ticket shape.
- 2026-08-03, authored for the engineer map, phases 11 and 22. Same-day blind-judge fixes, grade
  6, all by execution. `dag-mermaid.sh` rejects out-of-enum statuses, same-wave shared files,
  and duplicate or sanitize-colliding ids, and it tolerates a missing `deps` key end-to-end.
  `next-ticket.sh` rejects unknown deps and names cycles consistently. Step 3's shared-file edge
  follows the existing partial order, so it can no longer manufacture a cycle. The skill states
  NN overflow, the empty-roadmap counter, `blame_phase`, `kind:ticket`, and the override
  destination now.
