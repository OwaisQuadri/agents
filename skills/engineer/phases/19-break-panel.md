# phase 19 — break-panel

JOB: five fresh testers, angles picked for THIS ticket, trying to prove the builder wrong — including breaking parts the ticket never touched
IN:  testability.md, the ticket's short + long, the built app; phase 18 done
OUT: `.map/<ID>/panel.md` (angle selection + per-angle verdicts); failures.jsonl delta

## the angle library

Pick exactly 5 for this ticket — the angles change with the task, the count does not. Record the pick and one line of why per angle in panel.md before dispatching. Defaults that partition the failure space (input / transitions / blast radius / time / environment): `hostile-input`, `state-machine-abuse` (drives the phase-04 data-only engine directly), `regression-sweep` (exercises everything the ticket did NOT touch; gets the app's feature inventory — built from the README plus roadmap.json ticket titles), `concurrency-timing`, `resource-persistence` (kill/relaunch, corrupt or full storage, offline). Swap-ins when the ticket's surface warrants: `auth-permissions`, `offline-network`, `data-migration` (old-version data opened by the new code), `accessibility-input-modes`, `locale-formats`. A security-relevant surface always keeps at least one of hostile-input or auth-permissions — security is never skimped.

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
