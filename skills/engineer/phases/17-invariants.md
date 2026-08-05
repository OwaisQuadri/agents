# phase 17 — invariants

JOB: the applicable invariants selected from the global list plus project additions, applied to the PLAN to fixpoint — code touched only as new declarations and markers
IN:  `skills/engineer/invariants.md` (global, read every run) + the target repo's `.map/invariants.md` (project additions, created empty on first run); failures.jsonl and panel findings; interfaces.md
OUT: per-run selections in `.map/<ID>/invariants.md`; updated plan docs; new tasks + TODO markers for invariant work

## steps

1. grade every line of the global list and the project additions against this run's findings AND the branch diff (a clean test run alone never justifies selecting zero invariants — the diff decides applicability): copy applicable lines into `.map/<ID>/invariants.md` flipping `seed`→`selected`, record skips as `skipped` with no copying beyond the id. Done when every line is dispositioned.
2. append NEW invariants this run's findings imply (new id, status `selected`, the finding cited), each in the weakest form that still excludes the observed bad state — forbid what was seen, don't mandate one blessed shape; negative-space shapes (assert the states that must never occur are absent) are the default for exactly this reason, plus assertion-density and bounded-loop shapes per the global list's sources. Tier follows evidence: one finding stays run-local; a shape this project has hit before also goes to `.map/invariants.md`; house-general is proposed in the run report only when a second project can be cited. Done when no finding lacks its invariant.
3. each `selected` line names its affected phase (data shape→06, contract→07, coverage→10, ordering→11, marker→12); walk forward from the earliest per the walk-back rule — PLAN-ONLY: this phase's walks span 06 through 12 and never re-enter 13-16 (the implementation is shelved; there is nothing to rebuild or re-test yet). Apply invariants to the PLAN artifacts — in code, ONLY new declarations and `TODO(<task-id>)` markers, keeping the phase-18 stash-apply conflict surface minimal. New work becomes tasks in tasks.json, executed at 18. Done when each selected line is `applied` (its check passes or its task exists).
4. repeat the forward walk until one full pass changes nothing (`changed: []` on every visit) — cap `loop_counts.invariants` = 3 → human. Commit append-only: `map(<ID>): phase 17 invariants` — never rewrite the phase-12 commit; the stash depends on that history. Done at fixpoint with zero `selected` lines remaining.

## blame tags

`invariant-missed` `invariant-misapplied` `phase-mapping-wrong`
