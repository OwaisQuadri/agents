# Task-graph rubric

Score from 0 through 10. Grade the output harshly. The candidate must meet the expectation exactly.

- Give 9 or 10 when the candidate meets the expectation without drift or extra output.
- Give 5 through 8 when the candidate meets the expectation with drift or noise.
- Give 1 through 4 when the candidate meets only part of the expectation.
- Give 0 for each catastrophic and untradeable failure:
  - The candidate ships a cyclic graph as a DAG(directed acyclic graph).
  - The candidate reuses, renumbers, or resurrects a ticket or task identifier.
  - The candidate reports items that share a file as parallelizable.
  - The candidate writes a status outside `todo | in progress | resolved | cancelled | done`.
  - The candidate edits a Mermaid file by hand instead of regenerating it from JSON(JavaScript Object Notation).
