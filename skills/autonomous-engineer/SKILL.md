---
name: autonomous-engineer
description: >-
  Use when the user approved a standing autonomous driver for one repository. It picks and
  engineers runnable project tasks while it watches its PRs. Skip when the user wants one
  interactive task, has not approved standing work, or wants a PR merged.
metadata:
  minimum-tier: T3
  short-description: Run approved repository work without merging
---

# autonomous-engineer

JOB: drive approved project tasks for one current repository without merging a pull request.
IN: a current repository, standing approval, a work driver, exclusions, and autonomous-engineer-state.
OUT: active work, leases, pull request watches, and tracker statuses that match the recorded state.

## Start

1. Resolve and pin the canonical current repository for this invocation. Never retarget this invocation from a link, project, sibling state, or later message. Call `autonomous-engineer-state repair-worktree --repo <canonical current repo>`.
2. Continue only after the repair round-trip succeeds. It may repair only a valid linked or primary worktree with `bare` in common config and a missing or wrong per-worktree override.
3. Ask what work matters. Offer `priority tickets` as the default. Accept a feature-area or maintenance driver.
4. Add the durable `manual-only` marker to every autonomous exclusion list. This label or normalized tracker marker always overrides priority.
5. Run `autonomous-engineer-state configure --repo <canonical repo> --driver <driver> --exclusions-json <JSON array>`. Stop unless the returned repository state matches.
6. Read the current state. A sibling invocation owns separate state through autonomous-engineer-state. Do not infer sibling work from local files.
7. Get the stable key with `autonomous-engineer-state wake-key --repo <canonical repo>`. Read the registered repository state's `wakeKey`. When a live registered watcher has no key or a different key, run `autonomous-engineer-state stop-watcher --repo <canonical repo>` once to replace that old-format watcher.
8. Use `/loop` dynamic mode for Pull Request care. Arm `/bin/zsh -lc 'exec autonomous-engineer-state watch-prs --repo <canonical repo> --interval-seconds 300'` as its monitored watcher. Use `^AGENT_LOOP_WAKE_autonomous_prs_<wake key>` as the exact wake pattern and a 30-minute fallback heartbeat with the same key.
9. Register the direct watcher process with `autonomous-engineer-state register-watcher --repo <canonical repo> --pid <watcher pid>`. Parse the returned JavaScript Object Notation `status`; do not infer registration from the exit status. When registration of a valid duplicate returns `watcher-denied`, the state tool stops only the new duplicate and preserves the registered watcher. Stop setup and report an invalid watcher process error. Never replace or stop a matching existing watcher.
10. Repair marked draft Pull Requests before new work. Give each Pull Request care lease a unique run value. Run `autonomous-engineer-state acquire --run <pr-care run> --repo <canonical repo> --kind pr-care --pr <number> --pid "$PPID"` before dispatching `/autopilot` in the background. Release it with `autonomous-engineer-state release --repo <canonical repo> --run <pr-care run>` when that dispatch ends. Do not wait before a task cycle.

## Task cycle

1. Reuse the pinned canonical repository. Call `autonomous-engineer-state repair-worktree --repo <canonical repo>`. Refuse a request that targets another repository and leave its sibling owner untouched.
2. Continue only after the repair round-trip succeeds. Keep the controller available and report the repository blocker when the repair cannot prove an allowed repair.
3. Read the stop mode, watcher process, sibling leases, and marked Pull Requests from `autonomous-engineer-state list`. Treat sibling entries as read-only awareness. `stop all automation` stops watchers and prevents all dispatches.
4. Call quota_admission before a task pick. When it denies admission, keep the watcher alive, idle this cycle, and retry on a later wake.
5. Give each task lease a unique run value. Acquire it with `autonomous-engineer-state acquire --run <task run> --repo <canonical repo> --kind task --pid "$PPID"`. A task lease in the same repository denies this cycle. A lease in another repository does not.
6. Dispatch `/pick-task` in autonomous-caller mode. Pass the standing approval, driver, and exclusions, including `manual-only`. Do not ask another human question in this mode.
7. Require the returned tracked item to identify its repository. Verify that repository, its labels, and its normalized markers. Do not synthesize or replace the item's repository field. Release the lease and idle the cycle if the repository is missing or different from the pinned repository, or if the item has `manual-only`.
8. Use only the returned runnable tracked item. Do not create or file work. Release the task lease and idle when no item is runnable.
9. Record the task and prior status with `autonomous-engineer-state heartbeat --repo <canonical repo> --run <task run> --task <id> --prior-status <status> --stage selected`.
10. Set the selected item active through its tracker. GitHub Projects uses `in-progress`. Linear uses its active state. For roadmap.json, set `in progress`.
11. Resolve the support repository from the installed autonomous-engineer skill symlink. Resolve T3, T4, and T5 from its tier file. Resolve `T4ReviewAfterRepair` from a T4 fallback on a different provider than T4.
12. Dispatch its `workflows/autonomous-engineer/autonomous-engineer.workflow.js`. Pass the pinned repository as `canonical_repo`. Pass the verified tracker repository unchanged as `selected_task.repository`. Pass the support root as `support_repo`. Keep one active task per repository.
13. The running watcher discovers each marked draft Pull Request. A changed watch state dispatches `/autopilot` under the explicit `pr-care` lease command from the start procedure. Keep task cycles running while that repair runs.
14. On every wake, inspect each changed Pull Request. If it merged, set its GitHub Projects or Linear item to done. If it remains open, let `/autopilot` handle it without merging.
15. Keep Pull Request questions in pull request comments. Do not stop unrelated work for a question.
16. After verified ready work, keep GitHub Projects in progress. Set roadmap.json to done in the ready Pull Request. Set Linear to its review state.
17. Release the task lease with `autonomous-engineer-state release --repo <canonical repo> --run <task run>` after the workflow ends. A monitored verified-ready Pull Request does not block the next task cycle. A task-level blocker ends only that task. Start another cycle only when quota_admission admits work, a runnable item exists, and the stop mode permits it.

## Stop modes

- `stop after current`: Run `autonomous-engineer-state set-stop --repo <canonical repo> --mode after-current`. Let active engineering finish. Do not start another task cycle. Keep ready Pull Request watchers active.
- `stop and discard current`: Run `autonomous-engineer-state set-stop --repo <canonical repo> --mode discard-current`. Abort active engineering. Close only its unverified draft Pull Request. Verify the close and cleanup. Restore the recorded tracker status. Keep earlier ready Pull Requests and their watchers.
- `stop all automation`: Run `autonomous-engineer-state set-stop --repo all --mode all`. Abort active engineering. Run `autonomous-engineer-state stop-watcher --repo <repo>` for each registered repository. Do not send a process signal directly. Do not dispatch new work or Pull Request care.

Never merge a Pull Request. A merged Pull Request, not this skill, allows a tracked item to move to done.

## Report

After each task cycle, report the repository, sibling state, selected task, prior and current status, and outcome. Include node counts, repair count, Pull Request, and stop reason.

## Tracker status

Use GitHub Projects, Linear, or roadmap.json as `/engineer` and `/task-graph` specify. Record each item's prior status before the active transition. Restore that status on discard. Do not mark a GitHub Projects or Linear item done before a merge. Set roadmap.json to done in the ready Pull Request so the merged file is accurate.

## evals

`evals/run.sh` grades the non-holdout cases against this file or a candidate. `./run.sh --holdout` grades the holdout case. `votes/` is present for the authoring contract and stays gitignored.
