---
name: task-graph
description: Use when turning work items with dependencies into a validated graph — the task DAG(directed acyclic graph) for one ticket's implementation plan, or ABCD-NNNN tickets filed into a project's roadmap — with statuses, a cycle check, and disjoint-files parallelism. Skip for multi-agent run topologies (workflow-author owns the GRAPH SPEC) and for reading or executing an existing graph (the caller walks it).
metadata:
  minimum-tier: T3
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
- ids: tickets match `^[A-Z]{2,4}-[0-9]{4}$` (prefix from roadmap.json); tasks are `<ticket>.T<NN>`, NN zero-padded 2 and growing to three digits past 99, assigned max+1 in creation order. Creation order never implies completion order. An empty roadmap starts next_nnnn at 1. A repo with GitHub Issues enabled uses the native issue number as the ticket id instead — see "GitHub Issues backend" below; the `AGNT-NNNN` scheme stays exactly as-is for tasks.json (always local, never promoted to Issues) and for any repo without a connected tracker.
- priority ∈ `urgent | high | med | low`. Optional; a missing or unknown value reads as `med`. Priority ranks SELECTION only (next-ticket.sh sorts on it before unlock count) and never adds an edge: a low ticket that blocks an urgent one is surfaced by the urgent ticket's deps, not by inflating the low one.
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

`scripts/next-ticket.sh roadmap.json` prints the runnable ticket ranked by priority (urgent > high > med > low, missing = med), then by how many todo tickets it unlocks (most transitive todo descendants), then lower NNNN. It rejects unknown deps and cycles by name. A ticket with a cancelled dep is listed as needs-replan on stderr — never auto-selected. The caller may override the pick with a recorded one-line reason (the engineer map stores it in state.json.next_override).

## GitHub Issues backend

For any repo with GitHub Issues enabled and `gh` authenticated, GitHub Issues is the
primary roadmap backend — `roadmap.json` is the fallback shape for a repo without
one. Migrated 2026-08-27 (AGNT-0072); `tasks.json` is unaffected either way (always
local scratch under `.context/`, never promoted to Issues).

**Status** lives in a `status:todo|in-progress|resolved|cancelled|done` label — the
source of truth, one value per issue (`next-issue.sh` hard-errors, naming the issue,
on either zero or more-than-one `status:*` label present at once — an issue must
carry exactly one). Values are hyphenated (`in-progress`, not `in progress`) since a
GitHub label is friendlier to grep/CLI-pass without an embedded space; this is a
spelling difference from roadmap.json's own enum (which used a literal space), not a
meaning difference — the five values still mean exactly what they meant there.
GitHub's native `state`/`stateReason` (open/closed/completed/not_planned) get set in
parallel purely as a UX mirror for the ordinary Issues UI; no script reads them as
the source of truth, since that 2-axis pair alone can't tell `done` from `resolved`
(both close as `COMPLETED`).

**Priority** lives in a `priority:urgent|high|med|low` label. Missing label reads as
`med`, same default as roadmap.json.

**Dependency edges** are GitHub's native `blockedBy`/`blocking` relationship (GA
2025-08-21), not a `deps` array or free-text convention. `gh issue create/edit
--blocked-by`/`--add-blocked-by` sets them; `gh issue list --json ...,blockedBy`
returns the whole graph self-describing (each blocker node carries its own
number/state/stateReason) in one call — no per-issue follow-up needed, and it's
repo-local: GitHub also supports a cross-repo `org/repo#N` blocker, which this repo
files none of today and which `next-issue.sh` does not yet scope correctly (see the
comment at the `blocked_by` extraction in the script) — a known, undone limitation,
not a silent one. **GitHub's own API already refuses a cycle-creating edge
server-side** (confirmed live, 2026-08-27, for a direct two-issue cycle:
`GraphQL: ... this dependency would create a cycle`, mutation rejected, zero partial
write) — not exhaustively verified here for a longer transitive cycle or a cross-repo
edge, so `gh-edge-guard.sh` still treats this as best-effort (matching GitHub's error
text, not a structured error code) rather than assuming it can never be reached. The
same guarantee step 4's `dag-mermaid.sh` validate-before-commit gives roadmap.json is
provided natively here, to the extent confirmed.

`scripts/next-issue.sh` (no file argument — reads the current repo via `gh`) is the
GitHub-backed sibling of `next-ticket.sh`: same ranking algorithm (priority, then
most transitive todo descendants unlocked, then lowest number), same needs-replan
flagging for a cancelled blocker, same stdout/stderr contract. The same bidirectional-
injection warning from step 3 above applies unchanged: a newly filed issue that
nothing points at scores 0 unlocks and sorts last, however much it actually blocks.

`scripts/gh-edge-guard.sh <issue> --blocked-by <ids>` is the write path for adding a
dependency edge to an existing issue (fold `--blocked-by` directly into `gh issue
create` when filing a new one) — it issues the real `gh` mutation, translates a
GitHub-side cycle rejection into the same named-reason failure shape `next-issue.sh`
uses, and round-trips (`gh issue view --json blockedBy`) to confirm a reported success
actually landed the edge sent, not just that the CLI call returned 0 (AGNT-INV-003).

## evals

`evals/run.sh` smoke-tests both scripts on fixtures (cycle rejected, waves rendered, next-in-line computed), then grades every non-holdout case in `evals/cases.jsonl` against this file, or a candidate via `./run.sh candidate.md`, using `evals/rubric.md`; `--holdout` runs the held-out slice. One JSON line per case to stdout, mean to stderr.

`evals/smoke-gh.sh` covers the GitHub Issues backend (`next-issue.sh`,
`gh-edge-guard.sh`) the same way, but against real live `gh` issues on the current
repo — it is NOT run by `run.sh`'s default path, since a live smoke test on every eval
run would spam the real issue tracker. Invoke it explicitly; it creates and always
deletes its own scratch issues, verified via a trap-based cleanup.

## logging

At the end of a use, append ONE JSON line to this artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"task-graph","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md' ':(exclude)**/logs/**' ':(exclude)**/votes/**'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
- `ts` is the machine's current local timezone with offset (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user analyzes these against their own day.
- The excerpt is the relevant transcript parts only: the trigger, the key outputs, any human correction. Never the full transcript; cap ~2KB per line.
