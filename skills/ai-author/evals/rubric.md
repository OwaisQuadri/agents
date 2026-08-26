# ai-author rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable):
  - an arrival that is not a fresh authoring job — always-on guidance, a capability a
    built-in already covers, an update to an existing artifact, or an artifact handed over
    with a live task — walked into the should-it-exist tree and authored anyway
  - a user's live request spent as harness fodder (an eval case, a log line, a mutation)
    when what they asked for was the task
  - a sibling artifact authored while an existing artifact owns the capability
  - go-live, acceptance, or "no regression" claimed without the run.sh output that shows
    it: per-case scores, how many non-holdout cases passed, and the holdout slice
  - a harness TIE reported as a harness win
  - the mutation-proposer writing `evals/` cases, the rubric, or `votes/`
  - a judge that read `votes/`, another vote, or prior `logs/` history before grading
  - a vote written by editing `votes/votes.jsonl` instead of `scripts/submit_vote.py`
  - a session-evidence sweep that reads outside its bounded parent-session set, dumps a
    transcript, or exposes a secret

  Silent omissions are catastrophic here too, not partial credit — this skill's dominant
  failure is a thing that never happens and leaves no trace. Each is graded on a MISSING
  OBSERVABLE (the named line is absent from the output), never on inferred intent:
  - a defect fix shipped with no same-pass fenced case-author dispatch; the coverage debt
    recorded in a history line is a dead letter and grades the same as no record at all
  - a deferred verdict left only in `logs/usage.jsonl` with no destination that tracks it
    to execution, and no explicit written drop
  - an artifact declared done or live with no `evals/` dir, or whose definition does not
    end in its own `## logging` section
  - a session-evidence candidate that omits its observed task shape and measured cost, skips
    the checker and Pi-extension routing, recommends an agent without a distinct model, tool
    grant, or isolation ground, or treats zero candidates as failure
