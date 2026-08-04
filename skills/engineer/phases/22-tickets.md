# phase 22 — tickets

JOB: the selected candidates filed as ABCD-NNNN tickets through the same planner as phase 11, with the refreshed roadmap diagram shown
IN:  exploration.md (or the ideation file), roadmap.json; phase 21 committed
OUT: roadmap.json (tickets + counter), `.map/roadmap.mmd` regenerated and SHOWN

## steps

1. select what to file — taste from inspiration.md ordering; when unsure, file rather than drop: cancelled is cheap, forgotten is not. Done when the selection is named.
2. invoke /task-graph (ticket shape): ids `<prefix>-<NNNN>` from next_nnnn — zero-padded 4, creation order, never reused, never reordered on completion; deps referencing ticket ids; coarse `files` areas; status todo. SINGLE WRITER: this step is never fanned out; only it increments next_nnnn. Done when every id passes `^[A-Z]{2,4}-[0-9]{4}$`, uniqueness holds, and `next_nnnn == max(NNNN)+1`.
3. the /task-graph run regenerates `.map/roadmap.mmd` — SHOW the updated diagram; ticket deps must be acyclic (same validation as tasks). Done when the diagram is rendered and shown.
4. commit `map(<ID>): phase 22 tickets` (plain commit in ideate mode; ideate stops here).

## blame tags

`duplicate-NNNN` `counter-drift` `ticket-dep-cycle` `reused-cancelled-id`
