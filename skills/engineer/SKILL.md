---
name: engineer
description: Use when any development change begins in a project — picking the next ticket from .map/roadmap.json, resuming mid-run from .map/<TICKET-ID>/state.json, adopting an existing branch started outside the map, or ideating future work into tickets. Task scale sets the pace, never the route: every change walks the 23-phase spine (selection, research, UX(user experience) research, testability, structural definition, DAG(directed acyclic graph) implementation, fresh-context testing, invariants, break panel, human sign-off, roadmap refill, PR(Pull Request)). Skip for non-development asks — pure research questions (research-sweep owns those) and authoring skills, agents, or workflows (ai-author owns those).
metadata:
  short-description: The exact 23-phase map for agent coding work
---

# engineer

JOB: drive one tracked coding task through the 23-phase map, ticket selection to landed change plus a refilled roadmap, every loop through the one walk-back rule
IN:  a target project repo; its .map/ dir (bootstrapped on first run); optionally a forced ticket id, a resume ask, a branch to adopt, or an ideation ask
OUT: the change landed as an open PR or a verified local squash merge, the ticket closed in .map/roadmap.json, new tickets plus a refreshed roadmap.mmd, a human sign-off checklist, every gate verdict recorded in state.json.gates, and the full .map/<TICKET-ID>/ run record (state, plan docs, DAG, ledgers, test results) on disk and NEVER tracked

## Files

- `phases/NN-<slug>.md` — one file per phase. Entering phase NN, read that file and ONLY that file; never read ahead. On walk-back, read the re-entry phase's file next, then each visited phase's file as the walk forward reaches it.
- `invariants.md` — the global invariants list; read in phase 17 only, together with the target repo's `.map/invariants.md` additions.
- `inspiration-seed.md` — the default innovation stance; read in phase 21 only; the target repo's `.map/inspiration.md` overrides it.
- `templates/todo.sh` — the todo.sh skeleton; copied and filled in phase 12.

## scale

Task scale sets the pace, never the route. A one-line fix walks all 23 phases in minutes — a phase may complete trivially (a two-line research note, a one-task DAG, a one-tester wave), but no phase is skipped and no gate is waved through. Deciding a phase "doesn't apply" happens inside that phase, on the record, never by not entering it.

## the phase index

| NN | phase | one line | dispatches |
|----|-------|----------|------------|
| 01 | setup | gitignore the run dir; pick by unlock count; branch; GATE A | task-graph scripts |
| 02 | research | ≤500-char cited summary + project state | web-research-summarizer or research-sweep |
| 03 | ux-research | precise terms; experience brief; GATE UX — user selects the direction | /vocabulary |
| 04 | testability | drive matrix; data-only engine plan; gaps become tasks | — |
| 05 | env-check | read-only audit; gaps → todos, never fixed | — |
| 06 | data-structures | declarations only; DO NOT BUILD or TEST | — |
| 07 | interfaces | signatures only; DO NOT EDIT THE BODY | — |
| 08 | task-breakdown | smallest atomic tasks, files ownership mandatory | — |
| 09 | dependencies | result-edges only; shared file forces an edge | — |
| 10 | test-cases | natural-language cases, security + edges, stranger-runnable | — |
| 11 | dag | tasks.json + dag.mmd; fresh plan review; GATE B (incl. ux) | /task-graph, anchor-verifier |
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
| 22 | tickets | candidates to GATE D, then survivors filed; roadmap.mmd shown | /task-graph |
| 23 | close | comment + style pass; fresh review; GATE E; land (PR or local squash) | code-reviewer, /simplify, create-pr |

## the walk-back rule (the only loop in this map)

When any phase produces a FINDING that blames an earlier phase:

1. record — append an open line to `.map/<ID>/walkback.jsonl`: `{"walk":"W<k>","ts","event":"open","from_phase","to_phase","trigger_ids":[…],"reason"}`.
2. re-enter — read the blamed phase's file; redo the phase WITH the finding as input; its output files update in place; commit as a rework.
3. walk forward — re-run every phase between, in order, against the updated upstream outputs. Each visit reads THAT phase's file, appends `{"walk","event":"visit","phase","pass","changed":[…],"open_deviations","open_failures"}`, and commits `map(<ID>): phase NN rework (walk W<k>)`. On a no-change visit the walkback append IS the commit content, so the commit still happens. The walk is CONSECUTIVE, with no gaps. A walk from 11 to 02 visits 02, 03, 04, 05, 06, 07, 08, 09, 10, and 11, every integer in range, one by one. "The finding doesn't touch phase 04" is NOT a skip reason. That call gets made inside 04 and recorded as a `note` on its visit line, MANDATORY whenever `changed` is empty. Only the phase-17 walk has a different range, 06-12 resuming at 17, where 13-16 sit out of range rather than skipped.
4. converge — done only when one full pass from to_phase to from_phase yields zero new findings. Before you append the close line, assert that the final pass visited every integer in range, and that every empty-`changed` visit carries its note. A gap or a bare visit blocks the close. A new finding mid-walk CONTINUES the same walk at the earliest blamed phase: same `W<k>`, no second open line, and no reset.

