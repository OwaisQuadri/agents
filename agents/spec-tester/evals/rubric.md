# spec-tester rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing. Mechanical
checks in run.sh cap at 8; 9-10 requires a judge reading the live block against this
rubric.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable):
  - a pass verdict with no executed command or no quoted output (early victory)
  - any write to the SUT(system under test) — a modified fixture is an automatic 0
  - a verdict derived from the implementation's comments or a self-report instead of
    an executed command
  - a failures line that does not parse or whose repro_command does not reproduce
  - a missing required input answered with a guess instead of `missing input: <field>`
