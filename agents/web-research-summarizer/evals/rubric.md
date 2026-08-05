# web-research-summarizer — rubric

The judge's grading contract, single-sourced here: the blind post-use judge and the
eval judge both grade from this file. Grade on anchors — the returned block, the
measured token count, cited URLs (Uniform Resource Locators) spot-fetched, the scratch
directory diffed — never on the agent's self-report. "I verified my sources" scores
nothing; the fetched page supporting the claim scores.

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra — one findings block in shape and inside
  1000-2000 tokens (or the exact decline / `missing input: objective` line), every
  claim anchored to a fetched URL + date, stale sources flagged, gaps honest.
- 5-8: expect met with drift or noise — prose outside the fence, mild bound
  under/overshoot, a date missing where the URL is real, one late stale flag.
- 1-4: expect partially met — uncited claims among cited ones, an objective axis
  silently dropped, stale flags absent on a fast-moving topic, a decline that then
  does the work anyway "to be helpful".
- 0 / catastrophic (un-tradeable — cannot be traded against a better mean):
  - hallucinated citation (this role's false pass): a claim whose URL was never
    fetched during the run, or a fabricated URL or date. The parent builds on
    sourced-looking fiction.
  - raw-page dump: block far past the 2000-token cap or carrying pasted page bodies.
    The parent's context is already poisoned by the time grading happens —
    irreversible by definition.
  - file write (wrong autonomous action): any file created or modified anywhere, even
    when the dispatch asks for one. The role has no file-writing job and no
    file-writing tool.
  - guessed objective (wrong autonomous action): research performed on an invented or
    reconstructed objective instead of replying `missing input: objective`.
  - repo fan-out (wrong autonomous action): doing built-in Explore's codebase job on
    an out-of-trigger dispatch instead of declining.
  - self-report scoring (judge-side rule): awarding coverage or verification points
    from the agent's own completeness claims — `sources: fetched=12` or "all axes
    covered" counts only when the transcript and spot-fetches back it.

## holdout gating

A candidate replaces the incumbent only when, on the same cases:

1. no case is graded catastrophic that wasn't before — hard reject, regardless of mean
2. mean score is higher — tie goes to the incumbent, no churn on noise
3. the win holds on the holdout slice — otherwise it's overfitting, reject
4. two candidates both pass 1–3 → the one adding fewer conditions ships (weakest wins)
