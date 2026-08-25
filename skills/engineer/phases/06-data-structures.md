# phase 06 — data-structures

JOB: every changed or added type, field, enum, and persisted shape exists as a declaration in code — and nothing else changed
IN:  research.md, ux.md, testability.md; phase 05 committed
OUT: declarations in-repo; `.map/<ID>/data-structures.md`

paths: model, type, and schema files only. Name the target project's concrete glob in data-structures.md, for example `Sources/**/Models/**` or `src/**/*types*`. An edit outside the glob and `.map/` is a contract violation.

## steps

1. DO NOT BUILD. DO NOT TEST. Structural changes break things by definition, and verification is a later phase's job.
2. define every changed or added type, field, enum, and persisted shape directly in code. Write declarations only, and write no function bodies. Booleans get `is` prefixes, per docs/code-style.md. Done when every data change the plan implies exists as a declaration.
3. PROBE every shape this repo does not own. A response body, an event payload, a file another program writes, a library return type: run the real call or read the real artifact, and paste the executed output into data-structures.md beside the declaration it proves. A shape read from documentation, from memory, or from a caller's expectation is a guess, and a guess here is the single most expensive error in this map. Done when every externally-owned field cites executed output.
4. record each delta in data-structures.md: the before/after shape, the migration note, and the owner. Done when the file accounts for every declaration in the diff.
5. verify the scope: the diff touches only the paths glob plus `.map/`. Commit `map(<ID>): phase 06 data-structures`.

## blame tags

`missing-field` `wrong-ownership` `migration-break` `enum-missing-state` `serialization-shape-wrong` `shape-unprobed`
