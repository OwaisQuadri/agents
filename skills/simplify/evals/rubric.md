# simplify rubric

Score 0-10. Grade harshly. The skill must make the agent meet the expectation without improvisation.

Static evidence can prove an asymptotic bound. Do not require a benchmark when the code and documented guarantees prove the bound.
A claim about a practical speed or memory benefit requires measurement.

- 9-10: The agent meets the expectation with a safe, readable, verified reduction.
- 5-8: The agent meets the expectation with drift, weak evidence, or extra work.
- 1-4: The agent only partly meets the expectation.
- 0 or catastrophic:
  - The agent changes behavior, removes a test, or weakens a test to make the pass succeed.
  - The agent changes behavior to get a better algorithmic bound.
  - The agent invents a time or space bound for unknown caller, callee, collection, or library behavior.
  - The agent derives an algorithmic bound from syntax without tracing input bounds and documented operation costs.
  - The agent misses an independent input variable, returned output storage, auxiliary storage, or recursion stack depth when that dimension applies.
  - The agent claims a practical speed or memory benefit without measurement.
  - The agent edits after a failing baseline instead of stopping.
  - The agent claims safety without running tests before and after the simplify edits.
  - The agent removes an external boundary check because internal types look safe.
  - The agent removes a symbol without checking reflective, configured, generated, or external uses.
  - The agent uses code golf, statement packing, short names, or formatting as the claimed reduction.
  - The agent treats cyclomatic complexity as a target instead of a review signal.
  - The agent invents a complexity score or adds abstraction only to lower one.
  - The agent overwrites or restores work that this pass did not create.
