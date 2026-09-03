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
4. Add the durable `manual-only` marker to every autonomous exclusion list. This label or normalized backend marker always overrides priority.
5. Run `autonomous-engineer-state configure --repo <canonical repo> --driver <driver> --exclusions-json <JSON array>`. Stop unless the returned repository state matches.
6. Read the current state. A sibling invocation owns separate state through autonomous-engineer-state. Do not infer sibling work from local files.
7. Use `/loop` dynamic mode for Pull Request care. Arm `autonomous-engineer-state watch-prs --repo <canonical repo> --interval-seconds 300` as its monitored watcher. Use `^AGENT_LOOP_WAKE_autonomous_prs` as the wake pattern and a 30-minute fallback heartbeat.
8. Register the watcher process with `autonomous-engineer-state register-watcher --repo <canonical repo> --pid <watcher pid>`. Do not arm a duplicate watcher.
9. Repair marked draft Pull Requests before new work. Run `autonomous-engineer-state acquire --run <run> --repo <canonical repo> --kind pr-care --pr <number> --pid "$PPID"` before dispatching `/autopilot` in the background. Release it when that dispatch ends. Do not wait before a task cycle.

## Task cycle

1. Resolve the canonical current repository. Call `autonomous-engineer-state repair-worktree --repo <canonical current repo>`.
2. Continue only after the repair round-trip succeeds. Stop the cycle when the repair cannot prove an allowed repair.
3. Read the stop mode, watcher process, sibling leases, and marked Pull Requests from `autonomous-engineer-state list`. `stop all automation` stops watchers and prevents all dispatches.
4. Call quota_admission before a task pick. Stop this cycle when it denies admission.
5. Acquire a task lease with `autonomous-engineer-state acquire --run <run> --repo <canonical repo> --kind task --pid "$PPID"`. The state tool limits three active model workers globally. Stop this cycle when a sibling owns the lease.
6. Dispatch `/pick-task` in autonomous-caller mode. Pass the standing approval, driver, and exclusions, including `manual-only`. Do not ask another human question in this mode.
7. Verify the returned item's labels or normalized markers. Release the lease and stop the cycle if it has `manual-only`.
8. Use only the returned runnable backend item. Do not create or file work. Release the task lease when no item is runnable.
9. Record the task and prior status with `autonomous-engineer-state heartbeat --run <run> --task <id> --prior-status <status> --stage selected`.
10. Set the selected item active through its backend. GitHub Projects uses `in-progress`. Linear uses its active state. For roadmap.json, set `in progress`.
11. Resolve the support repository from the installed autonomous-engineer skill symlink. Dispatch its `workflows/autonomous-engineer/autonomous-engineer.workflow.js` and pass that root as `support_repo`. Keep one active task per repository.
12. The running watcher discovers each marked draft Pull Request. A changed watch state dispatches `/autopilot` under the explicit `pr-care` lease command from the start procedure. Keep task cycles running while that repair runs.
13. On every wake, inspect each changed Pull Request. If it merged, set its backend item to done. If it remains open, let `/autopilot` handle it without merging.
14. Keep Pull Request questions in pull request comments. Do not stop unrelated work for a question.
15. After verified ready work, set GitHub Projects and roadmap.json to `resolved`. Set Linear to its review state. Set done only after the pull request merges.
16. Release the task lease after the workflow ends. Start another cycle only when quota_admission admits work, a runnable item exists, and the stop mode permits it.

## Stop modes

- `stop after current`: Run `autonomous-engineer-state set-stop --repo <canonical repo> --mode after-current`. Let active engineering finish. Do not start another task cycle. Keep ready Pull Request watchers active.
- `stop and discard current`: Run `autonomous-engineer-state set-stop --repo <canonical repo> --mode discard-current`. Abort active engineering. Close only its unverified draft Pull Request. Verify the close and cleanup. Restore the recorded backend status. Keep earlier ready Pull Requests and their watchers.
- `stop all automation`: Run `autonomous-engineer-state set-stop --repo all --mode all`. Abort active engineering. Stop each registered watcher process, then unregister it. Do not dispatch new work or Pull Request care.

Never merge a Pull Request. A merged Pull Request, not this skill, allows a backend item to move to done.

## Report

After each task cycle, report the repository, sibling state, selected task, prior and current status, and outcome. Include node counts, repair count, Pull Request, and stop reason.

## Backend status

Use GitHub Projects, Linear, or roadmap.json as `/engineer` and `/task-graph` specify. Record each item's prior status before the active transition. Restore that status on discard. Do not mark an item done before a merge.

## evals

`evals/run.sh` grades the non-holdout cases against this file or a candidate. `./run.sh --holdout` grades the holdout case. `votes/` is present for the authoring contract and stays gitignored.