Counters: `loop_counts` is walk-scoped, and it resets when a walk OPENS. A resume into an already-open walk inherits its counters and resets nothing. Each key caps the CYCLES WITHIN that walk, meaning the plan reviews at 11 and the diagnosis at 15. That is all a cap can measure, because it exists to stop thrashing on one problem. No phase file owns a counter: every cap a phase file names reads `state.json.loop_counts` and inherits this reset.

CAP: 3 cycles per counter within a walk — the 4th STOPS the map, appends an escalate line, and shows the open ledger lines to the user. Across walks the backstop is the ledger rather than a counter. A 4th `open` line for the same trigger family in one run stops the map the same way, and that is what catches a problem recurring across walks. Instances: 13 deviation → the deviated phase; 14 missing functionality → the phase that should have provided it; 15 diagnosed failure → earliest blamed phase; 17 invariant change → earliest affected phase, PLAN-ONLY (the walk spans 06-12 and never re-enters 13-16 — the implementation is shelved); 19 panel failure → route through 15. Fixing at the point of discovery instead of walking back is this skill's named catastrophic.

## state

`.map/<ID>/state.json`: `{"ticket","phase","walk","phase_commits":{"12":"<sha>"},"stash":{"label","sha","attempt","backup_branch"},"loop_counts":{},"next_override","gates":{"A":{"ts","verdict"},…}}`. `loop_counts` belongs to the CURRENT walk and resets to `{}` every time a walk opens. It is walk-scoped state that happens to live in the run's pointer, and it is never a running total across the run. Every human gate writes its own `gates.<letter>` entry — the local timestamp and the human's verdict in their words — the moment it clears. A gate with no entry did not happen: that is the same evidence-not-self-report rule the mechanical gates below run on, and it is what lets a resume trust a gate and a walk-back blame one.

**`.map/<ID>/` is NEVER tracked.** A ticket's run dir is local scratch for one run, holding state.json, tasks.json, probes, rehearsal harnesses, and the ledgers, and none of it is part of the change under review. Phase 01 puts `.map/CPU-*/`-shaped rules in `.gitignore`, using the repo's own prefix, before the run dir exists. The artifacts are then untracked from birth rather than untracked later. The repo-level map is the opposite and IS tracked on purpose: roadmap.json, roadmap.mmd, invariants.md, inspiration.md, and ideation/ are shared state that outlives every ticket. Owner's instruction, 2026-08-07: "i dont think we should be committing ANY ticket specific artifacts". It was filed after CPU-0001, CPU-0003 and CPU-0006 rode into a product repo's history on `git add -A`. Never widen a phase commit to `git add -A`. Stage the paths the phase actually owns.

One commit per phase: `map(<ID>): phase NN <slug>`, made with `--allow-empty`. Most phases write only into the untracked run dir and so stage nothing. The commit is a marker, and the marker is the point. It keeps the phase spine, `phase_commits["12"]`, the phase-16 reset anchor, walk-back blame, and resume-against-the-log all working while it commits zero ticket files. A phase that genuinely touches the repo, meaning 13, 18, 21, 22, and 23, commits real content under the same message. Git history is truth for the spine, state.json is the pointer, and the run dir on disk holds the content. On a spine disagreement, reconcile the pointer to git before proceeding. A walk-back that reworks phase 12 replaces `phase_commits["12"]` with the rework commit — but only while `stash.sha` is empty; once a stash is outstanding the anchor is frozen until phase 18 consumes it. Never push before phase 23: phase 16 rewrites local history, and a pushed branch would then need a force-push, which is banned. While `stash.sha` is set, commits are append-only — no amend, no rebase — and `git stash clear` is banned outright. Stash and reset operations run in this session; they are never delegated to a reviewer (a stash or checkout by code-reviewer is catastrophic in its own rubric).

## entry points

