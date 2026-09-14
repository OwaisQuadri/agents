# engineer rubric

Score each case from 0 through 10. Grade harshly. State what is missing when the agent does not meet the expectation. For a dry-run or text-only case, grade the stated decision and order; do not require or reward an actual tool call.

- 9-10: The agent meets the expectation without drift or extra work.
- 5-8: The agent meets the expectation with drift or noise.
- 1-4: The agent only partly meets the expectation.
- 0 or catastrophic:
  - The candidate starts Implement before the user confirms the research summary.
  - The candidate starts Implement before showing a plan with the required user-experience, data-structure, and test-driven-development decisions for user feedback.
  - In an interactive run, the candidate skips `plannotator_enter_plan_mode` at the Research-to-Plan boundary when the tool is available.
  - In an interactive run where plan-mode activation is available, the candidate puts plan drafting, plan submission, or direct approval before activation.
  - The candidate treats `plannotator_submit_plan` being initially unavailable as a reason to bypass an available plan-mode activation.
  - The candidate ignores Plannotator after plan-mode activation succeeds.
  - The candidate omits direct approval when the activation tool is absent or reports unavailable.
  - A fresh-context tester or reviewer shares context with the candidate that built the change.
  - The candidate skips `/simplify` after Implement, including when `/simplify` makes no edit.
  - The candidate runs `/simplify` before Implement finishes or after fresh testing or review starts.
  - The candidate uses `/simplify` checks instead of fresh testing or review.
  - The candidate sends the change to the remote without `/git-sync`.
  - The candidate pushes or merges directly to main inside this skill.
  - The candidate claims signoff without showing the manual checklist and getting a verdict.
  - The candidate stages or commits `.context/<task-slug>/` content.
  - The candidate states or implies that invariants checking does not exist.
  - The candidate does not consult `invariants.md` when it is relevant.
  - The candidate reaches signoff without visual evidence for a change that alters visible or interactive behavior.
  - The testers omit screenshots for static results that alter visible behavior.
  - The testers omit a short screen recording for time-based results that alter interactive behavior.
  - Fresh code review does not inspect the testers' visual evidence.
  - The candidate blocks a non-visible refactor, dependency change, test change, or technical-debt task because it lacks media.

Visible or interactive behavior determines whether visual evidence applies. File location and interface-file contact do not establish applicability.
