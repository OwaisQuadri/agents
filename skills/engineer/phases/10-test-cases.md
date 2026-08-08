# phase 10 — test-cases

JOB: natural-language test cases a FRESH-context stranger can execute through the drive matrix, with security and edge coverage that is never skimped
IN:  testability.md (drive matrix), interfaces.md, ux.md; phase 09 committed
OUT: `.map/<ID>/test-cases.md`

## steps

1. write one case for each user-visible behavior. Each case is self-contained for a fresh reader. Write no "as discussed", and assume no repo lore. The shape is `### TC-NN <title>`, then `tags:`, `drive:`, `steps:`, and `expect:`. `tags:` is happy, edge, or security. `drive:` is the harness command sketch from the matrix. `steps:` is numbered natural language. `expect:` is an OBSERVABLE output or state. Phase 14 turns the `expect` into the debugger's `expected`, so a vague expect breaks the whole diagnosis chain. Done when every case carries a drive and an observable expect.
2. the security and edge sections are mandatory and non-empty. Cover injection-shaped and malformed input, boundary values, interrupted flows, and permission boundaries. A category that does not apply gets a one-line justification, never silence. Done when both sections exist or are justified.
3. check the coverage: every task's user-visible effect has ≥1 case. Done when the cross-check against tasks.json passes.
4. self-audit the cases. Rewrite any case whose expect a stranger could not check mechanically. Commit `map(<ID>): phase 10 test-cases`.

## blame tags

`uncovered-bug` `vague-expect` `undrivable-case`
