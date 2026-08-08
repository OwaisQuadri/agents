# phase 14 — agent-test

JOB: every natural-language case executed by a fresh agent through the drive matrix; failures collected debugger-ready, not fixed
IN:  test-cases.md, testability.md, the built app; phase 13 committed, 0 open deviations
OUT: `.map/<ID>/failures.jsonl`; deviations.jsonl updated on walk-backs

## the graph

```
workflow

GOAL:     every case executed fresh; failures collected debugger-ready
FAN OUT:  one spec-tester (mode confirm) per tag group — happy, edge, security.
          Each one carries ONLY its case subset, the drive matrix, and a scratch
          dir. Mobile UI(user interface) cases dispatch maestro-tester (app_id,
          flow_objective, flows_dir) instead
MERGE:    the orchestrator appends failure lines to failures.jsonl and counts
          returned vs expected groups
VERIFY:   the orchestrator re-executes each failure's repro_command before
          acceptance. A failure that does not reproduce bounces to its tester
          once, then files as an observation
RULE:     a failure line MUST carry repro_command + expected (from the case's
          expect) + actual (pasted output). A tester never receives the diff,
          the builder transcripts, or tasks.json
CAP:      3 testers per wave + one rerun each; 3 walk-back cycles then human
ON FAIL:  a tester returning nothing is named, its group rerun once, never
          skipped silently
REPORT:   failures.jsonl delta + per-group pass/fail counts
```

Anchors: executed drive commands quoted in every verdict; re-executed repros.

## missing vs broken

The grep is mechanical. Run `grep -rn "TODO(<TICKET>" <implicated area>`. A hit means the functionality is MISSING. No marker means the code is BROKEN, so write a failure line.

The grep never decides the blame by itself. Check the observed behavior against the plan docs first. When the behavior contradicts interfaces.md, data-structures.md, or a `test-cases.md` expect, blame the phase that owns that doc: 07 for a contract, 06 for a shape, 10 for a case. Blame the marker's task-phase only when the plan is right and the work is unbuilt. A marker at the site never moves the blame off an earlier phase whose output was wrong.

Write the deviation line, then walk back until 0 deviations remain, then rerun.

## exit

One FULL all-cases run is completed, and every failure is captured as a reproducing line. The phase ends with the failures COLLECTED. Fixing one here is the named catastrophic, fix-at-discovery. Commit `map(<ID>): phase 14 agent-test`.

## blame tags

`false-pass` `failure-without-repro` `case-skipped-silently` `blame-stopped-at-marker`
