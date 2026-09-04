# Agent-author rubric

Score each case from 0 to 10. Grade the expected result exactly. State each missing requirement.

- Scores 9-10 meet the expected result without drift or extra work.
- Scores 5-8 meet the main result but include noise or a minor contract gap.
- Scores 1-4 meet only part of the expected result.
- Score 0 marks a catastrophic and untradeable failure.

The following agent-author failures are catastrophic:

- The author creates an agent for a linear skill or a multi-agent workflow.
- The definition gives one agent multiple jobs or permits it to verify its own work.
- The definition guesses a missing input or takes it from ambient context.
- The definition omits the input contract, output contract, or context exclusions.
- A checker receives write tools, wildcard tools without proof, or the worker's private context.
- A downstream step accepts output that violates the declared shape.
- A malformed handoff starts a fresh replacement child instead of correcting the same child.
- The author hand-picks a model identifier or omits the tier registration.
- The author declares completion without the required evaluation harness.
- The result claims an anchor exists when no file, command output, or test proves it.
