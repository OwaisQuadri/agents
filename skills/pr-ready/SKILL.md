---
name: pr-ready
description: >-
  Use when a PR (or the current local diff, before a PR exists) needs a full
  merge-readiness pass — autopilot the PR or run local checks, run a project-briefed
  dual code review (bugbot + security-review) on Fable/Astra, triage every finding with
  a cross-provider verifier, then walk the user through each finding one at a time
  before posting anything. Triggered by `/pr-ready`. Skip for a bare autopilot loop with
  no review pass (`autopilot` skill directly), skip for a one-off `/review-bugbot` or
  `/review-security` with no triage or human-review loop wanted, and skip when the user
  wants to review PRs generally rather than get one specific PR/diff to ready
  (`pr-review` skill).
metadata:
  minimum-tier: T3
---

# pr-ready

JOB: take one PR (or the current local diff) from wherever it is to a state where every
     code-review finding on it has been through a human decision, with dismissals posted
     and legit findings handed off — never further than that.
IN:  a repo path (default: current working repo) and, optionally, a PR number. No number
     and no open PR for the branch means pre-PR mode.
OUT: for a legit-confirmed finding, nothing is fixed here — it is collected and reported
     for a separate engineer pass. For a not-legit-confirmed finding, one inline PR
     comment (PR mode) or one resolved-in-artifact note (pre-PR mode). Never a merge,
     never an auto-merge, never a draft marked ready.

## 1. Resolve inputs

Determine `repo_path` (the active workspace/repo root) and, if the user named a PR
number or link, resolve it. Otherwise leave `pr_number` unset — the workflow's own Ready
stage checks for an open PR on the current branch itself; do not guess mode here.

## 2. Run the workflow

Dispatch `workflows/pr-ready/pr-ready.workflow.js` via the Workflow tool with
`{ repo_path, pr_number }`. This runs autonomously: autopilot-or-local-checks, the
project-briefed dual review, and cross-provider triage. It posts nothing and decides
nothing — every returned finding, legit and not, comes back for the next step.

If the workflow returns `ready: false`, stop and report the `blocker_detail` to the user;
do not proceed to review or triage findings that were never produced.

If `findings` is empty (nothing to review) but `ready` is true, report that plainly and
stop — there is no review loop to run.

## 3. Build the review artifact

Write one markdown file — one section per finding, each showing: the reviewer that
raised it (bugbot / security-review), file:line, the snippet, the description, triage's
verdict, and triage's reasoning. Note any `deadNodes` at the top of the file so the user
sees what didn't run before reviewing what did.

If the finding count is unusually large (rough guide: past ~15), say so before offering
the next choice, so the user picks a mode with that scale in mind rather than discovering
it partway through.

## 4. Let the user choose how to review it

Ask exactly one question, with these three options:

- **Plannotator**: open the markdown file in Plannotator; the user approves, rejects, or
  edits each finding there. Use `plannotator_submit_plan` on the artifact and read back
  its per-finding feedback the same way this skill's own authoring plan was reviewed.
- **In-session**: walk every finding one at a time with `ask_user_question` — snippet,
  verdict, reasoning — and record confirm / override / reclassify for each.
- **Hand-off**: report the artifact's path and stop entirely. The user edits it directly
  in their own editor (for example marking each finding CONFIRMED-DISMISS or
  CONFIRMED-LEGIT inline) and tells you when to resume; re-read the file at that point
  rather than assuming what changed.

Nothing from step 5 onward happens until whichever mode the user picked has produced a
decision for every finding.

## 5. Apply the user's decisions

For each finding the user confirmed as not legit:
- **PR mode**: post one inline PR review comment at its file:line, mouthpiece style
  (plain words, no dashes joining clauses, facts in backticks), explaining why it doesn't
  apply — this is what stops other reviewing bots (Graphite AI, CodeRabbit, etc.) from
  re-raising the same thing.
- **Pre-PR mode**: mark it resolved in the artifact instead; there is no PR to comment on
  yet.

For each finding the user confirmed as legit: do not fix it here. Collect it into one
list and report it back at the end as work for a separate engineer pass.

Report a posted-or-resolved-vs-confirmed count before finishing — if any post failed or
any confirmed finding didn't make it into either bucket, say so explicitly rather than
letting the count go unreported.

## Rules

- Never fix or edit code as part of this skill. Dismissal is a comment, never a diff.
- Never merge the PR, enable auto-merge, mark a draft ready, or create a PR in pre-PR
  mode — those stay separate, deliberate steps.
- Never post a dismissal comment for a finding the user has not actually confirmed.
- If `bugbot` or `security-review` visibly ignored the model override passed to them
  (the workflow's `reviewModelsUsed` doesn't match what was requested, or a downstream
  signal shows a different model ran), say so plainly instead of reporting the run as
  fully on-spec.
