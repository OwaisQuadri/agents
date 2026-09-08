---
name: pr-ready
description: Use as the autonomous half of a merge-readiness pass — an autopilot-or-local-checks stage, a project-briefed dual code review (bugbot + security-review, T6), and a cross-provider triage stage that writes evidence for every finding. Never dispatch this directly for interactive work — it posts nothing and decides nothing on its own; the `pr-ready` skill calls it and owns the human review loop and any posting. Skip when only the autopilot loop is wanted with no review pass (autopilot skill directly), and skip for a one-off `/review-bugbot` or `/review-security` run with no triage needed.
metadata:
  minimum-tier: T6
---

# pr-ready (workflow)

The autonomous fan-out/verify half of the `pr-ready` skill's pipeline. It runs three
stages, no human interaction anywhere, and returns structured findings the calling skill
then reviews with the user.

## GRAPH SPEC

```
workflow

GOAL:     take a repo (with or without an open PR) from unready to a set of triaged,
          evidenced code-review findings the caller can walk with the user
FAN OUT:  Review stage only — bugbot and security-review dispatch in one parallel wave,
          each independently on T6 (Fable primary, Astra fallback, high thinking)
MERGE:    plain code collects whichever review(s) returned text — no model, zero tokens
VERIFY:   a fresh-context triage node reads only the two raw review reports, writes a
          verdict + reasoning + snippet for every distinct finding either raised, and
          runs on a T5 chain entry that shares no provider with whichever model actually
          produced the review it is judging
LOOP:     none — one review wave, one triage pass; retries happen only within a single
          dispatch's own primary-then-fallback attempt, never a repeated round
RULE:     triage never shares a provider family with a review model that actually ran
          (the non-same-provider rule); if both reviewers together exhaust every T5
          provider, triage falls back to the untouched full T5 chain rather than skipping
CAP:      1 ready node + 2 review dispatches (each up to 2 attempts: primary, fallback)
          + up to 4 triage attempts (walking T5's chain until one responds) = 11 agents
ON FAIL:  a dead review dispatch (both its primary and fallback return nothing) is named
          in deadNodes, never silently dropped; triage still runs on whatever survived. If
          both reviewers die, triage is skipped and reported as such. If triage's whole
          chain dies, findings return empty and the raw review text is returned instead so
          nothing is lost.
REPORT:   { mode, pr_number, ready, findings[], reviewModelsUsed, triageModelUsed,
          deadNodes[] } — see output contract below
```

Anchors: every finding carries the reviewer that raised it, the file/line/snippet, and
triage's own reasoning for its verdict — never a bare label. The report's `deadNodes` is
the fan-in guard: any dispatch that returned nothing is named, never folded into a
falsely-complete result.

## input contract

Run via the Workflow tool:

```
Workflow({ scriptPath: "<repo>/workflows/pr-ready/pr-ready.workflow.js",
           args: { repo_path: "<absolute repo path>", pr_number: 1364 } })
```

- `repo_path` — absolute path to the repository to work in. REQUIRED; a run without it
  returns `{ error: 'missing input: repo_path' }` and spawns nothing.
- `pr_number` — optional. When given, the Ready stage treats the run as PR mode against
  that PR directly. When omitted, the Ready stage node itself checks whether an open PR
  exists for the current branch (`gh pr view`) and picks PR mode or pre-PR mode from that
  live check — the workflow script itself never decides this, since it has no bash access.

## output contract

```
{
  mode: 'pr' | 'pre-pr',
  pr_number: number | null,
  ready: boolean,                 // false means blocked before review/triage ever ran
  blocker_detail?: string,        // present only when ready is false
  reviewModelsUsed?: { bugbot, security_review },  // each { model, effort, provider } or undefined if dead
  triageModelUsed?: { model, effort, provider },
  findings: [
    { source: 'bugbot' | 'security-review', file, line, snippet, description,
      verdict: 'legit' | 'not-legit', reasoning }
  ],
  rawReviews?: [{ source, text }],  // present only when triage's whole chain died
  deadNodes: string[],             // e.g. ['security-review'], ['triage'], []
}
```

`findings` carries BOTH legit and not-legit verdicts — this workflow classifies, it never
filters. The calling skill decides what happens to each one with the user.

## history

- 2026-09-07 founding run: A fresh-context review of PR #1364 (PIL-3605, the tvOS app)
  showed the need for a repeatable review and triage pass. This workflow does not grade
  its own work. The same change added the dual-provider T6 to `config/model-tiers.json`.
  `docs/routing.md` defines T6 as the highest tier and the Turbo target.
