# phase 12 — todos

JOB: every change site marked, every command captured runnable, and the stash-anchor commit recorded
IN:  tasks.json, env.md; phase 11 committed (GATE B passed)
OUT: TODO markers in-repo; `.map/<ID>/todo.sh`; state.json phase_commits["12"] set

paths: the union of all tasks' `files` plus `.map/<ID>/todo.sh`. Markers only — no logic edits, DO NOT BUILD, DO NOT TEST.

## steps

1. at every change site of every code task, place `TODO(<task-id>): <reason>` — reason ≤2 lines, the whitelisted deliberate-TODO comment shape and the ONLY comment shape this phase writes. Done when every code task with non-empty files greps ≥1 `TODO(<id>)`.
2. write todo.sh from `templates/todo.sh`: env.md FIX lines become the lowest-numbered steps, then each `kind: command` task one step; every step guard-first idempotent (walk-backs re-run them); the `list` function prints `stepN task-id` pairs — the cross-reference is data, not comments. Done when `sh -n todo.sh` passes and `./todo.sh list` covers every command task.
3. commit `map(<ID>): phase 12 todos` and record the commit SHA in `state.json.phase_commits["12"]` — this is the anchor phases 16-18 reset to and verify against. Done when the SHA is recorded.

## blame tags

`marker-missing` `marker-misplaced` `todo-step-broken`
