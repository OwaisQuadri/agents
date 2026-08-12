# phase 12 — todos

JOB: every change site marked, every command captured runnable, and the stash-anchor commit recorded
IN:  tasks.json, env.md; phase 11 committed (GATE B passed)
OUT: TODO markers in-repo; `.map/<ID>/todo.sh`; state.json phase_commits["12"] set

paths: the union of all tasks' `files` plus `.map/<ID>/todo.sh`. Write markers only, and no logic edits. DO NOT BUILD. DO NOT TEST.

## steps

1. place `TODO(<task-id>): <reason>` at every change site of every code task. The reason runs to 2 lines at most. This is the whitelisted deliberate-TODO comment shape, and it is the ONLY comment shape this phase writes. Done when every code task with non-empty files greps ≥1 `TODO(<id>)`.
2. write todo.sh from `templates/todo.sh`. The FIX lines from env.md become the lowest-numbered steps. Each `kind: command` task becomes one step after them. Every step is guard-first and idempotent, because a walk-back re-runs it. The `list` function prints `stepN task-id` pairs, so the cross-reference is data and not comments. Done when `sh -n todo.sh` passes and `./todo.sh list` covers every command task.
3. HUMAN GATE F. Present the CHANGE-SITE INVENTORY before the build fans out. Three parts, and none of them is a file path list on its own. (a) every marker as `<task-id> → <file>:<line>` with its reason, grouped by file, so the user reads where each task will cut and can say "that is the wrong place" while it is still one line to move. (b) todo.sh in full, verbatim, because its steps RUN on his machine — phase 05's FIX lines install things — and a script nobody read is a script nobody approved. (c) the count of files this ticket will touch, naming any that a live system executes. Wait for the verdict; `state.json.gates.F` records it in his words. Done when the user has seen all three and the entry exists.
4. commit `map(<ID>): phase 12 todos`. Record the commit SHA in `state.json.phase_commits["12"]`. Phases 16 to 18 reset to this anchor and verify against it. Done when the SHA is recorded.

## blame tags

`marker-missing` `marker-misplaced` `todo-step-broken`
