# phase 19 — break-panel

JOB: five fresh testers, angles picked for THIS ticket, trying to prove the builder wrong — including breaking parts the ticket never touched
IN:  testability.md, the ticket's short + long, the built app; phase 18 done
OUT: `.map/<ID>/panel.md` (angle selection + per-angle verdicts); failures.jsonl delta

## the angle library

Pick exactly 5 angles for this ticket. The angles change with the task. The count does not.
Record the 5 picks in panel.md before you dispatch. Write one line of why for each pick.

The default 5 partition the failure space, one angle per region:

- `hostile-input` — input
- `state-machine-abuse` — transitions. It drives the data-only engine from phase 04 directly
- `regression-sweep` — blast radius. It exercises every feature the ticket did NOT touch.
  Build its feature inventory from the README and the ticket titles in roadmap.json
- `concurrency-timing` — time
- `resource-persistence` — environment. It kills and relaunches the app, it corrupts
  storage, it fills storage, and it goes offline

Swap in another angle when the ticket's surface warrants it:

- `auth-permissions`
- `offline-network`
- `data-migration` — the new code opens data written by an old version
- `accessibility-input-modes`
- `locale-formats`

A security-relevant surface always keeps `hostile-input` or `auth-permissions`. Never skimp
on security.

## the graph

```
workflow

GOAL:     break the changed app; zero surviving unexplained failures
FAN OUT:  5 FRESH spec-testers (mode break), one per selected angle; each
          carries its angle charter, the drive matrix, the ticket summary, and
          a scratch dir — never the diff, transcripts, sibling findings, or the
          phase-10 cases (a breaker re-running the suite is not attacking)
MERGE:    the orchestrator appends candidate failure lines; counts 5 returned
          vs 5
VERIFY:   fresh anchor-verifier per candidate failure — work_product_paths =
          failures.jsonl, verify_command = the line's repro_command, rubric =
          [the command reproduces the reported actual]; unreproduced → bounced
          once, then observation
RULE:     every failure line carries repro_command + expected + actual;
          breakage in an untouched feature IS a failure (is_regression true)
CAP:      5 testers + 5 verifiers + 1 bounce round
ON FAIL:  a silent tester is named in panel.md and its angle rerun once —
          never skipped
REPORT:   failures.jsonl delta + per-angle verdicts + returned vs expected
```

Anchors: re-executed repro commands; maestro junit reports on disk for mobile angles.

## exit

Any verified failure → set state.walk and route through phase 15 (its loop; `loop_counts.panel` cap 3 → human). After the fixes land, the PANEL runs again — hardened code gets re-attacked, not just re-tested. Leave only when a full panel round returns all 5 angles with zero verified failures and the phase-14 suite reruns green. Commit `map(<ID>): phase 19 break-panel`.

## blame tags

`panel-miss` `angle-not-run` `unrelated-breakage-waved-through`
