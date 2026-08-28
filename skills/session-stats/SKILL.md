---
name: session-stats
description: Use when the user asks about his agent token usage, session history, model spend, context growth, or session-over-time comparisons across Claude Code, Pi, Codex, or Cursor. Compile the stats with the session-stats binary and analyze the JSON. Skip when the ask is about one live session's current context, which needs no compiled history.
metadata:
  short-description: Compile cross-agent token-usage stats for analysis
---

# session-stats

JOB: Compile per-session token-usage rows from every local agent store and answer the usage question from them.
IN: A usage, cost, history, or comparison question. No arguments arrive; the binary reads the local stores itself.
OUT: The answer, grounded in named rows or aggregates, plus the path of the compiled JSON. A web view only when the user asks to see the graph.

## steps

1. Compile the rows. Run `session-stats --json /tmp/session-stats.json` (build with
   `cargo build --release` in `tools/session-stats/` if the binary is missing).
   Done when the command prints a row count.
2. Analyze the JSON with `jq` or `python3` — never by loading raw session transcripts,
   and never by pasting the whole JSON into context. Done when every figure in the
   answer traces to a query you ran.
3. Only when the user asks to see the graph:
   `session-stats --out /tmp/session-stats.html --open`.

## row fields

One row per (session, model). `src`: claude | pi | codex | cursor. `project`,
`session`, `model`: identity. `input`, `output`, `cacheRead`, `cacheCreate`: summed
tokens. `messages`: assistant-message count. `first`, `last`: ISO 8601 session bounds.
`firstCtx`, `lastCtx`: context tokens at the first and last message (0 = not recorded).

## known gaps

- Claude rows start at the retention horizon (`cleanupPeriodDays`); older transcripts are deleted.
- Cursor rows carry tokens for only a minority of sessions and no context sizes; timelines are complete.

## evals

`evals/run.sh` builds the binary against the fixture store and checks row shape,
aggregation, and dedup. Run it from `skills/session-stats/`.

## logging

At the end of a use, append one bounded JSON line (~2KB, local timezone with offset,
never UTC) to `<repo-root>/skills/session-stats/logs/usage.jsonl`, where `<repo-root>`
is the output of `git rev-parse --show-toplevel` — never a path relative to the
caller's own working directory:

```json
{"ts":"<date +%Y-%m-%dT%H:%M:%S%z>","artifact":"session-stats","trigger":"<what fired it>","excerpt":"<question + key figures>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md' ':(exclude)**/logs/**' ':(exclude)**/votes/**'`. A
  Reflect pass drops lines written against a prompt that no longer exists.