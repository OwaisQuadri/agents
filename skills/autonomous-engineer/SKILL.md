---
name: autonomous-engineer
description: >-
  Use when the user approved a standing autonomous driver for one repository. It picks and
  engineers runnable backend work while it watches its PRs. Skip when the user wants one
  interactive task, has not approved standing work, or wants a PR merged.
metadata:
  minimum-tier: T3
  short-description: Run approved repository work without merging
---

# autonomous-engineer

JOB: drive approved backend work for one current repository without merging a pull request.
IN: a current repository, standing approval, a work driver, exclusions, and autonomous-engineer-state.
OUT: active work, leases, pull request watches, and backend statuses that match the recorded state.

## Start

1. Resolve the canonical current repository. Call `autonomous-engineer-state repair-worktree --repo <canonical current repo>`.
2. Continue only after the repair round-trip succeeds. It may repair only a valid linked or primary worktree with `bare` in common config and a missing or wrong per-worktree override.
3. Ask what work matters. Offer `priority tickets` as the default. Accept a feature-area or maintenance driver.
4. Record the approved driver and exclusions in autonomous-engineer-state. Stop when the state records the current repository and driver.
5. Read the current state. A sibling invocation owns separate state through autonomous-engineer-state. Do not infer sibling work from local files.
6. Start watch-prs for every marked open or draft pull request. Start it from draft creation. Keep it until closure.
7. Repair marked draft pull requests before new work. Acquire a `pr-care` lease before dispatching `/autopilot` in the background. Do not wait for that dispatch before a task cycle.

## Task cycle

1. Resolve the canonical current repository. Call `autonomous-engineer-state repair-worktree --repo <canonical current repo>`.
2. Continue only after the repair round-trip succeeds. Stop the cycle when the repair cannot prove an allowed repair.
3. Read the stop mode and the marked pull requests from autonomous-engineer-state. `stop all automation` stops watch-prs and prevents all dispatches.
4. Call quota_admission before a task pick. The quota admission limits three active model workers globally. Stop this cycle when it denies admission.
5. Acquire a task lease for this repository. Stop this cycle when a sibling owns the lease.
6. Dispatch `/pick-task` in autonomous-caller mode. Pass the standing approval, driver, and exclusions. Do not ask another human question in this mode.
7. Use only the returned runnable backend item. Do not create or file work. Release the task lease when no item is runnable.
8. Set the selected item active through its backend. GitHub Projects uses `in-progress`. Linear uses its active state. For roadmap.json, set `in progress`.
9. Dispatch `workflows/autonomous-engineer/autonomous-engineer.workflow.js` for the selected issue. Keep one active task per repository.
10. Register each draft pull request with watch-prs when the workflow creates it. A changed watch state dispatches `/autopilot` under a `pr-care` lease. Keep task cycles running while that repair runs.
11. Keep pull request questions in pull request comments. Do not stop unrelated work for a question.
12. After verified ready work, set GitHub Projects and roadmap.json to `resolved`. Set Linear to its review state. Set done only after the pull request merges.
13. Release the task lease after the workflow ends. Start another cycle only when quota_admission admits work, a runnable item exists, and the stop mode permits it.

## Stop modes

- `stop after current`: Let active engineering finish. Do not start another task cycle. Keep ready-pull-request watchers active.
- `stop and discard current`: Abort active engineering. Close only its unverified draft pull request. Verify the close and cleanup. Restore the recorded backend status. Keep earlier ready pull requests and their watchers.
- `stop all automation`: Abort active engineering and stop every watcher. Do not dispatch new work or pull request care.

Never merge a pull request. A merged pull request, not this skill, sets a backend item to done.

## Backend status

Use GitHub Projects, Linear, or roadmap.json as `/engineer` and `/task-graph` specify. Record each item's prior status before the active transition. Restore that status on discard. Do not mark an item done before a merge.

## evals

`evals/run.sh` grades the non-holdout cases against this file or a candidate. `./run.sh --holdout` grades the holdout case. `votes/` is present for the authoring contract and stays gitignored.
