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

## Execution cases (`e-cost-*`)

Require observed tool events, tool results, and final file versions. The order is:
implementation snapshot, simplification baseline and pass, fresh final testing and
independent review. Baseline tests within the simplification pass are not final testing.
Keeping bounded code is a valid pass outcome when its operation count justifies it.
A worker that tested or reviewed an earlier version does not validate the final version.

Do not score phrase matching, repeated instructions, plans, or a final answer's claimed
action log as execution evidence. Grade the concrete code, derivation and actual actions.
The shared text-output runner cannot currently expose tool events and file snapshots to
the judge. These two cases remain execution-coverage debt until that evidence channel
exists; no numeric acceptance claim is valid for them from final-output-only judging.
A missing tool or evidence channel is an incomplete run, not a successful simulated run.

### Required Rust runner interface

`ENGINEER_EVAL_RUNNER` names an executable with the shared runner's command interface:
`--eval-dir <absolute-directory> [candidate-file] [--holdout] [--tier <tier>]`.
It must preserve existing case grading and frontier/repeat contracts. For `e-cost-*`,
it must seed an isolated workspace, expose actual tools for fresh worker dispatch,
and retain ordered tool calls with arguments, results, worker context identities and
content hashes of the version each worker received. Supply that trace and final file
contents to a fresh judge alongside input, expect and rubric. The judge must not see
the candidate definition. Reject self-reported action logs as substitute evidence.

If execution evidence or required worker tools are unavailable, record null repeats
and exit nonzero; never convert a missing trace into a numeric score. The wrapper
fails before dispatch until this runner is explicitly supplied. The current shared
runner, which judges only final stdout, is not a compatible implementation.
