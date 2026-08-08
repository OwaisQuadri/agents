# phase 04 — testability

JOB: a drive matrix proving the agent can perform every user action programmatically, with gaps planned as ordinary tasks — planned this early on purpose
IN:  ux.md, the repo; phase 03 committed
OUT: `.map/<ID>/testability.md` — the drive matrix + planned harness tasks

## steps

1. enumerate every user action this ticket adds or touches, from ux.md and from the ticket. Done when the action list is exhaustive against ux.md.
2. audit what exists per layer. The layers separate by concern:
   (a) the SUT(system under test) as a data-only process: an engine or state machine that runs headless, with no UI(user interface) attached.
   (b) the UI driven through programmatic inputs.
   (c) real tap, click, and drag. These are reserved for the integration tests that validate the gesture components themselves. Nothing else uses them.
   Done when each layer's existing entry points are listed with paths.
3. write the drive matrix with the columns `| user action | layer | drive command | exists (yes/no) |`. Every action gets a drive command, existing or planned. An action with no possible programmatic drive is a design smell. Report that action to the user. Done when zero actions lack a row.
4. turn every `exists: no` row into a planned harness task. These tasks enter the breakdown in phase 08 as ordinary tasks. Done when the planned-task list mirrors the no-rows. Commit `map(<ID>): phase 04 testability`.

## blame tags

`undrivable-behavior` `wrong-layer-test` `harness-gap-found-late`
