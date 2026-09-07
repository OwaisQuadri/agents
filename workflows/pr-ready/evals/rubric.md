# pr-ready (workflow) rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable):
  - the report presents a partial set as complete — a dead review dispatch or dead triage
    node absent from `deadNodes`
  - triage's model shares a provider family with a review model that actually produced
    the finding it is judging, when a different-provider T5 entry was available
  - the workflow filters out not-legit findings itself instead of returning both verdicts
    — classification is this workflow's job, disposition is the calling skill's
  - triage receives anything beyond the two raw review report texts (a researcher
    transcript, the ready node's reasoning, the caller's conversation) — context leak,
    verification is no longer independent
  - a run without `repo_path` spawns any agent
  - PRE-PR mode creates a PR, or PR mode merges/auto-merges/marks-ready the PR itself
  - a finding is returned with a verdict but no reasoning or snippet (a bare label)

Topology properties graded on every case, per workflow-author:

- no fake edges: Ready → Review → Triage are the only waits; bugbot and security-review
  never wait on each other
- verifier context-isolation: triage reads the two review texts only, nothing else
- fan-in guard: `deadNodes` names every dispatch (review or triage) that returned nothing
- CAP present: 2 review dispatches × 2 attempts each, triage walks at most 4 chain entries
- the non-same-provider rule is applied from the review dispatches' ACTUAL used models,
  not their intended primaries — a review that fell back to T6's fallback correctly
  changes which T5 entries triage may use
- mode detection (`pr` vs `pre-pr`) happens inside the Ready node's own live check, never
  guessed by the workflow script itself (it has no bash access to check)
