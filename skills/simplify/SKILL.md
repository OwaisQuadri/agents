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

JOB: reduce the code and control-flow complexity that carry current behavior, without code golf or behavior changes.
IN: a completed code change or named code scope, plus the repository and its test commands.
OUT: a verified edit and a fixed report with scope, tests, complexity evidence, reductions, and retained candidates.

## process

1. Start from the changed symbols, not only the changed lines.
2. Trace their callers, callees, events, shared state, persistence, errors, and external boundaries.
3. Add every module whose behavior or side effects the change can affect.
4. Stop the expansion where neither behavior nor side effects can change.
5. Read the project instructions and every module in the scope.
6. Find the narrowest test command that covers the full scope.
7. Run that command before any simplify edit.
8. Stop without edits if the baseline fails. Report the failing command and result.
9. Run `rust-code-analysis-cli --metrics --output-format json --paths <file>` once for each changed Rust, TypeScript, JavaScript, or Python file. Done when every supported changed file has JSON output.
10. Walk each JSON `spaces` tree. Keep records where `kind` is `function`. Report each function name, line range, and cyclomatic metrics.
11. Re-run the analyzer after each control-flow reduction. Report the before and after values. Done when the report names both values.
12. Stop if `rust-code-analysis-cli` is absent. Run the managed agents installer before the next simplify pass. Do not invent a complexity score.

Inspect the scope in this order:

1. Remove unused imports, variables, functions, branches, and obsolete compatibility code.
2. Merge repeated logic when one direct form is clearer than the copies.
3. Reduce measured control-flow complexity when the reduction also makes the code clearer.
4. Inline a single-use helper when its name or boundary adds no useful meaning.
5. Replace verbose loops, chains, and conditionals with clear language or standard-library forms.
6. Remove defensive checks only when the type system proves that the state is impossible.

Search all references before you remove or inline a symbol. Check generated, reflective, configured, and externally used entry points.

Use the language and library versions that the repository supports. Do not modernize past those versions.

Prefer fewer concepts, branches, and repeated words. A lower line count is evidence only when the structure also becomes simpler.

Cyclomatic complexity is a review signal. It is not a target. Keep a high score when flat branches are the clearest form. Never add a helper only to lower the score.

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
Complexity: <function records from rust-code-analysis-cli; before → after or kept reason>
Final: <test, formatter, and static-check results>
```

Do not claim a reduction from formatting, statement packing, renamed short identifiers, or deleted tests.

## evals

`evals/run.sh` grades the non-holdout cases against this file. Run `evals/run.sh candidate.md` for a candidate.
Run `evals/run.sh --holdout candidate.md` for the holdout slice.

## logging

At the end of a use, append one JSON(JavaScript Object Notation) line to
`<repo-root>/skills/simplify/logs/usage.jsonl`, where `<repo-root>` is the output of
`git rev-parse --show-toplevel` — never a path relative to the caller's own working
directory:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"simplify","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md' ':(exclude)**/logs/**' ':(exclude)**/votes/**'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
Use the machine's current local timezone with its offset. Get it with `date +%Y-%m-%dT%H:%M:%S%z`.
Never use UTC(Coordinated Universal Time). Keep the excerpt under 2KB and never include the full transcript.
