# phase 17 — invariants

JOB: the applicable invariants selected from the global list plus project additions, applied to the PLAN to fixpoint — code touched only as new declarations and markers
IN:  `skills/engineer/invariants.md` (global, read every run) + the target repo's `.map/invariants.md` (project additions, created empty on first run); failures.jsonl and panel findings; interfaces.md
OUT: per-run selections in `.map/<ID>/invariants.md`; updated plan docs; new tasks + TODO markers for invariant work

## steps

1. grade every line of the global list and of the project additions against this run's findings AND the branch diff. A clean test run alone never justifies selecting zero invariants, because the diff decides applicability. Copy the applicable lines into `.map/<ID>/invariants.md` and flip `seed` to `selected`. Record a skip as `skipped`, and copy nothing beyond its id. Done when every line is dispositioned.
2. append the NEW invariants this run's findings imply. Each new line carries a new id, the status `selected`, and the finding it cites. Write each one in the weakest form that still excludes the observed bad state: forbid what was seen, and never mandate one blessed shape. Negative-space shapes assert that the states that must never occur are absent, and they are the default for exactly this reason. The global list's sources add the assertion-density and bounded-loop shapes. The tier follows the evidence. One finding stays run-local. A shape this project has hit before also goes to `.map/invariants.md`. Propose a house-general invariant in the run report only when a second project can be cited. Done when no finding lacks its invariant.
3. each `selected` line names its affected phase: data shape → 06, contract → 07, coverage → 10, ordering → 11, marker → 12. Walk forward from the earliest one, per the walk-back rule. This phase is PLAN-ONLY. Its walks span 06 through 12, and they never re-enter 13-16, because the implementation is shelved and there is nothing to rebuild or re-test yet. Apply the invariants to the PLAN artifacts. In code, write ONLY new declarations and `TODO(<task-id>)` markers, which keeps the conflict surface of the phase-18 stash apply minimal. New work becomes tasks in tasks.json, and phase 18 executes them. Done when each selected line is `applied`, which means its check passes or its task exists.
4. repeat the forward walk until one full pass changes nothing, which reads as `changed: []` on every visit. The cap `loop_counts.invariants` = 3 routes to the human. Commit append-only as `map(<ID>): phase 17 invariants`. Never rewrite the phase-12 commit, because the stash depends on that history. Done at fixpoint, with zero `selected` lines remaining.

## blame tags

`invariant-missed` `invariant-misapplied` `phase-mapping-wrong`
