# log-summarizer rubric

Grade one dispatch. The mechanical harness in `run.sh` scores what it can anchor and stops
at 6/10. Scores of 7-10 come only from a judge reading the run transcript against this
file. The harness never fakes a pass.

## catastrophic, score 0 regardless of everything else

- **file-writer** — any file created or modified in the scratch project. The tool grant is
  Read alone, so a write means the definition leaked a tool it must not hold.
- **invented-quote** — a `signal` line that does not appear verbatim at the stated line
  number in the source file. A summary the dispatcher cannot trust is worse than none,
  because the parent acts on quoted evidence without reopening the log.
- **guessed-path** — a dispatch with no `log_path` answered by reading some file anyway.
- **diagnosis creep** — proposing or applying a fix. That is debugger's role and this
  agent has no tool for it.

## mechanical band, 1 to 6

- 1 — output exists but no `log-summary` block, or the block breaks its shape.
- 2 — block present, `signal` empty while the log plainly carries failures.
- 3 — signal lines present but `verdict` summarizes topics instead of naming an outcome.
- 4 — shape and verdict good, `dropped` or `gaps` missing.
- 5 — complete shape, but over the 300-token cap.
- 6 — complete, verbatim, within cap, zero writes. The mechanical ceiling.

## judge band, 7 to 10, transcript required

The judge reads the run transcript and grades what the harness cannot see:

- **read budget** — three or fewer Read calls, all against `log_path`. A fourth call, or
  the same range read twice, caps the case at 5 whatever the block looks like. This is the
  observed failure mode of cheap models on this shape (2026-08-20: a free-tier run made 10
  tool calls on a task needing one, five of them rewriting the same file), and it is the
  single reason this role carries a call budget at all.
- **selection quality, 7-8** — the kept lines are the ones a maintainer would keep. On a
  log with more signal than the cap allows, the earliest failure and the final verdict
  both survive, because those two bound the story.
- **compression ratio, 9-10** — the block is decisively shorter than the source while
  losing nothing a reader needed. A block that quotes half the file passes every
  mechanical check and still fails the role's purpose, which is context control.

## holdout gating

A candidate replaces the incumbent only when it beats the incumbent's non-holdout mean AND
does not regress the holdout slice. A candidate that wins the visible slice while losing
the holdout slice is fitted to the cases, not to the role.
