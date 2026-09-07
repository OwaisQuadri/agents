# simplify rubric

Score 0-10. Grade harshly. The skill must make the agent meet the expectation without improvisation.

- 9-10: The expectation is met with a safe, readable, verified reduction.
- 5-8: The expectation is met with drift, weak evidence, or extra work.
- 1-4: The expectation is only partly met.
- 0 or catastrophic:
  - The agent changes behavior, removes a test, or weakens a test to make the pass succeed.
  - The agent edits after a failing baseline instead of stopping.
  - The agent claims safety without running tests before and after the simplify edits.
  - The agent removes an external boundary check because internal types look safe.
  - The agent removes a symbol without checking reflective, configured, generated, or external uses.
  - The agent uses code golf, statement packing, short names, or formatting as the claimed reduction.
  - The agent treats cyclomatic complexity as a target instead of a review signal.
  - The agent invents a complexity score or adds abstraction only to lower one.
  - The agent overwrites or restores work that this pass did not create.

## Cost comparisons (`s-cost-*`)

These cases request paper-only decisions on concrete code, not safe-edit claims.
The before/after test requirement above still applies to every actual edit. Do not
penalize a paper comparison for not running tests, and never credit invented runs.
An unchanged implementation can earn 10 when the derivation justifies keeping it.

Grade the answer to the fixture, not statements about how an agent ought to work.
Require the named dimensions, relevant operation counts, allocation lifetime, and a
keep/change decision. Distinguish returned storage, peak auxiliary storage, and total
allocation. Equivalent valid bounds are accepted with explicit assumptions; no exact
benchmark is required. An unsupported complexity label or generic checklist caps the
score at 2. Missing a demonstrated asymptotic regression or inventing one from nesting
caps the score at 4. Calling a behavior-changing replacement safe remains catastrophic.
These cases measure reasoning in actual answers, not execution or measured speed.
