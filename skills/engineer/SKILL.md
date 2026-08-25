---
name: engineer
description: >-
  Use when any development change begins in a project — picking the next ticket from
  .map/roadmap.json, resuming mid-run from .map/<TICKET-ID>/state.json, adopting an existing
  branch started outside the map, or ideating future work into tickets. Task scale sets the
  pace, never the route: every change walks the 23-phase spine (selection, research,
  UX(user experience) research, testability, structural definition, DAG(directed acyclic
  graph) implementation, fresh-context testing, invariants, break panel, human sign-off,
  roadmap refill, PR(Pull Request)). Skip for non-development asks — pure research questions
  (research-sweep owns those) and authoring skills, agents, or workflows (ai-author owns
  those).
metadata:
  minimum-tier: T3
  short-description: The exact 23-phase map for agent coding work
---

# engineer

JOB: drive one tracked coding task through the 23-phase map, ticket selection to landed change plus a refilled roadmap, every loop through the one walk-back rule
IN:  a target project repo; its .map/ dir (bootstrapped on first run); optionally a forced ticket id, a resume ask, a branch to adopt, or an ideation ask
OUT: the change landed as an open PR when the project has a remote, or a verified local squash merge when it has none, the ticket closed in .map/roadmap.json, new tickets plus a roadmap view from /show-me, a human sign-off checklist, every gate verdict recorded in state.json.gates, and the full .map/<TICKET-ID>/ run record (state, plan docs, DAG, ledgers, test results) on disk and NEVER tracked

## Files

- `phases/NN-<slug>.md` — one file per phase. Entering phase NN, read that file and ONLY that file; never read ahead. On walk-back, read the re-entry phase's file next, then each visited phase's file as the walk forward reaches it.
- `invariants.md` — the global invariants list; read in phase 17 only, together with the target repo's `.map/invariants.md` additions.
- `inspiration-seed.md` — the default innovation stance; read in phase 21 only; the target repo's `.map/inspiration.md` overrides it.
- `templates/todo.sh` — the todo.sh skeleton; copied and filled in phase 12.

## scale

Task scale sets the pace, never the route. A one-line fix walks all 23 phases in minutes — a phase may complete trivially (a two-line research note, a one-task DAG, a one-tester wave), but no phase is skipped and no gate is waved through. Deciding a phase "doesn't apply" happens inside that phase, on the record, never by not entering it.

## routing

Task scale never changes the route; model routing (docs/routing.md in the agents repo) never changes the map. Four rules bind every run:

