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

JOB: land on ONE task to work on right now, through interrogation rather than a queue
IN:  nothing required — an optional hint ("something for the statusline", "whatever's
     blocking the release"), or a caller (engineer) asking for a task before it can start
OUT: one chosen task — either an existing backlog id (with its short/long) or a fresh
     one-off description — handed back to the caller, plus the reasoning for the pick

## why interrogation, not a queue read

Tasks here are allowed to be big — velocity matters more than atomicity. That means the
right pick is rarely "top of the list": it depends on what's actually pressing right now,
which only you know. Read the backlog for context, never as the answer.

## steps

1. **Ask what's driving this.** One or two direct questions: what's prompting a task
   right now (a bug just hit, a release is coming, idle time to spend, a specific itch)?
   Don't accept a vague answer — press once more if the first answer is "whatever's
   next" or similarly non-committal. Done when you can state, in one sentence, what
   outcome the user actually wants from this session.

2. **Check for a connected task system.** In order: `gh issue list` and `gh project
   list` (GitHub), then any configured Linear MCP(Model Context Protocol) tools. If
   either exists and holds live items, that's the system of record — read it instead of
   (or alongside) `roadmap.json`. If none exists, root `roadmap.json` is the backlog.
   Done when you know which backend holds the real list. When the backend is GitHub
   Issues with `task-graph`'s conventions in place (native project Status/Priority
   fields, native `blockedBy`), `skills/task-graph/scripts/next-issue.sh` ranks
   the backlog the same way `next-ticket.sh` ranks roadmap.json — reach for it instead
   of eyeballing the issue list by hand.

3. **Surface 2-4 candidates**, drawn from that backend and filtered by what step 1
   surfaced — never the raw unfiltered list. For each: one line on why it fits (or
   doesn't) what the user said they want. Include a "none of these — something new"
   option always.

4. **Grill on the pick.** Ask directly: does one of these match, should it be scoped
   bigger or smaller, or does none of them fit? Steer, don't just present — if the
   user's stated driver (step 1) doesn't match anything in the backend, say so and offer
   to hand off to `/ideate` rather than forcing a fit.

5. **Land.** One task, named plainly: its id (if it's a backend item) or a fresh
   one-off description (if net-new and small enough not to need `/ideate`'s fuller
   brainstorm). State why this one, in the user's own stated terms from step 1.

6. **Hand off.** Return the chosen task to the caller. If pick-task was invoked
   standalone (not by engineer), just report the pick — don't auto-start engineer
   unless asked.

## backend notes

- GitHub Projects is the default recommendation when a project needs to pick a backend
  and none exists yet; Linear when the team already runs on it. Don't decide this
  silently — name the recommendation and let the user confirm.
- Writing a NEW item into any backend (GitHub, Linear, or `roadmap.json`) always passes
  through `/ideate`'s filing gate — pick-task selects, it never files.

## evals

`evals/run.sh` grades every non-holdout case in `evals/cases.jsonl` against this file or
a candidate, using `evals/rubric.md`; `--holdout` runs the held-out slice.
