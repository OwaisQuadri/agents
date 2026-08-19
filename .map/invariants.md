# project invariants — agents repo

Additions phase 17 reads alongside the global list. Same line shape:
`id | affected phase | status | rule | check`. Statuses here stay `seed`.

(empty on first run, AGNT-0001, 2026-08-12)

AGNT-INV-001 | 10 test-cases | seed | concurrent test fixtures use process-locally unique filesystem paths | check: repeat the complete suite under the default parallel test runner without a shared-path collision
AGNT-INV-002 | 07 interfaces | seed | a third-party flag or API semantic that code or a test harness relies on is proven against a hand-built known answer before verdicts or builds trust it | check: the run record names the known-answer validation (oracle) for each relied-on external semantic; cites AGNT-0020 (pyte, RAG-0001) and AGNT-0014 F-01 (herdr pane split ratio)
