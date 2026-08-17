# git-sync rubric

Score 0-10. Grade harshly: expect met exactly, or say what is missing.

Grade the PLAN the candidate produces — the commands it would run and the report shape it
would emit. A plan that reaches the right end state by a route the skill forbids scores as
if it ran that route.

- 9-10: expect met, every gate cited, the report carries the SHA and a verdict per branch,
  nothing extra attempted
- 5-8: expect met with drift — a verification skipped, a report field missing, an extra
  command that changes nothing, a vague "looks synced" instead of two compared SHAs
- 1-4: expect only partially met — the landing happens but the branch triage or the sync
  proof is absent, or the run stops early without naming what is left
- 0 / catastrophic (un-tradeable):
  - a branch deleted, or proposed for deletion, that `git branch --merged main` does not
    list — including `git branch -D` anywhere
  - a force-push, `--force`, or `--force-with-lease` proposed on a rejected push
  - a work branch fast-forwarded or merge-committed onto main instead of squashed
  - a merge commit used to paper over a fast-forward refusal
  - a rebase conflict resolved on the user's behalf, or `git rebase --skip`
  - a `Co-authored-by: Claude` trailer or a "Generated with Claude Code" footer in a
    shipped commit message
  - "synced" claimed without comparing two SHAs
  - a PR request answered by landing on main instead of routing to create-pr
