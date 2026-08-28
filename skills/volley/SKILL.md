---
name: volley
description: Use when the user wants a fast back-and-forth instead of one long turn, so he steers between steps and waits on nothing. Every turn ends inside 30 seconds. Work that runs longer is dispatched to the background, and its result arrives on a later turn. Skip when the user asks for one finished result and no conversation. Skip a single quick step that already fits inside the budget.
metadata:
  short-description: Short turns; long work goes to the background
---

# volley

JOB: run the session in short turns, and dispatch every step that does not fit the turn budget
IN:  the user invokes /volley; he names a task, or he continues the one already in play
OUT: one step per turn, with its result or the handle of the job dispatched for it; a flight list that closes every turn

## the budget

30 seconds is the cap on one turn. It measures the turn, and it never measures the work.
A background job runs for as long as it needs. The cap says where the work runs, not how
much work the session does.

Estimate each step before you start it. A step that does not fit the cap never starts in
the foreground. When the estimate is not clear, treat the step as large and dispatch it.

## scope

One concern per turn. A second concern becomes the next turn.

Never widen the step the user asked for. He steers this session between turns, so an
improvement he did not ask for costs him a turn that he did not choose. Bring the idea to
him in one line instead, and let him spend the next turn on it.

## steps

1. size the next step against the 30-second cap. Done when you label the step small or large.
2. small step: do it in the foreground and report the result. Done when the turn carries the result.
3. large step: dispatch it. Use `Bash` with `run_in_background: true` for one command. Use the `Agent` tool with `run_in_background: true` for work over several steps. Done when the turn carries the handle instead of a result.
4. never block on a background job. Do not poll it, and do not sleep against it. The harness sends a notification when the job ends. Done when the turn ends with the job still open.
5. close every turn with the flight list. Write one line per open job: `in flight: <handle> — <what it does>`. Write `in flight: none` when no job is open. Done when the list accounts for every job still open.
6. on a notification that a job ended, report its result in one short turn. Take the next step in the turn after that. Done when the result reaches the user and the flight list drops that job.

## the trade

A dispatched step costs the user one turn of latency and buys back the 30 seconds. A
foreground step that overruns the cap costs him the whole wait with nothing to steer. Pay
the dispatch cost whenever the estimate is close.

## evals

`evals/run.sh` grades every non-holdout case in `evals/cases.jsonl` against this file, or a candidate via `./run.sh candidate.md`, using `evals/rubric.md`; `--holdout` runs the held-out slice. One JSON(JavaScript Object Notation) line per case to stdout, mean to stderr.

## logging

At the end of a use, append ONE JSON line to
`<repo-root>/skills/volley/logs/usage.jsonl`, where `<repo-root>` is the output of
`git rev-parse --show-toplevel` — never a path relative to the caller's own working
directory:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"volley","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md' ':(exclude)**/logs/**' ':(exclude)**/votes/**'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
- `ts` is the machine's current local timezone with offset (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user analyzes these against their own day.
- The excerpt is the relevant transcript parts only: the trigger, the key outputs, any human correction. Never the full transcript; cap ~2KB per line.
