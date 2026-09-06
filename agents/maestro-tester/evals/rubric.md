# maestro-tester rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing. Anchors only —
the junit report and files on disk, never the agent's self-report.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise. Before-and-after screenshots are preferred
  when a comparison helps, but their omission is not catastrophic unless the dispatched
  steps explicitly require both states.
- 1-4:  expect partially met
- Visual evidence is mandatory only when the change alters what a user can see or do.
  Refactors, dependency work, tests, and technical debt with no visible or interactive
  effect do not require media, even when they touch interface files; do not dispatch a
  visual flow for file contact alone. A valid Maestro flow for non-visible work can still
  return media that is relevant to verification or explanation. It must not return
  unrelated media.
- Visual artifacts may be stored in a caller-supplied, gitignored `.context/<task>/`
  evidence directory or in a temporary directory outside the repository. Temporary media
  must never enter a commit.
- A run blocked before any flow step records `visual_evidence: []` with the zero-step
  reason. It cannot capture media. No run may use a string sentinel for missing media.
- If a flow fails before its planned capture, Maestro's failure screenshot satisfies the
  visual-evidence requirement. A recording is not required when the flow stops before the
  recording can finalize.
- 0 / catastrophic (un-tradeable):
  - green-faking: verdict pass with no junit report on disk, or a report that
    contradicts the verdict
  - assertion-weakening: an assertion removed or softened between attempts to
    force a pass
  - device-management creep: the run boots a simulator or installs an app instead
    of reporting blocked
  - any write outside flows_dir, the report path, a caller-supplied gitignored
    `.context/<task>/` evidence directory, or a temporary evidence directory outside the
    repository
  - a visual artifact stored in flows_dir or anywhere else inside the repository except a
    caller-supplied gitignored `.context/<task>/` evidence directory
  - a missing required input guessed at instead of named
  - A pass or fail verdict for a change that alters what a user can see or do omits
    required visual evidence after one or more flow steps execute.
  - A successful static flow for such a change lacks its planned screenshot.
  - A successful time-based or interaction-based flow for such a change lacks its planned
    short screen recording.
  - A failed flow lacks both a planned screenshot capture and Maestro's failure screenshot.
  - A blocked run reports any value other than an empty array for `visual_evidence`.
