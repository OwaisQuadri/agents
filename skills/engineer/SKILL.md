---
name: engineer
description: Use when any development change begins in a project — picking the next ticket from .map/roadmap.json, resuming mid-run from .map/<TICKET-ID>/state.json, adopting an existing branch started outside the map, or ideating future work into tickets. Task scale sets the pace, never the route: every change walks the 23-phase spine (selection, research, UX(user experience) research, testability, structural definition, DAG(directed acyclic graph) implementation, fresh-context testing, invariants, break panel, human sign-off, roadmap refill, PR(Pull Request)). Skip for non-development asks — pure research questions (research-sweep owns those) and authoring skills, agents, or workflows (ai-author owns those).
metadata:
  short-description: The exact 23-phase map for agent coding work
---

# engineer

JOB: drive one tracked coding task through the 23-phase map, ticket selection to merged-ready PR plus a refilled roadmap, every loop through the one walk-back rule
IN:  a target project repo; its .map/ dir (bootstrapped on first run); optionally a forced ticket id, a resume ask, a branch to adopt, or an ideation ask
OUT: an open PR, the ticket resolved in .map/roadmap.json, new tickets plus a refreshed roadmap.mmd, a human sign-off checklist, and the full .map/<TICKET-ID>/ run record (state, plan docs, DAG, ledgers, test results)

## Files

- `phases/NN-<slug>.md` — one file per phase. Entering phase NN, read that file and ONLY that file; never read ahead. On walk-back, read the re-entry phase's file next.
- `invariants.md` — the global invariants list; read in phase 17 only, together with the target repo's `.map/invariants.md` additions.
- `inspiration-seed.md` — the default innovation stance; read in phase 21 only; the target repo's `.map/inspiration.md` overrides it.
- `templates/todo.sh` — the todo.sh skeleton; copied and filled in phase 12.

## scale

Task scale sets the pace, never the route. A one-line fix walks all 23 phases in minutes — a phase may complete trivially (a two-line research note, a one-task DAG, a one-tester wave), but no phase is skipped and no gate is waved through. Deciding a phase "doesn't apply" happens inside that phase, on the record, never by not entering it.

## the phase index

| NN | phase | one line | dispatches |
|----|-------|----------|------------|
| 01 | setup | flip merged tickets done; pick by unlock count; branch; GATE A | task-graph scripts |
| 02 | research | ≤500-char cited summary + project state | web-research-summarizer or research-sweep |
| 03 | ux-research | precise terms; existing + novel options | /vocabulary |
| 04 | testability | drive matrix; data-only engine plan; gaps become tasks | — |
| 05 | env-check | read-only audit; gaps → todos, never fixed | — |
| 06 | data-structures | declarations only; DO NOT BUILD or TEST | — |
| 07 | interfaces | signatures only; DO NOT EDIT THE BODY | — |
| 08 | task-breakdown | smallest atomic tasks, files ownership mandatory | — |
| 09 | dependencies | result-edges only; shared file forces an edge | — |
| 10 | test-cases | natural-language cases, security + edges, stranger-runnable | — |
| 11 | dag | tasks.json + dag.mmd; fresh plan review; GATE B | /task-graph, anchor-verifier |
| 12 | todos | TODO(task-id) markers + todo.sh; the stash-anchor commit | — |
| 13 | implement | fan out disjoint branches; deviations disclosed, never absorbed | fresh builders, anchor-verifier |
| 14 | agent-test | fresh testers execute the cases; failures collected | spec-tester, maestro-tester |
| 15 | diagnose | repro-carrying blame; earliest blamed phase wins | debugger |
| 16 | stash | reset to the phase-12 commit; implementation shelved | — |
| 17 | invariants | select + apply to fixpoint | — |
| 18 | final-implement | verified stash apply; zero TODO markers left | fresh builders |
| 19 | break-panel | 5 fresh breakers, task-picked angles | spec-tester, anchor-verifier |
| 20 | signoff | 6-8 manual bullets under 75 chars; GATE C | — |
| 21 | roadmap | work/right/fast + iron-man candidates | web-research-summarizer |
| 22 | tickets | ABCD-NNNN filed; roadmap.mmd regenerated and shown | /task-graph |
| 23 | close | comment + style pass; fresh review; PR | code-reviewer, /simplify, create-pr |

## the walk-back rule (the only loop in this map)

When any phase produces a FINDING that blames an earlier phase:

1. record — append an open line to `.map/<ID>/walkback.jsonl`: `{"walk":"W<k>","ts","event":"open","from_phase","to_phase","trigger_ids":[…],"reason"}`.
2. re-enter — read the blamed phase's file; redo the phase WITH the finding as input; its output files update in place; commit as a rework.
3. walk forward — re-run every phase between, in order, against the updated upstream outputs; skip nothing; each visit appends `{"walk","event":"visit","phase","pass","changed":[…],"open_deviations","open_failures"}` and commits `map(<ID>): phase NN rework (walk W<k>)`.
4. converge — done only when one full pass from to_phase to from_phase yields zero new findings, then append the close line. A new finding mid-walk recurses at the earliest blamed phase.

