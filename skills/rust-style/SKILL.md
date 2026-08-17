---
name: rust-style
description: Use when Pi or Codex writes, changes, reviews, or tests Rust `.rs` files. Apply the Rust baseline and report its checks. Skip in Claude Code, where the matching path rule owns this baseline, and skip for Rust discussion that touches no files.
---

# Rust style

JOB: Apply the shared Rust baseline to work on Rust source files.
IN: A Pi or Codex task that reads, changes, reviews, or tests one or more `.rs` files.
OUT: The work follows the baseline, and the final report names checks and exceptions.

1. Read `rust-baseline.md` before work. Continue only after every baseline rule is in context.
2. Apply the baseline to each Rust file in scope. Stop before any change that conflicts with the baseline.
3. State each conflict. Use the weakest compliant design that meets the requirement.
4. Run each applicable baseline check after a change. Report a blocked or skipped check as such.

End the final report with these fields:

```text
Rust baseline: applied
Checks: <commands and results, or review checks>
Exceptions: <none, or each exception and its reason>
```

## evals

`evals/run.sh` checks triggering, baseline use, check reporting, and the Claude Code exclusion.
Run `evals/run.sh` for development cases. Run `evals/run.sh --holdout` for the held-out case.

## logging

At the end of a use, append one bounded JSON (JavaScript Object Notation) line to `logs/usage.jsonl`:

```json
{"ts":"<local time with offset>","artifact":"rust-style","trigger":"<what fired it>","excerpt":"<relevant input and output>","outcome":"success|failure|partial","notes":"<corrections or surprises>"}
```

Use the machine's current timezone with `date +%Y-%m-%dT%H:%M:%S%z`. Never use Coordinated Universal Time.
Keep the excerpt under 2KB. Include only the trigger, key output, and any human correction.
