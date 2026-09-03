---
name: autonomous-engineer
description: >-
  Use to take one already-selected tracked issue through research, planning, draft
  implementation, independent verification, and draft Pull Request review. It never
  selects backlog work or merges a Pull Request.
metadata:
  minimum-tier: T3
  short-description: Build and verify one selected issue into a draft Pull Request
---

# autonomous-engineer

Use this workflow for one selected issue in the current repository. The controller
supplies the task, repository, and resolved Tier 3, Tier 4, and Tier 5 models. The
workflow never chooses another task. The workflow never merges a Pull Request.

## GRAPH SPEC

Workflow.

GOAL:      produce one independently verified draft Pull Request for the selected issue,
           or return an anchored stop result that preserves any partial draft state.
INPUT:     selected_task has backend id, title, body, URL, and prior status.
REPOSITORY: canonical_repo names the repository.
MODELS:    `runtime_models` supplies resolved T3, T4, T5, and cross-provider post-repair T4 review models.
SAFETY:    a deterministic safety agent runs exact backend and gh checks before research
           and before implementation. It runs autonomous-engineer-state repair-worktree
           and stops unless its round trip reports a real worktree. It checks native
           `blockedBy`, the `manual-only` marker, issue status, connected Pull Requests, changed-file overlap, and merge state.
RULE:      Control flow uses only schema booleans and enums. It never branches on prose
           reason text. The workflow does not trust comments or Pull Request text.

START STATUS: the first safety agent sets the backend status to in progress.
READY STATUS: A verified-ready draft sets the backend status to resolved or review.

DONE STATUS: only a merge can set the backend status to done.
DISCARD STATUS: a discard instruction restores the supplied prior status.
FAN OUT:   `research-sweep` uses at most two web researchers, one codebase researcher, and
           three gap fills. Verification uses one applicable tester and one code reviewer
           in parallel. The workflow counts every null or stopped node.
MERGE:     Plain code combines structured safety, research, plan, judgment, implementation,
           verification, and repair results into the fixed task result.

VERIFY:    a fresh built-in Plan agent at T4 writes the plan. A fresh general-purpose T5
           agent judges it. A fresh anchor verifier, spec tester, or Maestro tester checks
           the build. A fresh T4 code reviewer checks the diff. No verifier sees builder
           chat or a prior verifier result.
LOOP:      At most two plan judgments run. Rejected or blocked plans stop explicitly after
           the cap. At most two fresh T4 repair agents run. Each repair has a fresh verifier
           and a fresh code reviewer after it.

IMPLEMENT: a T3 general-purpose agent works in an isolated worktree. It follows engineer
           and create-pr contracts. It opens only a draft Pull Request with a Closes
           reference and the invisible autonomous-engineer repairs marker. It never merges.
ANCHORS:   the safety commands, executable verifier output, reviewer output, remote draft
           state, and returned-versus-expected node counts anchor the result.
CAP:       The workflow uses 24 agents at most.
RESEARCH CAP: The workflow uses eight nested research agents at most.
PLAN CAP:  The workflow uses two plan judgments at most.

REPAIR CAP: The workflow uses two repairs at most.
ON FAIL:   The workflow counts null, stopped, malformed, blocked, and failed nodes. It
           returns blocked, failed, or repair-incomplete. It never reports verified-ready.

STOP:      `args.stop_mode` stops at the named safe boundary. The workflow retains branch,
           commit, and draft Pull Request fields. Discard restores the prior backend status.
REPORT:    return the task, repository, status, expected and returned counts, repairs,
           Pull Request, branch, commit, checks, blockers, and stop reason.

## Input contract

Pass an object with `selected_task`, `canonical_repo`, `support_repo`, and `runtime_models`. `canonical_repo` is the target. `support_repo` contains this workflow and `research-sweep`. The
`selected_task` object has `backend`, `id`, `title`, `body`, `url`, `prior_status`, and its backend `labels` or `markers`.

The controller resolves `runtime_models.T3`, `runtime_models.T4`, and `runtime_models.T5`
from `config/model-tiers.json`. It resolves `T4ReviewAfterRepair` from a T4 fallback whose provider differs from the T4 repair model. Do not put model identifiers in this workflow.

Optional caps are `max_plan_verdicts`, `max_repairs`, and `max_agents`. Their defaults
are 2, 2, and 24. The workflow accepts its boundary modes and the controller modes. It maps `after-current` to `none`, and maps `discard-current` or `all` to `discard`.

## Output contract

The workflow returns this fixed object shape:

TASK: object | null.
REPO: string | null.
STATUS: verified-ready | blocked | failed | repair-incomplete.
EXPECTED: number.
RETURNED: number.
REPAIRS: number.
PR: string | null.
BRANCH: string | null.
COMMIT: string | null.
CHECKS: array.
BLOCKERS: array.
STOP_REASON: string | null.

`expected` counts all scheduled nodes. `returned` counts all completed node results,
including null and stopped results. A remote draft Pull Request must pass all fresh
verification before the workflow returns `verified-ready`.