CAP: 3 walks per trigger family (`state.json.loop_counts`) — the 4th STOPS the map, appends an escalate line, and shows the open ledger lines to the user. Instances: 13 deviation → the deviated phase; 14 missing functionality → the phase that should have provided it; 15 diagnosed failure → earliest blamed phase; 17 invariant change → earliest affected phase, PLAN-ONLY (the walk spans 06-12 and never re-enters 13-16 — the implementation is shelved); 19 panel failure → route through 15. Fixing at the point of discovery instead of walking back is this skill's named catastrophic.

## state

`.map/<ID>/state.json`: `{"ticket","phase","walk","phase_commits":{"12":"<sha>"},"stash":{"label","sha","attempt","backup_branch"},"loop_counts":{},"next_override"}`. One commit per phase: `map(<ID>): phase NN <slug>`. Git history is truth; state.json is the pointer — on disagreement, reconcile the pointer to git before proceeding. A walk-back that reworks phase 12 replaces `phase_commits["12"]` with the rework commit — but only while `stash.sha` is empty; once a stash is outstanding the anchor is frozen until phase 18 consumes it. Never push before phase 23: phase 16 rewrites local history, and a pushed branch would then need a force-push, which is banned. While `stash.sha` is set, commits are append-only — no amend, no rebase — and `git stash clear` is banned outright. Stash and reset operations run in this session; they are never delegated to a reviewer (a stash or checkout by code-reviewer is catastrophic in its own rubric).

## entry points

- fresh: no `.map/<ID>/` for the chosen ticket → phase 01. No `.map/roadmap.json` at all → phase 01 bootstraps: look for a connected task system first (Linear MCP(Model Context Protocol) tools, `gh issue list`, TODO or BACKLOG files) and offer to import into roadmap.json (ask the 2-4 letter prefix once); nothing found → ask the user to create the roadmap. HUMAN GATE.
- resume: state.json exists with phase < 23 → verify branch + last map commit against the log, reconcile, and check walkback.jsonl FIRST: an open walk (no close line) resumes inside the walk at its next unvisited phase — resuming up the spine would silently abandon it. Otherwise redo the recorded phase from its file and continue.
- take-over: work exists on a branch the map never ran → create `.map/<ID>/`, run phases 02-12 in reconstruct mode (each phase audits the existing diff into its output file: what exists, what is missing; phase 12 marks only the UNIMPLEMENTED change sites, and already-implemented tasks enter as resolved), then enter the spine at 13.
- ideate: the user brings or wants future-work ideas → run phases 21-22 only on the current branch, exploration recorded to `.map/ideation/YYYY-MM-DD.md`, tickets filed via 22, stop. No run dir, no PR.

Dispatched roles resolve from this repo's fleet: spec-tester, anchor-verifier, debugger, code-reviewer, maestro-tester, web-research-summarizer live in `agents/` (installed at `~/.claude/agents`); /task-graph, /vocabulary, /simplify, /create-pr are sibling skills.

## gates

HUMAN GATE A (end of 01): chosen ticket + unlock reasoning. HUMAN GATE B (end of 11): the plan bundle — plan docs, test-cases.md, tasks.json, dag.mmd. HUMAN GATE C (20): the sign-off checklist; wait for the manual-testing verdict. Standing: a difficult contract, or a walk-back CAP hit, stops the run for user discussion. Mechanical exit gates, evidence = files and exit codes, never self-report: leave 13 only at 0 open deviations; 15 only at 0 failures; 17 only at invariant fixpoint; 18 only at 0 TODO(<TICKET>) markers by grep; 19 only on an empty panel failure list.

## history

- 2026-08-03 authored (user-designed 23-phase map; prime-agent mechanics absorbed). Same-day blind-judge fixes (grade 6): phase 16 stashes with `:(exclude).map` so the run ledgers survive for 17 and ride the phase-16 commit; phase-17 walks declared plan-only (06-12); the phase-12 anchor freezes while a stash is outstanding; phase 18's content check is untracked-aware and its suite rerun uses fresh testers; the panel re-attacks after fixes; resume honors open walks; take-over's phase 12 marks only unimplemented sites; plan-review loops got their own counter.

## evals

`evals/run.sh` grades every non-holdout case in `evals/cases.jsonl` against this file plus each case's named phase files (a case may carry a `files` list), or a candidate via `./run.sh candidate.md`, using `evals/rubric.md`; `--holdout` runs the held-out slice. One JSON(JavaScript Object Notation) line per case to stdout, mean to stderr.

## logging

At the end of a work session under this skill, append ONE JSON line to this artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"engineer","trigger":"<entry point + ticket>","excerpt":"<phase span covered + gates passed + walk-backs>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `ts` is the machine's current local timezone with offset (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user analyzes these against their own day.
- The excerpt is the relevant transcript parts only: the trigger, the key outputs, any human correction. Never the full transcript; cap ~2KB per line.
