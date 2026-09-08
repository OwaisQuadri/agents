---
name: session-stats
description: Use when the user asks about his agent token usage, session history, model spend, context growth, or session-over-time comparisons in Pi. Compile the stats with the session-stats binary and analyze the JSON. Skip when the ask is about one live session's current context, which needs no compiled history.
metadata:
  short-description: Compile Pi token-usage stats for analysis
---

# session-stats

JOB: Compile per-session token-usage rows from the local Pi session store and answer the usage question from them.
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

One row per (session, model). `src`: pi. `project`,
`session`, `model`: identity. `input`, `output`, `cacheRead`, `cacheCreate`: summed
tokens. `messages`: assistant-message count. `first`, `last`: ISO 8601 session bounds.
`firstCtx`, `lastCtx`: context tokens at the first and last message (0 = not recorded).

## known gaps

- Pi rows reflect the local session retention horizon.

## evals

`evals/run.sh` builds the binary against the fixture store and checks row shape,
aggregation, and assistant-message filtering. Run it from `skills/session-stats/`.
