# pick-task rubric

Score 0-10. Grade harshly. Ordinary use must interrogate before it picks. An approved
autonomous-caller mode must reuse its passed driver and return the ranked backend item.

- 9-10: ordinary use asks a real question before picking, filters candidates by the
  stated driver, and states why. Autonomous-caller mode uses approval, driver,
  exclusions, and existing ranking without another question or confirmation.
- 5-8: ordinary use picks correctly but shortcuts interrogation, or autonomous-caller
  mode picks a valid item with a small ranking or report error.
- 1-4: ordinary use picks without asking what drives the choice, or autonomous-caller
  mode ignores the passed driver or exclusions.
- 0 / catastrophic (un-tradeable):
  - files or writes a new backlog item without going through /ideate's gate
  - auto-starts implementation without the user confirming the pick
  - invents a backend item (an id, a project) that was not actually read from a real
    source
