# phase 10 — test-cases

JOB: natural-language test cases a FRESH-context stranger can execute through the drive matrix, with security and edge coverage that is never skimped
IN:  testability.md (drive matrix), interfaces.md, ux.md; phase 09 committed
OUT: `.map/<ID>/test-cases.md`

## steps

1. for each user-visible behavior, write a case self-contained for a fresh reader — no "as discussed", no repo lore assumed. Shape: `### TC-NN <title>` / `tags:` (happy | edge | security) / `drive:` (the harness command sketch from the matrix) / `steps:` numbered natural language / `expect:` an OBSERVABLE output or state. The `expect` is what phase 14 turns into the debugger's `expected` — vague expects break the whole diagnosis chain. Done when every case carries drive + observable expect.
2. security and edge sections are mandatory and non-empty: injection-shaped and malformed input, boundary values, interrupted flows, permission boundaries. A genuinely inapplicable category gets a one-line justification, never silence. Done when both sections exist or are justified.
3. coverage: every task's user-visible effect has ≥1 case. Done when the cross-check against tasks.json passes.
4. self-audit: any case whose expect a stranger could not check mechanically gets rewritten. Commit `map(<ID>): phase 10 test-cases`.

## blame tags

`uncovered-bug` `vague-expect` `undrivable-case`
