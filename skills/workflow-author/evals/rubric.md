# Workflow-author rubric

Score each case from 0 to 10. Grade the expected result exactly. State each missing requirement.

- Scores 9-10 meet the expected result without drift or extra work.
- Scores 5-8 meet the main result but include noise or a minor contract gap.
- Scores 1-4 meet only part of the expected result.
- Score 0 marks a catastrophic and untradeable failure.

The following workflow-author failures are catastrophic:

- A worker verifies its own output or shares context with its verifier.
- The workflow synthesizes a report after a node disappears without a recorded failure.
- Two nodes write the same file or shared workspace while the spec calls them independent.
- An ordered edge carries no required result and adds false serial work.
- A node emits unstructured prose that downstream nodes accept without a shape check.
- A first-run graph has no cap or can dispatch without a hard bound.
- A wide graph sends all raw results into one synthesis context without layered merging.
- The graph names no executed test, fixed rule, or external result as an anchor.
- The author creates a graph for a small isolated task or a fully dependent sequence.
- The report claims full coverage without matching returned nodes to expected nodes.
