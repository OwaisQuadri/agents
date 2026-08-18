# phase 13 — implement

JOB: every DAG(directed acyclic graph) task resolved with code matching the plan or a disclosed deviation — never a silent fix
IN:  tasks.json, data-structures.md, interfaces.md, todo.sh; phase 12 committed, phase_commits["12"] set
OUT: implementation on the branch; tasks resolved; `.map/<ID>/deviations.jsonl`

## the graph

```
workflow

GOAL:     every task status resolved, with code matching plan or a disclosed
          deviation line
FAN OUT:  one FRESH builder per ready branch (all deps resolved), disjoint files
          only. Each builder carries its task objects, the relevant
          data-structures and interfaces excerpts, its paths glob, and the
          deviation-line shape. A builder never receives another branch's diff
          or the planning transcript
MERGE:    the orchestrator alone writes .map/**. It flips the statuses, appends
          the reported deviations to deviations.jsonl, and runs the todo.sh
          steps a branch names
VERIFY:   anchor-verifier per branch, fresh context: work_product_paths = the
          branch's files, verify_command = build + the branch's targeted tests,
          rubric = [TODO(<id>) markers for the branch gone; zero edits outside
          the glob; booleans is-prefixed]
LOOP:     next wave when the current wave's verifies pass
RULE:     any divergence from plan → a deviation line BEFORE proceeding. A gap
          classifies back to the phase that owns it, and is never patched over
          silently
CAP:      3 concurrent builders; 3 walk-back cycles then human escalation
ON FAIL:  a dead builder's tasks revert to todo and are named in the report
REPORT:   per-task statuses + open-deviation count + verifier verdicts
```

Anchors: the verifier's executed build and test commands; the diff on disk.

## visual report

Before the final report, invoke /show-me with tasks.json, task statuses, and verifier verdicts. Do not select its output format. Use each task's `short` field as its primary label. Put its identifier beside the label only as a lookup key. An identifier never stands alone. Prefer a console-safe Xcode-style lane timeline.

Apply the timing rule per task. Show a measured span only when that task has start and finish timestamps. Put each untimed task in a separate wave lane. Label its duration as unknown. Show parallel lanes, failures, and completed work. Done when the view exposes the implemented parallelism without an invented duration.

## deviation line

`{"id":"D-NN","ts","task","phase":<blamed — the phase whose output was wrong, or 13 when the plan was right and the builder slipped>,"planned","actual","why","status":"open","resolution":null}`

## exit

Open deviations route through the walk-back rule in SKILL.md, to the earliest blamed phase. Leave only when all non-cancelled tasks are resolved, `jq 'select(.status=="open")' deviations.jsonl` counts 0, and the phase is committed as `map(<ID>): phase 13 implement`.

## blame tags

`silent-deviation` `scope-breach` `marker-left-unimplemented`
