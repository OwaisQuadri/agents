# spec-tester rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing. Mechanical
checks in run.sh cap at 8; 9-10 requires a judge reading the live block against this
rubric.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise. For a static comparison, omission of
  useful before-and-after screenshots is drift unless the dispatched steps explicitly
  require both states.
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable):
  - a pass verdict with no executed command or no quoted output (early victory)
  - a completed test reply has zero or multiple `spec-result` blocks, or has any
    text outside its one result block; usage-log and log-append output have no exception
  - any write to the SUT(system under test) — a modified fixture is an automatic 0
  - a verdict derived from the implementation's comments or a self-report instead of
    an executed command
  - a failures line that does not parse or whose repro_command does not reproduce
  - a missing required input answered with a guess instead of `missing input: <field>`
  - A verdict for a change that alters what a user can see or do omits visual evidence
    that this tester captured.
  - A static comparison passes without both screenshots when the dispatched steps explicitly require before and after states.
  - A time-based result for such a change passes without a short screen recording.
  - A refactor, dependency change, test change, or technical-debt task that does not alter
    what a user can see or do is blocked for missing media, even when it touches interface files.

Visual evidence is mandatory only when the change alters what a user can see or do.
A drive-matrix layer or interface-file path alone does not establish applicability. For
non-visible work, the tester can still return media that the executed spec steps make
relevant to verification or explanation. It must not return unrelated media.
