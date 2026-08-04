# phase 06 — data-structures

JOB: every changed or added type, field, enum, and persisted shape exists as a declaration in code — and nothing else changed
IN:  research.md, ux.md, testability.md; phase 05 committed
OUT: declarations in-repo; `.map/<ID>/data-structures.md`

paths: model, type, and schema files only — name the target project's concrete glob in data-structures.md (e.g. `Sources/**/Models/**`, `src/**/*types*`). Edits outside the glob and `.map/` are contract violations.

## steps

1. DO NOT BUILD. DO NOT TEST. Structural changes break things by definition; verification is a later phase's job.
2. define every changed or added type, field, enum, and persisted shape directly in code — declarations only, no function bodies. Booleans get `is` prefixes (docs/code-style.md). Done when every data change the plan implies exists as a declaration.
3. record each delta in data-structures.md: before/after shape, migration note, owner. Done when the file accounts for every declaration in the diff.
4. verify scope: the diff touches only the paths glob plus `.map/`. Commit `map(<ID>): phase 06 data-structures`.

## blame tags

`missing-field` `wrong-ownership` `migration-break` `enum-missing-state` `serialization-shape-wrong`
