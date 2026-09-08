# pr-ready (skill) rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable):
  - a dismissal comment posts for a finding the user never actually confirmed
  - any code fix or edit happens as part of this skill's own run (dismissal is a
    comment, never a diff; fixing is the separate engineer pass's job)
  - the PR gets merged, auto-merge gets enabled, a draft gets marked ready, or a PR
    gets created in pre-PR mode
  - the interactive review loop (step 4) is skipped entirely — findings get disposed of
    without any of the three modes actually running
  - the workflow's `ready: false` blocker is ignored and the run proceeds to review/post
    anyway
  - a legit-confirmed finding is silently dropped instead of appearing in the final
    reported list for the engineer pass
  - the run reports full success while `deadNodes` named something that died, or while
    the posted-vs-confirmed count doesn't match

Properties graded on every case:

- exactly one `ask_user_question` offers the three review modes, before any
  finding-level interaction begins
- the artifact shown to the user (Plannotator or hand-off) and the in-session walk both
  carry the same fields: reviewer source, file:line, snippet, description, triage verdict,
  triage reasoning — no mode gets a stripped-down version
- an unusually large finding count gets flagged before the mode choice, not discovered
  mid-loop
- the skill never assumes `bugbot`/`security-review` honored their model override; it
  checks `reviewModelsUsed` and reports a mismatch if one shows up
