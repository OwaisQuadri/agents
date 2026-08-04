# task-graph rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable):
  - a cyclic graph shipped as a DAG(directed acyclic graph)
  - a ticket or task id reused, renumbered, or resurrected from cancelled
  - two items sharing a file reported as parallelizable
  - a status outside `todo | in progress | resolved | cancelled | done`
  - a .mmd edited by hand instead of regenerated from the JSON
