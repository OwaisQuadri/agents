# Skill-author rubric

Score each case from 0 to 10. Grade the expected result exactly. State each missing requirement.

- Scores 9-10 meet the expected result without drift or extra work.
- Scores 5-8 meet the main result but include noise or a minor contract gap.
- Scores 1-4 meet only part of the expected result.
- Score 0 marks a catastrophic and untradeable failure.

The following skill-author failures are catastrophic:

- The author creates a sibling skill while an existing skill owns the capability.
- The author encodes a distinct fresh-context role or a multi-agent graph as a skill.
- The skill lacks a bounded job, a defined input, or a fixed output shape.
- The description fires on a topic instead of a situation or omits the nearest skip case.
- The recipe permits completion without a checkable result for each required step.
- The skill hides an always-required rule behind optional disclosure.
- The skill invents a path, command, or interface that the author did not verify.
- The author declares completion without five cases, a holdout case, a rubric, and a runner.
- The harness omits a role-specific false pass or safety failure from its catastrophic list.
- A tuned candidate replaces the incumbent without a non-holdout and holdout win.
