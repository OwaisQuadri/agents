# phase 22 — tickets

JOB: the selected candidates filed as ABCD-NNNN tickets through the same planner as phase 11, with the refreshed roadmap shown through /show-me
IN:  exploration.md (or the ideation file), roadmap.json; phase 21 committed
OUT: roadmap.json (tickets + counter), with its roadmap view SHOWN through /show-me

## steps

1. select what to file. The ordering taste comes from inspiration.md. When unsure, propose rather than drop: cancelled is cheap, forgotten is not. Done when the selection is named.
2. HUMAN GATE D. Apply STANDING APPROVAL from SKILL.md over the complete candidate list, both dependency directions, and ordering. Write nothing to roadmap.json before this gate. Present the CANDIDATE list as one line each (`short` + the one-line value + deps BOTH WAYS — what the candidate depends on, AND which already-filed tickets it would block), grouped by the buckets from exploration.md, and STOP. The blocking direction is the half the human cannot reconstruct from the candidate list alone, and it is the half that changes what they keep. The human keeps, drops, merges, reorders, or edits any candidate. Only the survivors are filed. Rationale: an id is immutable once assigned, because /task-graph refuses renumbering and gap-closing. A filed ticket is therefore permanent, and the set of filed tickets IS the project's next scope. That is the human's call, not the run's. Done when approval carries or the human's keep-or-drop verdict is written to `state.json.gates.D` with its approval number, immutable snapshot, timestamp, and their words.
3. invoke /task-graph in ticket shape, on the SURVIVORS only. The ids are `<prefix>-<NNNN>` drawn from next_nnnn, zero-padded to 4, in creation order. An id is never reused, and never reordered on completion. The deps reference ticket ids and run in BOTH directions, per /task-graph's step 3, which owns that rule and the reason for it. Editing an existing ticket's deps is status-neutral and allowed here; editing its id is not. Require the reverse-direction verdict in the report even when it is empty. The `files` areas stay coarse. The status is todo. SINGLE WRITER: never fan this step out, and let only this step increment next_nnnn. Done when every id passes `^[A-Z]{2,4}-[0-9]{4}$`, uniqueness holds, and `next_nnnn == max(NNNN)+1`.
4. Tell /task-graph to validate and land roadmap.json. Done when the ticket dependencies are acyclic.

   Invoke /show-me with the validated roadmap. Ask for its smallest fitting view. Do not select its output format. Prefer a console-safe view.

   Use each ticket's `short` field as its primary label. Put its identifier beside the label only as a lookup key. An identifier never stands alone. Let /show-me select a UML(Unified Modeling Language) view when it fits the relationship. The view shows each ticket's status, both dependency directions, and the next runnable work. Done when the user sees the view and the view represents every ticket by name.
5. invoke /byline on each filed ticket's `short` and `long`. A ticket is read months later by whoever picks it up, so the prose ships. Facts stay verbatim through the pass. Done when `skills/byline/evals/check.py` returns zero FAIL lines on each one.
6. commit `map(<ID>): phase 22 tickets`, or a plain commit in ideate mode. Ideate stops here, and its gate D is the same one.

A run that files tickets the human never saw has skipped this phase, however valid the ids.

RE-ENTRY: a redo is a resume, a walk-back, or a second ideate pass. On a redo, reconcile against roadmap.json BEFORE filing anything. A candidate whose `short` already names a filed ticket is already filed. Ids are permanent, so a blind redo mints a second id for the same work, and creates exactly the duplicate-scope harm this gate exists to prevent. Done when every survivor is either new or matched to an existing id.

## blame tags

`duplicate-NNNN` `counter-drift` `ticket-dep-cycle` `reused-cancelled-id` `ticket-filed-unreviewed`
