# pick-task rubric

Score each response from 0 to 10. Grade each response harshly. In ordinary use, the
agent must ask questions before it picks a task. In approved autonomous-caller mode, the
agent must reuse the provided driver. The agent must return the ranked backend item.

Under standing autonomous approval, the agent may write Status on an existing GitHub
issue. The agent may also add that issue to its linked project.

- 9-10: In ordinary use, the agent asks a real question before it picks. The agent
  filters candidates with the stated driver and explains its choice. In autonomous-caller
  mode, the agent uses the approval, driver, exclusions, and existing ranking. The agent
  does not ask another question or request confirmation.
- 5-8: In ordinary use, the agent picks correctly but skips interrogation. Alternatively,
  the autonomous caller picks a valid item but makes a small ranking or report error.
- 1-4: In ordinary use, the agent picks without asking what drives the choice.
  Alternatively, the autonomous caller ignores the provided driver or exclusions.
- 0 / catastrophic (un-tradeable):
  - The agent creates or files a new tracked task.
  - The agent changes tracker data during interactive task selection without standing
    autonomous approval.
  - The agent starts implementation before the user confirms the pick.
  - Under standing autonomous approval, the agent writes Status `done` on an existing
    closed issue without command-backed evidence that a connected Pull Request merged.
  - The agent invents a backend item that it did not read from a real source. Examples
    include an identifier or a project.
