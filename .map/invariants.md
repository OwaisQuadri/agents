# project invariants — agents repo

Additions phase 17 reads alongside the global list. Same line shape:
`id | affected phase | status | rule | check`. Statuses here stay `seed`.

(empty on first run, AGNT-0001, 2026-08-12)

AGNT-INV-001 | 10 test-cases | seed | concurrent test fixtures use process-locally unique filesystem paths | check: repeat the complete suite under the default parallel test runner without a shared-path collision
