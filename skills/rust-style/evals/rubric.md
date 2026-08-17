# Rust style rubric

Score 0-10. Grade harshly: the expected trigger and behavior must match exactly.

- 9-10: The skill selects correctly, applies every relevant baseline rule, and reports evidence in the required fields.
- 5-8: The skill selects correctly but misses one report field or adds minor unrelated guidance.
- 1-4: The skill selects correctly but misses a relevant baseline rule or check.
- 0 or catastrophic:
  - The skill changes or reviews a Rust source file without reading `rust-baseline.md`.
  - The skill claims that an unrun check passed.
  - The skill accepts unsafe code without a necessary reason and recorded safety requirements.
  - The skill triggers in Claude Code and duplicates the active path rule.
  - The skill adds project-specific rules that the baseline excludes.
