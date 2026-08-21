# project invariants — agents repo

Additions phase 17 reads alongside the global list. Same line shape:
`id | affected phase | status | rule | check`. Statuses here stay `seed`.

(empty on first run, AGNT-0001, 2026-08-12)

AGNT-INV-001 | 10 test-cases | seed | concurrent test fixtures use process-locally unique filesystem paths | check: repeat the complete suite under the default parallel test runner without a shared-path collision
AGNT-INV-002 | 07 interfaces | seed | a third-party flag or API semantic that code or a test harness relies on is proven against a hand-built known answer before verdicts or builds trust it | check: the run record names the known-answer validation (oracle) for each relied-on external semantic; cites AGNT-0020 (pyte, RAG-0001) and AGNT-0014 F-01 (herdr pane split ratio)

AGNT-INV-003 | 07 interfaces | seed | an operation that reports success acted on exactly the object it was handed, or it reports failure | check: for every success-returning path that hands an object to a foreign surface, a round-trip probe names the object the surface actually acted on and compares it byte-for-byte to the argument; cites AGNT-0015 F-07 (openInNvim returned true while nvim opened a differently-named file, because the escaping stripped a control byte instead of rejecting the input)