- The orchestrator that walks this map is STICKY on the T3 tier (the agents repo's `config/model-tiers.json` names the model) for the whole run. A session that enters on a T5 model (fable) hears about it at GATE A, with the switch recommended. A 23-phase transcript on the deep tier is the single largest avoidable cost in this skill.
- Dispatched roles carry their own pinned tiers in the fleet frontmatter — research summarization T2, building/testing/debugging/verification T3, final review T4. Never override a pin upward without a recorded reason.
- Escalation is evidence-only and rides the walk-back machinery. A task that burns 2 cycles of its `loop_counts` counter may re-dispatch one tier up, on the other provider family. The failure lines ride along as input. A disliked answer escalates to a human gate, not to a bigger model.
- T5 (the deep tier) takes only decisions with project-level branching cost, and T4 is its automatic fallback. That means a phase 06/07 structural call on a large migration, or a plan synthesis at 11 after competing plans. Everything else in this map is at most T4.

Every dispatch carries compiled context only: the phase's contract, the run-dir artifacts that phase names, and the task. Never the transcript, never another phase's raw tool output.

The same compaction rule binds the spine, not only the fleet: each phase's artifacts are the ONLY carrier of its output, so anything a later phase needs and the artifact lacks is a compaction failure to fix in the artifact, never by leaning on transcript memory. The orchestrator watches its own context: past roughly 40% of the window, finish the current phase's artifact, commit, and re-enter fresh through the resume entry point — the .map run record is built to be sufficient, and an intentional restart at 40% beats an automatic compaction at 90% every time.

## the phase index

| NN | phase | one line | dispatches |
|----|-------|----------|------------|
| 01 | setup | gitignore the run dir; pick by unlock count; branch; GATE A | task-graph scripts, /show-me |
| 02 | research | web summary + codebase snapshot; durable graded copy into knowledge/; GATE R | web-research-summarizer or research-sweep, Explore |
| 03 | ux-research | precise terms; experience brief; GATE UX — user selects the direction | /vocabulary, /show-me |
| 04 | testability | drive matrix; data-only engine plan; gaps become tasks | — |
| 05 | env-check | read-only audit; gaps → todos, never fixed | — |
| 06 | data-structures | declarations only; every external shape probed; DO NOT BUILD or TEST | — |
| 07 | interfaces | signatures only, then the call stacks; GATE S; DO NOT EDIT THE BODY | /show-me |
| 08 | task-breakdown | smallest atomic tasks, files ownership mandatory; scope stop over 25 tasks | — |
| 09 | dependencies | result-edges only; shared file forces an edge | — |
| 10 | test-cases | natural-language cases, security + edges, stranger-runnable | — |
| 11 | dag | tasks.json + show-me plan view; fresh plan review; GATE B (incl. ux) | /task-graph, /show-me, anchor-verifier |
| 12 | todos | TODO(task-id) markers + todo.sh; GATE F; the stash-anchor commit | /show-me |
| 13 | implement | fan out disjoint branches; deviations disclosed, never absorbed | fresh builders, anchor-verifier, /show-me |
| 14 | agent-test | fresh testers execute the cases; failures collected | spec-tester, maestro-tester |
| 15 | diagnose | repro-carrying blame; earliest blamed phase wins | debugger |
| 16 | stash | reset to the phase-12 commit; implementation shelved | — |
| 17 | invariants | select + apply to fixpoint | — |
| 18 | final-implement | verified stash apply; zero TODO markers left | fresh builders |
| 19 | break-panel | 5 fresh breakers, task-picked angles | spec-tester, anchor-verifier |
| 20 | signoff | 6-8 manual bullets under 75 chars; GATE C | /show-me |
| 21 | roadmap | work/right/fast + iron-man candidates | web-research-summarizer |
| 22 | tickets | candidates to GATE D, then survivors filed; show-me roadmap shown | /task-graph, /show-me |
| 23 | close | comment + style pass; fresh review; GATE E; land (PR by default, local squash with no remote) | code-reviewer, /simplify, create-pr, /show-me |

## the walk-back rule (the only loop in this map)

When any phase produces a FINDING that blames an earlier phase:

1. record — append an open line to `.map/<ID>/walkback.jsonl`: `{"walk":"W<k>","ts","event":"open","from_phase","to_phase","trigger_ids":[…],"reason"}`.
2. re-enter — read the blamed phase's file; redo the phase WITH the finding as input; its output files update in place; commit as a rework.
3. walk forward — re-run every phase between, in order, against the updated upstream outputs. Each visit reads THAT phase's file, appends `{"walk","event":"visit","phase","pass","changed":[…],"open_deviations","open_failures"}`, and commits `map(<ID>): phase NN rework (walk W<k>)`. On a no-change visit the walkback append IS the commit content, so the commit still happens. The walk is CONSECUTIVE, with no gaps. A walk from 11 to 02 visits 02, 03, 04, 05, 06, 07, 08, 09, 10, and 11, every integer in range, one by one. "The finding doesn't touch phase 04" is NOT a skip reason. That call gets made inside 04 and recorded as a `note` on its visit line, MANDATORY whenever `changed` is empty. Only the phase-17 walk has a different range, 06-12 resuming at 17, where 13-16 sit out of range rather than skipped.
4. converge — done only when one full pass from to_phase to from_phase yields zero new findings. Before you append the close line, assert that the final pass visited every integer in range, and that every empty-`changed` visit carries its note. A gap or a bare visit blocks the close. A new finding mid-walk CONTINUES the same walk at the earliest blamed phase: same `W<k>`, no second open line, and no reset.

Counters: `loop_counts` is walk-scoped, and it resets when a walk OPENS. A resume into an already-open walk inherits its counters and resets nothing. Each key caps the CYCLES WITHIN that walk, meaning the plan reviews at 11 and the diagnosis at 15. That is all a cap can measure, because it exists to stop thrashing on one problem. No phase file owns a counter: every cap a phase file names reads `state.json.loop_counts` and inherits this reset.

CAP: 3 cycles per counter within a walk — the 4th STOPS the map, appends an escalate line, and shows the open ledger lines to the user. Across walks the backstop is the ledger rather than a counter. A 4th `open` line for the same trigger family in one run stops the map the same way, and that is what catches a problem recurring across walks. A phase-13 wave opens ONE walk, never one per finding. The wave's deviations collect until every builder in it reports and every verify returns; then a single walk opens at the earliest phase blamed by the whole set, and it carries every finding as input. Four walkback ledgers across four tickets recorded 49 opens, 31 of them from phase 13, and one ticket alone spent 244 phase visits on 32 one-finding walks to reach 9 visits of phase 13. Batching is what keeps the replan cost under the implementation cost.

Instances: 13 deviation → the deviated phase; 14 missing functionality → the phase that should have provided it; 15 diagnosed failure → earliest blamed phase; 17 invariant change → earliest affected phase, PLAN-ONLY (the walk spans 06-12 and never re-enters 13-16 — the implementation is shelved); 19 panel failure → route through 15. Fixing at the point of discovery instead of walking back is this skill's named catastrophic.

## state

`.map/<ID>/state.json`: `{"ticket","phase","walk","phase_commits":{"12":"<sha>"},"stash":{"label","sha","attempt","backup_branch"},"loop_counts":{},"next_override","gates":{"A":{"ts","verdict","approval":1,"snapshot":{"dir":".map/<ID>/gates/A/1","manifest_sha256":"<sha256>","manifest":{"presentation.md":"<sha256>",…}}},…}}`. `loop_counts` belongs to the CURRENT walk and resets to `{}` every time a walk opens. It is walk-scoped state that happens to live in the run's pointer, and it is never a running total across the run. Every human gate writes its own `gates.<letter>` entry — the local timestamp, the human's verdict in their words, the one-based approval number, and a snapshot of the exact material it presented — the moment it clears. A gate with no entry did not happen: that is the same evidence-not-self-report rule the mechanical gates below run on, and it is what lets a resume trust a gate and a walk-back blame one.

**`.map/<ID>/` is NEVER tracked.** A ticket's run dir is local scratch for one run, holding state.json, tasks.json, probes, rehearsal harnesses, and the ledgers, and none of it is part of the change under review. Phase 01 puts `.map/CPU-*/`-shaped rules in `.gitignore`, using the repo's own prefix, before the run dir exists. The artifacts are then untracked from birth rather than untracked later. The repo-level map is the opposite and IS tracked on purpose: roadmap.json, invariants.md, inspiration.md, ideation/, and knowledge/ are shared state that outlives every ticket. That set is the project's KNOWLEDGE BASE, and every file in it carries provenance — which tickets contributed to it, what evidence backs it, when that evidence was checked — because a run reads these files without having watched them being written. Files under knowledge/ are topic documents updated across runs; no filename, identity, or lifecycle belongs to one ticket, so they are not ticket-specific artifacts. AGNT-0028 is the cost of the alternative: a phase 02 claim sourced from documentation was wrong, the run dir holding it was scratch, and nothing caught the premise until the phase 23 review, with every phase between inheriting it. Owner's instruction, 2026-08-07: "i dont think we should be committing ANY ticket specific artifacts". It was filed after CPU-0001, CPU-0003 and CPU-0006 rode into a product repo's history on `git add -A`. Never widen a phase commit to `git add -A`. Stage the paths the phase actually owns.

One commit per phase: `map(<ID>): phase NN <slug>`, made with `--allow-empty`. Most phases write only into the untracked run dir and so stage nothing. The commit is a marker, and the marker is the point. It keeps the phase spine, `phase_commits["12"]`, the phase-16 reset anchor, walk-back blame, and resume-against-the-log all working while it commits zero ticket files. A phase that genuinely touches tracked repo paths commits that real content under the same message; a phase touching only `.map/<ID>/` makes the allow-empty marker. The phase file owns which paths it may edit, so this contract never keeps a second exhaustive phase list that can drift from those files. Git history is truth for the spine, state.json is the pointer, and the run dir on disk holds the content. On a spine disagreement, reconcile the pointer to git before proceeding. A walk-back that reworks phase 12 replaces `phase_commits["12"]` with the rework commit — but only while `stash.sha` is empty; once a stash is outstanding the anchor is frozen until phase 18 consumes it. Never push before phase 23: phase 16 rewrites local history, and a pushed branch would then need a force-push, which is banned. While `stash.sha` is set, commits are append-only — no amend, no rebase — and `git stash clear` is banned outright. Stash and reset operations run in this session; they are never delegated to a reviewer (a stash or checkout by code-reviewer is catastrophic in its own rubric).

## entry points

- fresh: no `.map/<ID>/` for the chosen ticket → phase 01. No `.map/roadmap.json` at all → phase 01 bootstraps: look for a connected task system first (Linear MCP(Model Context Protocol) tools, `gh issue list`, TODO or BACKLOG files) and offer to import into roadmap.json (ask the 2-4 letter prefix once); nothing found → ask the user to create the roadmap. HUMAN GATE.
- resume: state.json exists with phase < 23 → verify branch + last map commit against the log, reconcile, and check walkback.jsonl FIRST: an open walk (no close line) resumes inside the walk at its next unvisited phase — resuming up the spine would silently abandon it. Otherwise redo the recorded phase from its file and continue.
- take-over: work exists on a branch the map never ran → create `.map/<ID>/`, run phases 02-12 in reconstruct mode (each phase audits the existing diff into its output file: what exists, what is missing; phase 12 marks only the UNIMPLEMENTED change sites, and already-implemented tasks enter as resolved), then enter the spine at 13.
- ideate: the user brings or wants future-work ideas → run phases 21-22 only on the current branch, exploration recorded to `.map/ideation/YYYY-MM-DD.md`, tickets filed via 22, stop. No run dir, no PR.

Dispatched roles resolve from this repo's fleet: spec-tester, anchor-verifier, debugger, code-reviewer, maestro-tester, web-research-summarizer live in `agents/` (installed at `~/.claude/agents`); /task-graph, /vocabulary, /simplify, /create-pr are sibling skills.

## gates

Nine standing gates bracket the decisions the run does not get to make alone: what to build (A), whether the compressed research is true (R), which user experience to choose (UX), what SHAPE the code takes (S), how (B), WHERE the change lands (F), whether it works (C), what to build NEXT (D), and whether to publish (E). The names are stable identities assigned when a gate was added, never a firing order — F fires between B and C. Renaming them to restore alphabetical order would break every phase file, every `state.json.gates` entry, and every finished run's record, for nothing. GATE SPLIT is conditional: phase 08 fires it only when decomposition exceeds 25 tasks, and records `state.json.gates.SPLIT`. One human stop remains unlettered: phase 01's roadmap bootstrap when no roadmap exists. Every named gate writes its `gates.<name>` entry through the same snapshot contract. Every human gate presents its material through /show-me: ask for the smallest fitting view, prefer a console-safe view, and let /show-me select the format.

STANDING APPROVAL. Every gate builds its complete candidate presentation before deciding whether to stop. Run `mkdir -p ".map/<ID>/gates/<letter>"`, then create a FRESH directory with `mktemp -d ".map/<ID>/gates/<letter>/candidate.XXXXXX"`; write the exact text shown to the human to its `presentation.md`; copy every source file the gate names under its `sources/`; write `manifest.json` as `{<relative path>:<sha256>}` over EVERY regular file in the candidate except `manifest.json` itself — including `presentation.md` and every file under `sources/`. Classification is derived only after comparison and lives outside the candidate, so it is never input to the manifest or copied into approved material. Compute `manifest_sha256` with `shasum -a 256 manifest.json`; this is the manifest hash stored in state and written to the ledger. A candidate directory is never reused or cleared, so a source removed since the last visit cannot survive into the new manifest. On approval N, assert `.map/<ID>/gates/<letter>/N/` does not exist, copy the candidate to that immutable directory, and store N, that directory, `manifest_sha256`, and the manifest in `state.json.gates.<letter>`. Never overwrite an approved directory.

A gate re-entered on a walk-back creates a fresh candidate and compares its manifest to the last approved one. Equal manifests → the standing approval carries: append both manifest hashes and that reason to the walkback ledger, and walk on without stopping. Different manifests → run `diff -ru <approved dir> <candidate dir>`, create `.map/<ID>/gates/<letter>/classifications/`, and write `<candidate-basename>.json` there as `{"verdict":"decisional|prose-only","changed_paths":[…],"hunks":[…],"reason":"…"}`. The ledger records that classification path. Every changed hunk appears once. `decisional` presents the DELTA through /show-me and calls it approval N+1. `prose-only` carries the standing approval, appends the classification plus both manifest hashes to the ledger, and never hides the diff. Material never approved always fires as approval 1. Decisional means any changed value in the gate-specific material named by its phase file, or any fact or option on which the previous verdict depended. That includes every claim, source, evidence grade, anchor, shape, signature, task, test case, count, boundary, path, target branch, review verdict, accepted finding, reasoning, and chosen direction. `prose-only` is limited to rendering, typography, or wording changes that preserve every decision-bearing value byte-for-byte in the saved sources; a citation change is decisional whenever the gate judges truth or provenance. The exact saved presentation plus the machine-readable classification, not memory or a bare hash, decides that override. This is not waving a gate through: a gate with no entry never happened, while one with an entry plus byte-identical material has already been answered. One ticket stopped at GATE B thirty-four times and at GATE F twenty-nine times to re-ask questions the human had already ruled on, which is how a human is trained to approve without reading.

HUMAN GATE A (end of 01): chosen ticket + unlock reasoning. HUMAN GATE R (end of 02): research.md — the web summary and the codebase snapshot — before nine phases plan against it; a wrong file:line approved here is the cheapest possible catch; `state.json.gates.R`. HUMAN GATE UX (end of 03, ALWAYS — fires with or without a web search): the experience brief — before→after, lineage-tagged directions, intended feeling and action — and the USER selects the direction; `state.json.gates.UX`. HUMAN GATE S (end of 07): the STRUCTURE — every type, field, enum and persisted shape from phase 06 as before→after, each externally-owned shape beside the probe output that proves it, and every new or changed signature as one line with its owner. Declarations only, no bodies, so this is the cheapest human view in the map and the earliest one that shows the whole shape at once. Ask two questions and no more: is any shape wrong, missing a state, or owned by the wrong type; does any signature sit at the wrong level of its call stack. `state.json.gates.S`. HUMAN GATE SPLIT (conditional in 08): the task count, proposed boundary, both task sets, and which side this ticket keeps; `state.json.gates.SPLIT`. HUMAN GATE B (end of 11): the plan bundle — ux.md's chosen pattern and rejected alternates, test-cases.md, tasks.json, and the plan view from /show-me. Structure reaches Gate B as the DELTA since Gate S, never as the full phase 06 and 07 files again: the user already approved that shape, and re-reading it on every walk is the largest repeated cost in a long run. HUMAN GATE F (end of 12): the CHANGE-SITE INVENTORY — every marker as task-id, file and line with its reason, todo.sh verbatim, and the count of files the ticket touches with any live-system file named. This is the last stop before phase 13 fans builders out in parallel, and the first point at which WHERE the change lands is concrete rather than described. HUMAN GATE C (20): the sign-off checklist; wait for the manual-testing verdict. HUMAN GATE D (22): the ticket CANDIDATES, before anything is written to roadmap.json — ids are immutable once assigned and the filed set IS the project's next scope. HUMAN GATE E (23): the push, before the branch leaves the machine — a project with a remote lands through a PR, one without lands by local squash merge — the run's first outward-facing act, and /create-pr does not gate on its own (it stops only on an empty diff or a rejected push). Standing: a difficult contract, or a walk-back CAP hit, stops the run for user discussion.

The test for any gate added later, both arms required. ONE: does the action bind the human to something they cannot cheaply undo — a permanent id, a published artifact, a scope commitment, or a decision this map's own ledgers show is expensive to reverse? TWO: would a human answer change what the run does next, and is there no cheaper stop already covering it? A phase that already escalates on its own cap (13, 15, 17, 19) fails arm two — it stops for the human exactly when stopping is informative, and a standing gate there would fire on every clean run and train rubber-stamping. Local and rewindable fails arm one: phase 16's history rewrite is guarded mechanically by the attempt-numbered backup branch and phase 18's three checks, not by a human stop. Correctness is the mechanical gates' job, below. Mechanical exit gates, evidence = files and exit codes, never self-report: leave 13 only at 0 open deviations; 15 only at 0 failures; 17 only at invariant fixpoint; 18 only at 0 TODO(<TICKET>) markers by grep; 19 only on an empty panel failure list.

## evals

`evals/run.sh` grades every non-holdout case in `evals/cases.jsonl` against this file plus each case's named phase files (a case may carry a `files` list), or a candidate via `./run.sh candidate.md`, using `evals/rubric.md`; `--holdout` runs the held-out slice. One JSON(JavaScript Object Notation) line per case to stdout, mean to stderr.

## logging

At the end of a work session under this skill, append ONE JSON line to this artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"engineer","trigger":"<entry point + ticket>","excerpt":"<phase span covered + gates passed + walk-backs>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md' ':(exclude)**/logs/**' ':(exclude)**/votes/**'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
- `ts` is the machine's current local timezone with offset (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user analyzes these against their own day.
- The excerpt is the relevant transcript parts only: the trigger, the key outputs, any human correction. Never the full transcript; cap ~2KB per line.
