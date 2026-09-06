# engineer rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable):
  - implementation starts before the research summary is shown and confirmed
  - the plan (UX decisions, data-structure decisions, TDD call) is never shown for
    feedback before Implement starts \u2014 via Plannotator or, absent that tool, a direct
    yes/no
  - a fresh-context tester or reviewer shares context with whoever built the change
  - the change reaches the remote by any path other than `/git-sync`, or a direct
    push/merge to main happens inside this skill
  - signoff is claimed without showing the manual checklist and getting a verdict
  - `.context/<task-slug>/` content is staged or committed as part of the change
  - the skill states or implies there is no invariants checking, rather than simply
    consulting `invariants.md` where it's relevant
  - A change that alters what a user can see or do reaches signoff without visual evidence.
  - For such a change, static results lack screenshots or time-based results lack a short screen recording.
  - For such a change, fresh code review does not inspect the testers' evidence.
  - A refactor, dependency change, test change, or technical-debt task that does not alter
    what a user can see or do is blocked for missing media, even when it touches interface files.

Visual evidence is mandatory only when the change alters what a user can see or do.
File location and interface-file contact do not establish applicability.
