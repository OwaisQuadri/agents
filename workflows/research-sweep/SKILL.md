---
name: research-sweep
description: Use to answer one research question by fanning out fresh-context web-research-summarizer dispatches over distinct angles, gap-checking with an independent critic, and filling what it finds missing — returns cited findings blocks plus a fan-in count; the caller synthesizes. Skip for a single-fact lookup the caller settles in a few tool calls, for repository or codebase questions (built-in Explore owns those), and when the caller already knows the exact angles and needs only one researcher (dispatch the agent directly).
---

# research-sweep

The sweep topology over the [[web-research-summarizer]] agent. The agent is the node;
this is the graph. Proven on the 2026-07-31 mobile-testing-tool sweep, where the
critic caught a live contradiction between two researchers (Maestro physical-iOS
support) and three gap-fill dispatches resolved it.

## GRAPH SPEC

```
workflow

GOAL:     answer one research question with per-claim-cited findings blocks the
          caller can act on without opening the sources
FAN OUT:  a plan node turns the goal into 3-6 self-contained dispatches, then one
          web-research-summarizer per dispatch, all in parallel
MERGE:    plain code collects the findings blocks — no model, zero tokens
VERIFY:   a fresh-context completeness critic reads the goal + blocks only, never a
          researcher's transcript; each researcher's own contract anchors every claim
          to a URL fetched that run
LOOP:     one gap-fill round, at most 3 dispatches named by the critic, then stop
RULE:     every claim line carries a source URL + date; stale sources flagged
CAP:      6 planned + 3 fill researchers; 11 agents total counting the plan node
          and the critic
ON FAIL:  any dispatch that returns nothing is named in the report by label, never
          dropped silently; a dead critic is reported as coverage-unverified
REPORT:   findings blocks + critic notes + expected vs returned counts + missing labels
```

Anchors: every claim's source URL resolves to a page fetched that run (the dispatcher
spot-fetches a sample), and the report's returned-vs-expected count.

## input contract

Run via the Workflow tool:

```
Workflow({ scriptPath: "<repo>/workflows/research-sweep/research-sweep.workflow.js",
           args: { goal: "<the research question>", max_researchers: 6 } })
```

- `goal` — the research question, specific enough that "answered" is checkable.
  REQUIRED; a run without it returns `missing input: goal` and spawns nothing. A bare
  string as `args` is treated as the goal.
- `max_researchers` — optional cap on planned dispatches, clamped to 6.

## output contract

`{ goal, blocks, criticNotes, expected, returned, missingLabels }` — `blocks` is
`[{ label, text }]` where each `text` is one fenced findings block per the
web-research-summarizer output contract, each ending with its fenced `log` block. The
dispatcher stamps `ts` on those log lines and appends them per that agent's own
logging section; this workflow's own log line is separate and covers the sweep.

## history

- 2026-07-31 founding run (mobile-testing tool sweep, 8/8 blocks, critic caught a live
  contradiction), then generalized with a plan node. Same day, pre-live blind-judge fix:
  dead researchers were wrapped into truthy blocks before the null-filter, so the fan-in
  guard could report a partial set as complete; results now stay null until counted, the
  plan node is guarded, and the never-binding total cap was dropped.

## logging

At the end of a use, append ONE JSON(JavaScript Object Notation) line to this
artifact's `logs/usage.jsonl` in the agents repo at `~/Documents/agents`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"research-sweep","trigger":"<what fired it>","excerpt":"<goal, dispatch labels, counts, critic verdict>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md' ':(exclude)**/logs/**' ':(exclude)**/votes/**'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
- `ts` is the machine's current local timezone with offset
  (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user
  analyzes these against their own day.
- The excerpt is the relevant parts only — the goal, the labels, the counts, any
  human correction. Never the full blocks; cap ~2KB per line.
