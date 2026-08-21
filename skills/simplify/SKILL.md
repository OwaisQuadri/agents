---
name: simplify
description: >-
  Use when code changes are complete and need a simplify pass before final review, or when
  the user invokes /simplify on named code. Remove dead code, duplication, needless
  indirection, and verbose constructs while preserving behavior and readability. Skip when
  tests cannot establish a passing baseline, the request allows behavior changes, or the
  target is prose rather than code.
metadata:
  short-description: Reduce code without code golf or behavior changes
---

# simplify

JOB: reduce the amount of code that carries the current behavior, without code golf or behavior changes.
IN: a completed code change or named code scope, plus the repository and its test commands.
OUT: a verified edit and a fixed report with scope, tests, reductions, and retained candidates.

## process

1. Start from the changed symbols, not only the changed lines.
2. Trace their callers, callees, events, shared state, persistence, errors, and external boundaries.
3. Add every module whose behavior or side effects the change can affect.
4. Stop the expansion where neither behavior nor side effects can change.
5. Read the project instructions and every module in the scope.
6. Find the narrowest test command that covers the full scope.
7. Run that command before any simplify edit.
8. Stop without edits if the baseline fails. Report the failing command and result.

Inspect the scope in this order:

1. Remove unused imports, variables, functions, branches, and obsolete compatibility code.
2. Merge repeated logic when one direct form is clearer than the copies.
3. Inline a single-use helper when its name or boundary adds no useful meaning.
4. Replace verbose loops, chains, and conditionals with clear language or standard-library forms.
5. Remove defensive checks only when the type system proves that the state is impossible.

Search all references before you remove or inline a symbol. Check generated, reflective, configured, and externally used entry points.

Use the language and library versions that the repository supports. Do not modernize past those versions.

Prefer fewer concepts, branches, and repeated words. A lower line count is evidence only when the structure also becomes simpler.

Keep a candidate unchanged when it does any of these jobs:

- Names a domain concept that would disappear inside its caller.
- Separates side effects, ownership, error handling, or a public boundary.
- Makes the common path easier to read.
- Guards data that enters from outside the type-checked boundary.
- Needs a dense expression, chained ternary, hidden mutation, or compressed control flow.

Apply only candidates whose behavior is clear from the code and tests. Do not add an abstraction to support a possible future use.

Run the same test command after the edits. Restore only this pass's edits if the command fails.

Run the repository formatter and static checks for the changed files. The pass is complete when all checks pass.

## report

Return exactly these fields:

```text
Scope: <affected modules and side-effect boundaries>
Baseline: <command and result>
Reduced: <removed symbols, branches, repetitions, or boilerplate>
Kept: <valuable candidates left unchanged, or none>
Final: <test, formatter, and static-check results>
```

Do not claim a reduction from formatting, statement packing, renamed short identifiers, or deleted tests.

## evals

`evals/run.sh` grades the non-holdout cases against this file. Run `evals/run.sh candidate.md` for a candidate.
Run `evals/run.sh --holdout candidate.md` for the holdout slice.

## logging

At the end of a use, append one JSON(JavaScript Object Notation) line to this artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"simplify","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

Use the machine's current local timezone with its offset. Get it with `date +%Y-%m-%dT%H:%M:%S%z`.
Never use UTC(Coordinated Universal Time). Keep the excerpt under 2KB and never include the full transcript.
