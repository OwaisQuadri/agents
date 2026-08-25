# phase 11 — dag

JOB: the task DAG(directed acyclic graph) validated, shown through /show-me, adversarially reviewed fresh, and approved by the human
IN:  tasks.json, test-cases.md; phase 10 committed
OUT: tasks.json final for implementation; a plan view shown through /show-me

## steps

1. Invoke /task-graph in task shape for the identifier, dependency, and cycle checks. Land tasks.json. Require the wave report and reverse-edge verdict. Done when the structural checks exit 0.
2. Invoke /show-me with tasks.json and the phase-06 through phase-10 plan files. Do not select its output format. Ask for the smallest view that shows every task and the user-experience choice.

   Use each task's `short` field as its primary label. Put its identifier beside the label only as a lookup key. An identifier never stands alone.

   Prefer a console-safe view. For parallel work, prefer an Xcode-style lane timeline. Use dependency waves as the horizontal axis. Let /show-me select any UML(Unified Modeling Language) view that makes another relationship clearer. Let /show-me select an interactive or web view only when density needs it. Done when the user sees the view and the view represents every task by name.
3. Run the fresh adversarial plan review. Dispatch anchor-verifier with three fields. Set `work_product_paths` to [tasks.json, data-structures.md, interfaces.md, test-cases.md]. Use the phase-09 topological-sort line for `verify_command`. Use this `rubric`:

   ```text
   [acyclic;
   disjoint sibling files;
   phase-06 and phase-07 deltas owned;
   each task covered;
   statuses correct].
   ```

   The dispatch must not carry this session's reasoning, drafts, or approval summary. Done when the verdict returns.
4. Fix each rubric failure. Re-dispatch until the verdict is pass. Each cycle increments `loop_counts.plan`. These fixes do not count against the phase-13 cap. Done when the verdict is pass.
5. Present Human Gate B. Include the chosen pattern and rejected alternates from ux.md. Include test-cases.md and tasks.json. Include structure as the DELTA since Gate S only — the shapes and signatures that changed after he approved them, and nothing he already read. On a walk-back visit, apply STANDING APPROVAL from SKILL.md FIRST: diff tasks.json, the plan docs, test-cases.md and ux.md against `gates.B.snapshot`. Where nothing decisional moved, record that the standing approval carries, append the diff to the walkback ledger, and continue to phase 12 without stopping. Never replay an identical bundle for another verdict. Present the plan view through /show-me. The interaction choice must reach the user before implementation. Commit `map(<ID>): phase 11 dag`. Done when the user sees the bundle, `state.json.gates.B` records the verdict, and the commit exists.

## blame tags

`dag-structure-wrong` `false-parallelism` `plan-review-missed-gap`
