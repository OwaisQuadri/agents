---
name: task-graph
description: Use when turning work items with dependencies into a validated graph — the task DAG(directed acyclic graph) for one ticket's implementation plan, or ABCD-NNNN tickets filed into a project's roadmap — with statuses, a cycle check, disjoint-files parallelism, and a regenerated mermaid rendering. Skip for multi-agent run topologies (workflow-author owns the GRAPH SPEC) and for reading or executing an existing graph (the caller walks it).
metadata:
  short-description: Work items + deps → statused DAG or ABCD-NNNN tickets + mermaid
---

# task-graph

JOB: turn work items with dependencies into a statused, cycle-checked graph — tasks.json (task shape) or roadmap.json (ticket shape) — plus a regenerated mermaid sibling
IN:  work items (short, long, deps, files) and the shape: tasks for one ticket (ticket id arrives) or tickets into a roadmap (roadmap.json arrives with prefix + next_nnnn)
OUT: the validated JSON(JavaScript Object Notation) file written, its .mmd sibling regenerated and shown, and a report naming the waves, the parallelizable branches, and anything rejected (cycle path, ID(identifier) violation, shared-file siblings)

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
4. validate + render off to the side: write the graph to `<file>.new`, run `scripts/dag-mermaid.sh <file>.new > <sibling>.mmd.new`. A nonzero exit names the offense — cycle path, unknown dep, duplicate or sanitize-colliding id, out-of-enum status, shared file inside one wave (tasks shape only) — and leaves the live files untouched. Done when the script exits 0.
5. land: move both .new files into place; new items enter with status `todo` and `created` stamped `date +%Y-%m-%dT%H:%M:%S%z`. Never hand-edit a .mmd — it is generated output. Done when the live JSON and .mmd agree.
6. report: the waves (which items run in parallel), item count, id range consumed, rejects, and the rendered diagram. Done when the caller can walk the graph without reopening the inputs.

## next-in-line

`scripts/next-ticket.sh roadmap.json` prints the runnable ticket that unlocks the most: among status=todo tickets whose deps are all done, the one with the most transitive todo descendants; ties go to the lower NNNN. It rejects unknown deps and cycles by name. A ticket with a cancelled dep is listed as needs-replan on stderr — never auto-selected. The caller may override the pick with a recorded one-line reason (the engineer map stores it in state.json.next_override).

## history

- 2026-08-10 injection made bidirectional (owner, at the live CPU-0026 gate D): "do we consider the dependencies created for the existing roadmap when we inject tickets into it? like does the make it right ticket block any existing tickets on the roadmap and is that updated automatically?" It did not, and it was not. Step 3 only ever computed the NEW item's deps, and nothing in this skill or in the engineer map's phase 22 ever revisited an existing item. The consequence is not a broken graph, which is why it survived: the graph stays valid and acyclic, and next-ticket.sh (line 21) just ranks by transitive dependents, so every newly filed ticket scores 0 unlocks and sorts last until everything else is done. The live case that exposed it: a barge-in-redesign ticket that reshapes an already-filed echo-suppression ticket would have been filed with no edge, letting the picker send someone to tune a multiplier against a design about to be replaced. The harness did not cover this — t3 is itself an injection case whose expect is edge-silent, so it actively certified the old behaviour — and the GEPA fence forbids the mutation-proposer from writing the case. A fenced case author was dispatched instead and appended t7 (the regression) and t8 (the negative, held out, catching a skill that reports reverse edges only when it finds some). Measured against defect stubs: the one-directional defect scores 9 on t3 and 2 on t7.

- 2026-08-05 shared-file wave rejection scoped to the tasks shape: filing tickets into conversation-mode's roadmap hit it on pairs involving done and cancelled tickets, and the live roadmap (filed with honest result-deps only) already violated it — evidence the rule never fit the serial ticket shape.
- 2026-08-03 authored for the engineer map (phases 11 and 22). Same day, blind-judge fixes (grade 6, by execution): dag-mermaid.sh now rejects out-of-enum statuses, same-wave shared files, duplicate and sanitize-colliding ids, and tolerates a missing deps key end-to-end; next-ticket.sh rejects unknown deps and names cycles consistently; step 3's shared-file edge follows the existing partial order so it can no longer manufacture a cycle; NN overflow, empty-roadmap counter, blame_phase, kind:ticket, and the override destination are now stated.

## evals

`evals/run.sh` smoke-tests both scripts on fixtures (cycle rejected, waves rendered, next-in-line computed), then grades every non-holdout case in `evals/cases.jsonl` against this file, or a candidate via `./run.sh candidate.md`, using `evals/rubric.md`; `--holdout` runs the held-out slice. One JSON line per case to stdout, mean to stderr.

## logging

At the end of a use, append ONE JSON line to this artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"task-graph","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `ts` is the machine's current local timezone with offset (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user analyzes these against their own day.
- The excerpt is the relevant transcript parts only: the trigger, the key outputs, any human correction. Never the full transcript; cap ~2KB per line.
