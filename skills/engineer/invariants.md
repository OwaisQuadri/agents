# global invariants

The live list phase 17 reads on every run, alongside the target repo's
`.map/invariants.md` additions. One pipe line per invariant:
`id | affected phase | status | rule (one testable sentence) | check: <command or assertion pattern>`.
Statuses here are always `seed`; a run flips its own copy to selected/applied/skipped.
Curate by editing lines; never renumber ids.

## assertions and safety (NASA/JPL Power of Ten)

INV-001 | 07 interfaces | seed | every loop carries a fixed upper bound on iterations, statically checkable | check: no unbounded while/for in changed files without an exit-bound counter
INV-002 | 07 interfaces | seed | assertion density averages at least two per public function; assertions are side-effect-free and each failure has a defined recovery action | check: count asserts per changed public function; grep asserts for mutations
INV-003 | 07 interfaces | seed | every caller checks non-void return values and every function validates its parameters | check: changed call sites ignoring returns fail review
INV-004 | 06 data-structures | seed | data objects are declared at the smallest possible scope | check: declarations hoisted above their narrowest use fail review
INV-005 | 12 todos | seed | the build passes at the strictest warning level with zero warnings | check: build once with warnings-as-errors in phase 18
INV-006 | 07 interfaces | seed | states that must never occur are asserted absent at the boundary where they would first appear (negative-space assertion) | check: every "never" sentence in interfaces.md maps to an assert

## contracts (Design by Contract)

INV-007 | 07 interfaces | seed | every public mutator asserts its preconditions so a violation fails at the call site, not downstream | check: interfaces.md preconditions map one-to-one to asserts
INV-008 | 07 interfaces | seed | every routine's postcondition is stated and observable | check: each postcondition in interfaces.md has a covering test case
INV-009 | 06 data-structures | seed | type invariants hold before and after every exported call | check: the invariant lives in one helper asserted at entry and exit, or a property test

## property families (property-based testing)

INV-010 | 10 test-cases | seed | serialize-then-deserialize returns the original for all domain values (round-trip) | check: a round-trip property test exists per persisted shape
INV-011 | 10 test-cases | seed | operations declared idempotent are tested as f(f(x)) == f(x) | check: an idempotence case per declared-idempotent operation
INV-012 | 10 test-cases | seed | stateful behavior is checked against a simplified model over generated command sequences | check: a model-based test drives the data-only engine

## state machines

INV-013 | 06 data-structures | seed | illegal states are unrepresentable — the type system excludes invalid combinations instead of runtime-validating them | check: no boolean pair or stringly state where a closed enum fits
INV-014 | 06 data-structures | seed | the system is in exactly one state at a time and every transition passes through the single transition function | check: grep for direct state assignment outside the transition function returns 0

## security (trust boundaries)

INV-015 | 07 interfaces | seed | data crossing a trust boundary is validated to type, range, and length against an allow-list before use | check: every external input path in interfaces.md names its validator
INV-016 | 10 test-cases | seed | output is encoded for its destination context at the point of use, never stored pre-encoded | check: no manual string concatenation into HTML(HyperText Markup Language), SQL(Structured Query Language), or shell in the diff

## persistence

INV-017 | 06 data-structures | seed | every schema change declares its reverse or explicitly refuses with an irreversible-migration error | check: each migration carries a down path or raises irreversible by name
INV-018 | 06 data-structures | seed | every foreign key resolves to an existing referenced row after any data movement | check: an integrity query runs post-migration and returns 0 orphans

## sources

Researched 2026-08-03; each theme's authority, for curation:

- Power of Ten: G. Holzmann, "The Power of Ten — Rules for Developing Safety-Critical Code", https://spinroot.com/gerard/pdf/P10exp.pdf (rules 1-10; INV-001..005 derive from rules 2, 5, 7, 6, 10).
- Negative-space assertions: community term (TigerBeetle's Joran Dirk Greef; popularized by ThePrimeagen), NOT NASA-coined — the practice is Power of Ten rule 5's "conditions that should never happen"; attribution note kept here so citations stay honest. https://double-trouble.dev/post/negativ-space-programming/
- Design by Contract: B. Meyer, https://www.eiffel.com/values/design-by-contract/ and "Applying Design by Contract" (1992), https://se.inf.ethz.ch/~meyer/publications/computer/contract.pdf
- Property-based testing: Claessen & Hughes, QuickCheck (ICFP 2000); model-based testing per fast-check docs, https://fast-check.dev/docs/advanced/model-based-testing/
- Illegal states unrepresentable: Y. Minsky, Effective ML, https://www.cs.cornell.edu/courses/cs3110/2013fa/lectures/27/lecture27_Minsky_EffectiveML.pdf
- Trust-boundary validation: SEI CERT input-validation rules, https://cmu-sei.github.io/secure-coding-standards/ ; output encoding: OWASP ASVS V1 Encoding and Sanitization, https://github.com/OWASP/ASVS/blob/master/5.0/en/0x10-V1-Encoding-and-Sanitization.md
- Reversible migrations: Rails Active Record Migrations, https://guides.rubyonrails.org/active_record_migrations.html ; referential integrity: https://atlasgo.io/blog/2024/10/09/strategies-for-reliable-migrations
