# phase 15 — diagnose

JOB: every failure root-caused, blamed on a phase, and driven to zero through the earliest blamed phase — with the plan reconciled to reality
IN:  failures.jsonl with open lines (from phase 14 or 19), deviations.jsonl, the plan docs
OUT: failures triaged (`blame_phase` set, status fixed), walkback.jsonl entries, updated plan docs

## steps

1. [FRESH] dispatch a debugger for each `triaged: false` failure. The dispatch carries `repro_command`, `expected`, and `actual` verbatim from the line, plus scope = the line's `area`. The dispatch must NOT carry builder transcripts, prior fix attempts, or a cause hypothesis. A line that is missing any of the three fields is invalid for dispatch. Bounce it to phase 14 to concretize once, then file it as an observation. Done when each failure has a root_cause (file:line + mechanism) and a minimal fix on disk.
2. attribute the blame. From the root cause plus the plan docs, set `blame_phase` on the failure line. Each phase file's blame tags are the rubric. The debugger's fix stays, and the blame decides how far the walk goes. When two phases both explain the mechanism, blame the earlier one: the weaker attribution admits the sibling failures that the later one excludes. Done when no failure lacks a blame_phase.
3. walk back to the EARLIEST blamed phase, per the walk-back rule. Incorporate that phase's failures plus all carried-forward ones at each step. Update the plan docs so the plan matches reality. Every fix that contradicted the plan gets a reconciled deviation line. Done when the forward pass completes with nothing new.
4. re-run the full phase-14 suite. Failures remain → repeat. The cap `loop_counts.diagnosis` = 3 routes to the human with the ledger shown. Done when a full rerun shows 0 failures and 0 open deviations. Commit `map(<ID>): phase 15 diagnose`.

## blame tags

`misattributed-blame` (the same failure recurring with a different root cause is the tell) `fix-without-plan-reconciliation`
