# phase 04 — testability

JOB: a drive matrix proving the agent can perform every user action programmatically, with gaps planned as ordinary tasks — planned this early on purpose
IN:  ux.md, the repo; phase 03 committed
OUT: `.map/<ID>/testability.md` — the drive matrix + planned harness tasks

## steps

1. enumerate every user action this ticket adds or touches, from ux.md and the ticket. Done when the action list is exhaustive against ux.md.
2. audit what exists per layer, separated by concerns: (a) SUT(system under test) as a data-only process — an engine or state machine drivable headless, no UI(user interface) attached; (b) UI driven through programmatic inputs; (c) real tap/click/drag reserved for integration tests that validate the gesture components themselves — nothing else uses them. Done when each layer's existing entry points are listed with paths.
3. write the drive matrix: `| user action | layer | drive command | exists (yes/no) |`. Every action gets a drive command, existing or planned; an action with no possible programmatic drive is a design smell reported to the user. Done when zero actions lack a row.
4. every `exists: no` row becomes a planned harness task — these enter phase 08's breakdown as ordinary tasks. Done when the planned-task list mirrors the no-rows. Commit `map(<ID>): phase 04 testability`.

## blame tags

`undrivable-behavior` `wrong-layer-test` `harness-gap-found-late`