- fresh: no `.map/<ID>/` for the chosen ticket → phase 01. No `.map/roadmap.json` at all → phase 01 bootstraps: look for a connected task system first (Linear MCP(Model Context Protocol) tools, `gh issue list`, TODO or BACKLOG files) and offer to import into roadmap.json (ask the 2-4 letter prefix once); nothing found → ask the user to create the roadmap. HUMAN GATE.
- resume: state.json exists with phase < 23 → verify branch + last map commit against the log, reconcile, and check walkback.jsonl FIRST: an open walk (no close line) resumes inside the walk at its next unvisited phase — resuming up the spine would silently abandon it. Otherwise redo the recorded phase from its file and continue.
- take-over: work exists on a branch the map never ran → create `.map/<ID>/`, run phases 02-12 in reconstruct mode (each phase audits the existing diff into its output file: what exists, what is missing; phase 12 marks only the UNIMPLEMENTED change sites, and already-implemented tasks enter as resolved), then enter the spine at 13.
- ideate: the user brings or wants future-work ideas → run phases 21-22 only on the current branch, exploration recorded to `.map/ideation/YYYY-MM-DD.md`, tickets filed via 22, stop. No run dir, no PR.

Dispatched roles resolve from this repo's fleet: spec-tester, anchor-verifier, debugger, code-reviewer, maestro-tester, web-research-summarizer live in `agents/` (installed at `~/.claude/agents`); /task-graph, /vocabulary, /simplify, /create-pr are sibling skills.

## gates

Five lettered gates bracket the decisions the run does not get to make alone: what to build (A), how (B), whether it works (C), what to build NEXT (D), and whether to publish (E). Two more human stops are conditional and unlettered: phase 01's roadmap bootstrap (no roadmap.json exists yet) and phase 03's sourced-findings check (only when that phase ran a web search). Each writes its `gates` entry the same way.

HUMAN GATE A (end of 01): chosen ticket + unlock reasoning. HUMAN GATE UX (end of 03, ALWAYS — fires with or without a web search): the experience brief — before→after, lineage-tagged directions, intended feeling and action — and the USER selects the direction; `state.json.gates.UX`. HUMAN GATE B (end of 11): the plan bundle — ux.md's chosen pattern and rejected alternates, plan docs, test-cases.md, tasks.json, dag.mmd. HUMAN GATE C (20): the sign-off checklist; wait for the manual-testing verdict. HUMAN GATE D (22): the ticket CANDIDATES, before anything is written to roadmap.json — ids are immutable once assigned and the filed set IS the project's next scope. HUMAN GATE E (23): the push, before the branch leaves the machine — the run's first outward-facing act, and /create-pr does not gate on its own (it stops only on an empty diff or a rejected push). Standing: a difficult contract, or a walk-back CAP hit, stops the run for user discussion.

The test for any gate added later, both arms required. ONE: does the action bind the human to something they cannot cheaply undo — a permanent id, a published artifact, a scope commitment? TWO: would a human answer change what the run does next, and is there no cheaper stop already covering it? A phase that already escalates on its own cap (13, 15, 17, 19) fails arm two — it stops for the human exactly when stopping is informative, and a standing gate there would fire on every clean run and train rubber-stamping. Local and rewindable fails arm one: phase 16's history rewrite is guarded mechanically by the attempt-numbered backup branch and phase 18's three checks, not by a human stop. Correctness is the mechanical gates' job, below. Mechanical exit gates, evidence = files and exit codes, never self-report: leave 13 only at 0 open deviations; 15 only at 0 failures; 17 only at invariant fixpoint; 18 only at 0 TODO(<TICKET>) markers by grep; 19 only on an empty panel failure list.

## history

