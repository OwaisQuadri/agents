# code-reviewer TUNING.md

## 2026-08-31 — gepa-due usage_count trigger (29 uses, prompt_version 76a831fda)

Zero votes were on file when this run started, so Reflect had nothing to read. Per
protocol, dispatched 5 fresh-context blind judges (Agent tool, general-purpose,
isolated), one per the 5 most recent `logs/usage.jsonl` lines, each given only
`code-reviewer.md` and its one assigned line. All 5 votes landed
(`agents/code-reviewer/votes/votes.jsonl`), all against `prompt_version: 76a831fda`.

Grades: B-, C+, C+, B, B-. 4 of 5 votes independently named the same defect: the
`## baseline discipline` section's example command used plain `mktemp
/tmp/code-review-baseline.XXXXXX` for `dispatch-baseline stamp --out`, and every
real invocation in the usage log hit `File exists (os error 17)` on the first
attempt because `tools/dispatch-baseline` opens `--out` with `create_new(true)` —
`mktemp` pre-creates the file, so this exact command form fails deterministically,
not intermittently. Reproduced directly against the built binary before touching
anything:

```
$ cargo run --release --manifest-path tools/dispatch-baseline/Cargo.toml -- \
    stamp --repo . --out $(mktemp /tmp/code-review-baseline.XXXXXX)
dispatch-baseline: /tmp/code-review-baseline.KOo0vu: File exists (os error 17)
```

and confirmed the fix (`mktemp -u`, name only, not pre-created) exits 0 and writes
the stamp on the first try.

**Decide**: this is a defect fix, not a tuning tweak, so it shipped on the
reproduction plus execution evidence above rather than on `evals/run.sh`'s mean —
none of the existing cases (c1-c5) exercised the literal baseline-stamp shell
command, so a harness score could not have caught or measured this. Per the fenced
defect-fix path, a case-author sub-agent was dispatched in the same pass with only
the failure histogram and the reproduction (never this diff, never the mutated
file). It added case `c6` to `evals/cases.jsonl` plus matching `grade_case`
support in `evals/run.sh`: `run.sh` now builds `dispatch-baseline` and puts it on
the dispatched agent's PATH, captures the raw tool-call transcript via
`--output-format stream-json`, and fails the case (`baseline-stamp-retry`,
`fragile-mktemp-form`) if the agent's chosen stamp command isn't a clean,
single-shot success.

**Mutation shipped**: `code-reviewer.md`'s baseline-stamp example command changed
from `mktemp /tmp/code-review-baseline.XXXXXX` to `mktemp -u
/tmp/code-review-baseline.XXXXXX`, with one added sentence naming why (`create_new`
on the tool side).

**Deferred, not acted on** (no reproduction, single-vote each, narrowing needs an
observed false positive it doesn't have yet): vote 2 questioned whether the
closing `dispatch-baseline check` should discard an entire substantiated review
when unrelated out-of-scope paths (e.g. `.context/.memory` artifacts) moved during
the run, rather than scoping the check to files the review actually touched. Vote 3
flagged that the source file doesn't explicitly say whether a live human
pause/stop mid-run should be treated the same as a self-detected baseline
mismatch. Both are open list items for the next Reflect pass if further evidence
repeats either one.
