# phase 14 — agent-test

JOB: every natural-language case executed by a fresh agent through the drive matrix; failures collected debugger-ready, not fixed
IN:  test-cases.md, testability.md, the built app; phase 13 committed, 0 open deviations
OUT: `.map/<ID>/failures.jsonl`; deviations.jsonl updated on walk-backs

## the graph

```
workflow

GOAL:     every case executed fresh; failures collected debugger-ready
FAN OUT:  one spec-tester (mode confirm) per tag group — happy, edge, security —
          each carrying ONLY its case subset, the drive matrix, and a scratch
          dir; mobile UI(user interface) cases dispatch maestro-tester (app_id,
          flow_objective, flows_dir) instead
MERGE:    the orchestrator appends failure lines to failures.jsonl and counts
          returned vs expected groups
VERIFY:   each failure's repro_command re-executed by the orchestrator before
          acceptance; a failure that does not reproduce bounces to its tester
          once, then files as an observation
RULE:     a failure line MUST carry repro_command + expected (from the case's
          expect) + actual (pasted output); testers never receive the diff,
          builder transcripts, or tasks.json
CAP:      3 testers per wave + one rerun each; 3 walk-back cycles then human
ON FAIL:  a tester returning nothing is named, its group rerun once, never
          skipped silently
REPORT:   failures.jsonl delta + per-group pass/fail counts
```

Anchors: executed drive commands quoted in every verdict; re-executed repros.

## missing vs broken

Mechanical: `grep -rn "TODO(<TICKET>" <implicated area>` hits → MISSING functionality → a deviation line blaming the marker's task-phase → walk-back until 0 deviations, then rerun. No marker → BROKEN → failure line.

## exit

One FULL all-cases run completed, every failure captured as a reproducing line. The phase ends with failures COLLECTED — fixing here is the named catastrophic (fix-at-discovery). Commit `map(<ID>): phase 14 agent-test`.

## blame tags

`false-pass` `failure-without-repro` `case-skipped-silently`
