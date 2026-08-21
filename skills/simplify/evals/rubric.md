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
  - The agent overwrites or restores work that this pass did not create.
