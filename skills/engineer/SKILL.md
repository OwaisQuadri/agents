---
name: engineer
description: >-
  Use when a coding change is actually starting on a picked task — research it, plan
  it, build it, test it, get it signed off, and land it. Walks Research → Plan →
  Implement → Test → Signoff → Close every time, at whatever pace the task needs; no
  task on hand means step 0 hands off to /pick-task first. Skip for choosing what to
  work on (pick-task), brainstorming future work (ideate), and pure research questions
  with no code change attached (dispatch the research workflow directly).
metadata:
  minimum-tier: T3
  short-description: Research, plan, build, test, sign off, land — one task at a time
---

# engineer

JOB: carry one task from a research base to a landed, signed-off change
IN:  a task \u2014 a backend id + short/long from `/pick-task`, or a plain description; if
     none is given, step 0 dispatches `/pick-task` and uses its pick
OUT: the change landed through `/git-sync` (a PR when the repo has a remote, a local
     squash merge otherwise), a short manual-test checklist the human signed off on,
     and the run's working notes on disk in `.context/<task-slug>/` (gitignored,
     never part of the change under review)

## working notes

Everything this run writes while working \u2014 the research summary, the plan doc, test
results \u2014 lives in `.context/<task-slug>/`, which `.gitignore` excludes. It is scratch
for this one run, not a project artifact. Nothing under it is ever staged or committed;
a step that touches tracked repo paths commits those real paths directly.

## 0. get a task

No task on hand \u2192 dispatch `/pick-task`, use what it returns, continue. A task already
in hand (an id, a clear description) skips straight to Research.

A backend-tracked task (GitHub Issue, Linear item, `roadmap.json` entry) gets flipped
to in-progress before Research starts: GitHub swaps the `status:*` label to
`status:in-progress` (`task-graph`'s convention \u2014 `gh issue edit <id> --remove-label
status:<old> --add-label status:in-progress`); Linear moves the issue to its "In
Progress" state via MCP(Model Context Protocol); `roadmap.json` sets the entry's
`status` field to `in progress`. A one-off description with no backend id has no
status to flip \u2014 skip.

Record the id in `.context/branch-tickets.md` (append, one id per line; create the
file if this is the first task the branch has picked up). This is how Close (step 6)
finds every ticket the branch carries \u2014 including a second task picked up mid-branch
(next paragraph) \u2014 without re-deriving it from memory.

**Folding a second ticket into the same branch.** Nothing here forces a new branch
per task \u2014 engineer and `/git-sync` only branch away from main when work is starting
from main itself (`/git-sync` step 3). Running engineer again for a new task while
already on a feature branch with unfinished or unlanded work continues on that same
branch by default: flip the new task's status the same way, append its id to
`.context/branch-tickets.md` alongside the first, and carry it through Research \u2192
Plan \u2192 Implement \u2192 Test same as any task. Both land through one Close.

## 1. Research

Dispatch the research workflow (`workflows/research-sweep/`) with a goal built from the
task: what exists today, how it's currently built, what a change would touch, and
whatever external angles the task needs (web, academic, design/UX(user experience),
news \u2014 the workflow's plan node decides which apply). Write the returned findings
blocks to `.context/<task-slug>/research.md`.

**Check it before building on it.** Show the summary, wait for a go. A wrong finding
caught here is the cheapest catch in the whole run \u2014 every step after this one plans
against it.

## 2. Plan

Write `.context/<task-slug>/plan.md` covering, at minimum:

- **UX(user experience) decisions** \u2014 the before/after, the chosen interaction pattern
  and what was rejected, the intended feeling and action. Pull in `/vocabulary` for
  precise terms and `/show-me` for a before/after diagram where a picture says it
  faster than prose.
- **Data-structure decisions** \u2014 every type, field, and persisted shape, declarations
  only (no bodies yet). Every externally-owned shape gets probed against the real
  thing, not read off documentation \u2014 paste the probe output beside the declaration.
  `/show-me` for the shape diagram when there's more than a couple of types in play.
- **Harness-shaped tasks route through /ai-author first** — a checker in `tools/`, a
  hook, or a skill/agent/workflow edit is harness-shaped. Such a task runs /ai-author's
  should-it-exist gate before you write this plan. /ai-author routes the work to
  tool-author, skill-author, agent-author, or workflow-author. Product-code tasks skip
  this bullet.
- **TDD or not** \u2014 name the call and why: tests-first suits a shape with a clear
  contract and edge cases worth pinning down before code exists; tests-after suits
  exploratory or UI-heavy work where the shape itself is still moving. Either way, Test
  (step 4) still runs.
- Anything in `invariants.md` (repo root) that bears on this task \u2014 read it here, the
  same way any engineer would check standing rules before committing to a shape.

**Get feedback on the plan before building it.** If this session has
`plannotator_submit_plan` available, write the plan and submit it through that tool \u2014
its approve/deny-with-feedback loop (write, submit, revise on denial, resubmit) is the
gate: UX and data-structure decisions live as sections in the one file it reviews, not
separate bespoke protocols. Without that tool available, show `plan.md` directly and
wait for an explicit yes before moving on. Either way: no manifest, no snapshot
hashing \u2014 the plan file itself plus a one-line log of the verdict is the record.

## 3. Implement

Build it. Default to simple and sequential \u2014 one continuous pass through the plan.
Reach for `/task-graph` only when the task is genuinely large enough that disjoint-file
parallel work pays for its own coordination cost; that's a deliberate, named call at
this step, never the default path. When a build deviates from the plan, say so plainly
and fold the deviation back into `plan.md` rather than quietly absorbing it.

## 4. Test

Dispatch fresh-context testers (`spec-tester`, `maestro-tester` \u2014 whichever fits the
surface) against the plan's cases, and a fresh-context reviewer (`code-reviewer`) over
the diff. Neither shares context with whoever built the change \u2014 a verifier reading
the builder's chat is grading its own homework. Collect failures plainly; fix and
re-run rather than arguing with a result.

## 5. Minimal manual test + signoff

6-8 plain-language bullets, each under 75 characters, naming exactly what a human
should click or run to confirm this works. Show the checklist, wait for the manual
verdict. This is the last stop before the change leaves the machine.

## 6. Close

Hand off to `/git-sync` for the whole landing sequence \u2014 committing, pushing, opening
or updating the PR (or the local squash merge when there's no remote), and pruning
branches main already contains. Pass every id from `.context/branch-tickets.md`
along: for each GitHub Issue id, `/create-pr` adds a `Closes #<id>` line to the PR
body, which is what makes merging the PR auto-close the issue; for each
`roadmap.json` id (no PR-merge equivalent exists there) flip its `status` to `done`
as part of this same step, not left for a later run; a Linear item with no linked
GitHub issue gets its state moved to "Done" directly via MCP(Model Context Protocol),
same reasoning. Never reimplement any of that here.

## evals

`evals/run.sh` grades every non-holdout case in `evals/cases.jsonl` against this file
or a candidate, using `evals/rubric.md`; `--holdout` runs the held-out slice.
