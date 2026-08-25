# anchor-verifier — tuning history

## 2026-08-25 — baseline delta, and an aborted chain

Two mutations. ONE: `files_modified` is measured as a delta between an opening and a closing
snapshot over work_product_paths, so a work product that arrives dirty or untracked is the
normal received state and never fails the run on its own. TWO: a `verify_command` that
aborts partway grades the items that stage owned as fail, while items an independent file
read or sub-command can anchor are graded on those anchors. Unreached is not unverifiable.

Evidence: 21 logged runs that passed every dispatched rubric item, wrote nothing, and still
returned `fail` on a non-empty `git status`. 33 more where a chain died under `set -e` and
every later item was graded fail as "not reached".

HARNESS RESULT: A LOSS.

    incumbent (pre-change)  c1 10 c2 10 c3 10 c4 10 c6 10 c7 10   mean 10.00
    candidate (mutated)     c1 10 c2 10 c3 10 c4 10 c6  5 c7 10   mean  9.17

c7 does not discriminate: the unchanged definition already grades the aborted chain
correctly on that fixture. c6 REGRESSED, 10 to 5, `pass-without-executed-anchor` — the
candidate ran the command but redirected stderr, so the runtime marker never reached its
anchor. One sample each, so the regression is a signal and not a measurement.

WHY c6 CANNOT SHOW THE FIX: its dispatched rubric asks about dag.mmd regeneration and
disjoint wave file sets. Nothing in it triggers the verifier's own empty-`git status`
safeguard, which is what produced all 21 originals. The incumbent therefore passes it
cleanly. The exam does not contain the disease.

STATUS: NOT ACCEPTED ON THE HARNESS. A replacement c6 is being built by an author blind to
this text, whose dispatch is satisfiable in full by executed evidence WHILE the work
products are untracked and the tree is dirty. Any agent imposing a cleanliness safeguard
fails it; any agent grading on a delta passes. Re-decide on that result, and revert both
mutations if it still shows nothing.

Also recorded, and NOT caused by these mutations: holdout case c5 scores 0
`false-pass-unverifiable-claim` on BOTH definitions. The agent writes its own 50-thread
stress test to anchor a rubric item the case calls unverifiable. Pre-existing, in the
artifact's own catastrophic list, and owed its own ticket.

## 2026-08-25 — final decision: reverted after the rebuilt case still tied

A fenced author rebuilt c6 from the 21 production failures, not from either definition. The
new fixture contains a tracked-and-modified work product, an untracked test, and unrelated
sibling dirt. Every dispatched rubric item is fully provable by an executed command and a
file read. A deliberately defective stub that applies an empty-tree safeguard scores 0, so
the case is capable of detecting the named failure.

The actual result still tied:

    incumbent (pre-change)  c6 10  c7 4   mean 9.00 over 6 cases
    candidate (mutated)     c6 10  c7 4   mean 9.00 over 6 cases

The unchanged definition would not reproduce the 21 production failures even in a fixture
built to elicit them. Therefore the definition text was not sufficient cause. The open
question moves upstream: inspect the live dispatch briefs from those 21 runs for an explicit
cleanliness rubric or other condition absent from the fixture.

DECISION: REVERT both definition mutations. This follows the recorded status above ("revert
both mutations if [the replacement c6] still shows nothing") and the GEPA rule that ties go
to the incumbent. The fixtures and harness fixes stay: they are real test infrastructure,
and c6 proved it can score a guard-applying stub 0.
