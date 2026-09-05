---
name: pick-task
description: >-
  Use when you need to land on the next unit of work — nothing is queued up, or several
  things compete, or engineer was invoked with no ticket. Grills you about what's needed
  right now, steers between the existing backlog and something that needs fresh ideation,
  and hands back one chosen task. Skip when a ticket id or a clear task is already named
  (go straight to engineer), and skip for brainstorming future work with nothing urgent
  driving it (that's /ideate).
metadata:
  minimum-tier: T3
  short-description: Interactively grill and land on the next task
---

# pick-task

JOB: Select one task for immediate work through questions instead of a queue.
IN: The user can give a hint, a caller request, or autonomous approval with a driver and exclusions.
OUT: Return one existing task identifier with details, or one small new description, and the reason for the choice.

## autonomous-caller mode

Only an approved standing driver can use this mode. The caller supplies autonomous approval, the exact driver, and the exclusions. Reuse that driver.

Add `manual-only` to the exclusions. Select one existing runnable backend item. Never invent or create a tracked task. Return the pick without human confirmation.

Treat the exact driver `priority tickets` as complete. Use `skills/task-graph/scripts/next-issue.sh` when GitHub Issues is the backend. Do not replace its ranking with an issue-list guess.

Only autonomous-caller mode repairs the exact `missing project Status on issue(s):` failure. Capture the selector's standard output and standard error. Parse every named issue number from standard error.

Read each issue with `gh issue view <number> --json state,stateReason,labels,closedByPullRequestsReferences`. Return no item when a named issue has `manual-only`. Do not mutate that issue. Name that blocker.

Map an `OPEN` issue to `todo`. Map a `CLOSED` and `NOT_PLANNED` issue to `cancelled`. A `CLOSED` and `COMPLETED` issue requires a closing Pull Request. Read each closing reference's number and repository fields. Verify each reference with `gh pr view <pr-number> -R <owner>/<repository> --json mergedAt --jq .mergedAt`. Map the issue to `done` when at least one reference has a nonempty `mergedAt`.

Return no item when a closed issue has no safe mapping. Name that blocker. Use `completed issue #<number> lacks connected merged-Pull-Request evidence` when no verified merged reference exists.

Run `skills/task-graph/scripts/gh-issue-field.sh <number> Status <mapped-status>` for each safe mapping. This command can add the existing issue to its linked project. Require every command and its built-in read-back check to pass. Rerun `next-issue.sh` once after all repairs pass. Continue with the returned issue.

Return no item when a repair or the retry fails. Name the exact blocker.

Check the labels on the returned issue. Return no item when the issue has `manual-only`.

For another backend, use its existing runnable-item ranking. Reject an item that has the same normalized marker.

## why interrogation, not a queue read

Tasks can be large because velocity matters more than atomicity. The right pick depends on the current need. The backlog gives context, not the answer.

## steps

1. **Ask about the current need.** Skip this step in autonomous-caller mode. Ask one direct question about the desired outcome. Ask one more question when the first answer is vague. Done when you can state the desired outcome in one sentence.

2. **Find the connected task system.** Check `gh issue list` and `gh project list` first. Then check the configured Linear MCP(Model Context Protocol) tools. Use either system when it contains live items. Otherwise, use the root `roadmap.json` file. Done when you identify the backend that holds the real list.

   Use `skills/task-graph/scripts/next-issue.sh` when GitHub Issues follows the `task-graph` conventions. These conventions use native Status, Priority, and `blockedBy` fields. Do not rank that issue list by hand. Outside autonomous-caller mode, report selector failures without changing tracker data.

3. **Surface two to four candidates.** Draw them from the selected backend. Filter them against the outcome from step 1. Give one line that states why each candidate fits. Always include a "none of these" option for new work.

4. **Grill the user about the pick.** Ask whether one candidate fits or needs a different scope. Say when no candidate matches the stated need. Offer `/ideate` instead of forcing a poor fit.

5. **Land on one task.** Give its identifier when it is a backend item. Give a short description when it is small new work. State why it matches the outcome from step 1.

6. **Return the task.** Return the chosen task to the caller. For standalone use, report the pick without starting engineer. Start engineer only when the user asks.

## backend notes

- Recommend GitHub Projects when a project has no task backend. Recommend Linear when the team already uses it. State the recommendation and ask the user to confirm.
- Send every new backend item through the filing gate in `/ideate`. Pick-task selects work and never creates a tracked task.

## evals

`evals/run.sh` grades non-holdout cases in `evals/cases.jsonl` against this file or a candidate. It uses `evals/rubric.md`. The `--holdout` option runs the held-out slice.