- 2026-08-07 the run dir left git. Owner, during CPU-0003's phase 23: "i dont think we should be committing ANY ticket specific artifacts. pls remove any `.map/CPU-NNNN/` files and make sure that never happens again". Three ticket dirs (CPU-0001, CPU-0003, CPU-0006) had reached a product repo's history, so the map was leaking one run's scratch into every PR it opened. `.map/<ID>/` is now gitignored by phase 01 before it exists, phase commits are `--allow-empty` markers (which is what preserves the spine, the phase-12 anchor, and resume-against-the-log once most phases stage nothing), phase 16's `:(exclude).map` is gone because gitignore gives the same guarantee for free, and phase 23 mechanically checks `git ls-files` before the PR. The repo-level map stays tracked — it is shared state, not run scratch.
- 2026-08-07 landing is not always a PR. Same run: the repo turned out to have no git remote at all, and the owner chose "merge locally into main (squash always)". Phase 23 step 7 now carries both routes, and phase 01's resolved->done sweep is demoted to a safety net, because a squash merge is invisible to `git branch --merged` — the squashed commit is new, so the branch tip never becomes an ancestor and the sweep silently never fires. Whoever merges flips the ticket to `done` at merge time. Also learned the hard way that untracking a run dir on the branch means merging it DELETES those files from the default branch's checkout: back up and count-verify first.
- 2026-08-07 walk-back corrections (user, at the live MJLS-0021 run, two dictated fixes): (1) walk-forward is CONSECUTIVE — the orchestrator proposed walking 02→05→07→08→09→10→11, skipping 03, 04 and 06 as "unaffected"; step 3's "skip nothing" was too weak to stop it, so it now enumerates the range and names "the finding doesn't touch phase 04" as a non-reason. (2) `loop_counts` is WALK-SCOPED and resets on open — W1's two plan reviews of the pre-W1 plan were still on the counter when W2 opened for an unrelated trigger, so W2's second review hit the cap and the run halted on a thrash that was not happening. Blind judge (grade 6) then caught that the first draft of both broke things: the reset contradicted a CAP that counted WALKS per family, which made that cap unreachable, so CAP now counts cycles-within-a-walk and walkback.jsonl's `open` lines carry the run-scoped backstop; "every integer in between" misfired by construction on phase 17's 06-12 plan-only walk, now carved out in step 3 itself; "recorded there" was unenforceable in the trivial case, now a MANDATORY `note` on any empty-`changed` visit, asserted at the close line so the arithmetic is a gate rather than a property; reset-on-open was ambiguous for the resume entry point (first-class here) and now explicitly excludes a resume into an open walk; mid-walk recursion is declared a continuation, not a new walk, so it cannot silently reset; walk-forward now reads each visited phase's file, not only the re-entry one. The judge also endorsed leaving 13/14/15/17/19 unamended but asked for the invariant behind that decline to be asserted once — "no phase file owns a counter" — which then made phase 11's inline note redundant, so it was cut.
- 2026-08-06 gate refinement round 2 (user, at the live CPU-0003 run): GATE UX made UNCONDITIONAL — fires at end of 03 with or without a web search, the user selects the direction, `state.json.gates.UX`; phase 21's candidate pool gains a mandatory first source (the run's own accumulation: in-session ideas, parked gate outcomes, adopted inspirations) and its web scan a second angle (commonly-built-next alongside bleeding-edge); confirmed GATE D (22) and GATE E (23) match the user's asked-for roadmap-signoff and pre-PR gates.
- 2026-08-06 gate audit (user-triggered: a run filed 12 tickets into roadmap.json unreviewed). Added GATE D (22, candidates before any roadmap write — ids are immutable, the filed set is next scope) and GATE E (23, the push — verified /create-pr does not gate, it stops only on an empty diff or a rejected push); widened GATE B to carry ux.md's chosen pattern (phase 03 picks an interaction pattern that was reaching implementation unshown). Declined gates at 16 (local, rewindable, mechanically guarded) and 13/15/17/19 (already escalate on their caps). Blind judge (grade 7) then drove: every gate writes `state.json.gates.<letter>` so a gate leaves evidence rather than a claim — phase 23's IN now requires D's entry; GATE E gained its consequence sentence and the unrecoverable-push rule (force-push is banned, so a premature push cannot be retracted); the gate test grew its second arm (a phase that already escalates on a cap fails it, which is what actually produced the declines); phase 22 gained a re-entry reconcile so a redo cannot mint a second permanent id for the same work.

- 2026-08-06 user corrections to the day-old gate, three at one live gate B: (1) the phase-03 gate presents an EXPERIENCE BRIEF (before→after experience, lineage-tagged directions, intended feeling and action), never a raw findings dump — first delivery was a term-definition list, rejected; (2) "prior inspiration" means what has inspired THE USER before when designing interfaces (rag search + .map/inspiration.md + inspiration-seed.md), not a shipped example — second delivery guessed generic products, rejected; (3) his history anchors one direction but the brief ALWAYS offers options beyond it, including a new-to-him fresh-inspiration candidate; one he adopts is appended to .map/inspiration.md.
- 2026-08-05 user-mandated (owner directive via ai-author, GEPA bypassed by authority): phase 03 gains a conditional HUMAN GATE — a web search in the UX phase always stops for the user before options are drafted.
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
