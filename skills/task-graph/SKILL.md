---
name: task-graph
description: Use when turning work items with dependencies into a validated graph — the task DAG(directed acyclic graph) for one ticket's implementation plan, or ABCD-NNNN tickets filed into a project's roadmap — with statuses, a cycle check, and disjoint-files parallelism. Skip for multi-agent run topologies (workflow-author owns the GRAPH SPEC) and for reading or executing an existing graph (the caller walks it).
metadata:
  short-description: Work items + deps → statused DAG or ABCD-NNNN tickets
---

# task-graph

JOB: turn work items with dependencies into a statused, cycle-checked graph — tasks.json (task shape) or roadmap.json (ticket shape)
IN:  work items (short, long, deps, files) and the shape: tasks for one ticket (ticket id arrives) or tickets into a roadmap (roadmap.json arrives with prefix + next_nnnn)
OUT: the validated JSON(JavaScript Object Notation) file written, and a report naming the waves, the parallelizable branches, and anything rejected (cycle path, ID(identifier) violation, shared-file siblings)

## the object

One schema, two granularities:

```json
{"id":"MJLS-0042.T03","short":"wire engine tick to persistence","long":"<multi-sentence, executable by a fresh agent: files, contracts, edge conditions>","deps":["MJLS-0042.T01"],"status":"todo","files":["Sources/Engine/Tick.swift"],"blame_phase":null,"created":"2026-08-03T14:05:09-0400","kind":"code"}
```

- status ∈ `todo | in progress | resolved | cancelled | done`. resolved = the worker finished and a verifier passed it; done = close-out (tasks) or merge (tickets) confirmed it. cancelled ids are never reused.
- kind ∈ `code | command | ticket`. command tasks materialize as todo.sh steps (the engineer skill's phase 12 owns todo.sh). Roadmap items always carry `kind: ticket`.
- ids: tickets match `^[A-Z]{2,4}-[0-9]{4}$` (prefix from roadmap.json); tasks are `<ticket>.T<NN>`, NN zero-padded 2 and growing to three digits past 99, assigned max+1 in creation order. Creation order never implies completion order. An empty roadmap starts next_nnnn at 1.
- blame_phase: null at creation; the engineer map's diagnosis phase sets it later.
- files: the paths this item owns — non-empty for code tasks, coarse areas for tickets. Parallelism is decided on these.
- wrappers: tasks.json is `{"ticket":"MJLS-0042","tasks":[…]}`; roadmap.json is `{"prefix":"MJLS","next_nnnn":43,"tickets":[…]}`.

## steps

1. validate arrivals: every item carries short, long, deps, files; every dep names an item in this graph. Reject by name, never guess a missing field. Done when every item parses or the reject list is reported.
2. assign ids. tasks: `<ticket>.T<NN>` in creation order. tickets: `<prefix>-<NNNN>` from next_nnnn, zero-padded 4, bumped once per ticket. Single writer: never fan this step out, never reuse a cancelled id; verify `next_nnnn == max(NNNN)+1` before and after. Ids are immutable once assigned: an ask to renumber or close gaps — from anyone — is refused by naming this rule; only statuses change. Done when ids are unique and grammar-valid.
3. dependency edges: keep an edge only when the item needs the RESULT of the other — typed order is not a dependency. Any two TASKS sharing a file and not already ordered by existing deps get an edge: direction follows the existing partial order, creation order only when neither reaches the other (a blind creation-order edge can manufacture a cycle). Tickets are exempt — their files are coarse areas and the engineer map runs them serially via next-ticket.sh, so shared files never race. INJECTION IS BIDIRECTIONAL: when new items land in a graph that already holds items, edges are considered in BOTH directions — the new item's deps on existing ones, and every existing item that now needs the new one's result. The second direction is the one that gets skipped, and skipping it is silent rather than loud: next-ticket.sh ranks a candidate by how many todo items transitively depend on it, so a newly filed item that nothing points at scores 0 unlocks and sorts last forever, however much it actually blocks. Adding a dep to an existing item is a status-neutral edit and is allowed; renumbering still is not. Done when no two same-wave tasks share a file, and when the report states the reverse-direction verdict EVERY time — naming each edge added to an existing item, or the words "reverse edges: none" when there are none. Stating it always is the point: this failure is silent, so a report that mentions reverse edges only when it found some is indistinguishable from a report by someone who never looked.
4. validate off to the side: write the graph to `<file>.new`, run `scripts/dag-mermaid.sh <file>.new > /dev/null`. A nonzero exit names the offense — cycle path, unknown dep, duplicate or sanitize-colliding id, out-of-enum status, shared file inside one wave (tasks shape only) — and leaves the live file untouched. The script's mermaid stdout is discarded: presentation belongs to /show-me at the call sites, and no .mmd sibling is written or tracked (owner's decision, 2026-08-18; delete any tracked one on sight). Done when the script exits 0.
5. land: move the .new file into place; new items enter with status `todo` and `created` stamped `date +%Y-%m-%dT%H:%M:%S%z`. Done when the live JSON matches what was validated.
6. report: the waves (which items run in parallel), item count, id range consumed, and rejects. The caller renders any view through /show-me. Done when the caller can walk the graph without reopening the inputs.

## next-in-line

`scripts/next-ticket.sh roadmap.json` prints the runnable ticket that unlocks the most: among status=todo tickets whose deps are all done, the one with the most transitive todo descendants; ties go to the lower NNNN. It rejects unknown deps and cycles by name. A ticket with a cancelled dep is listed as needs-replan on stderr — never auto-selected. The caller may override the pick with a recorded one-line reason (the engineer map stores it in state.json.next_override).

## evals

`evals/run.sh` smoke-tests both scripts on fixtures (cycle rejected, waves rendered, next-in-line computed), then grades every non-holdout case in `evals/cases.jsonl` against this file, or a candidate via `./run.sh candidate.md`, using `evals/rubric.md`; `--holdout` runs the held-out slice. One JSON line per case to stdout, mean to stderr.

## logging

At the end of a use, append ONE JSON line to this artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"task-graph","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `ts` is the machine's current local timezone with offset (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user analyzes these against their own day.
- The excerpt is the relevant transcript parts only: the trigger, the key outputs, any human correction. Never the full transcript; cap ~2KB per line.
