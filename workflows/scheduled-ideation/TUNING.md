# scheduled-ideation: tuning record

The GEPA(Genetic-Pareto prompt evolution) loop's inputs and outputs for this workflow.
`SKILL.md` never loads it.

## accepted mutations

- 2026-08-28, founding version. Built for issue #124 (a rescope of #67, closed as
  superseded — #67 was narrower: session-evidence-only, skills-only, no scheduling).
  Applied ai-author's type-decision tree directly: the trigger mechanics (launchd plist
  + shell script) are fully mechanizable and stay outside the artifact system entirely;
  the candidate-generation judgment work has no existing single owner (ai-author's own
  sweep is skill-only/evidence-only, `ideate` needs a live user, `capability-adoption`
  writes up one already-identified candidate) and fans out over ≥2 agents with a
  generate→filter shape, routing to rule 5 (workflow). Structure modeled directly on
  `workflows/research-sweep/`'s proven plan→fan-out→merge→verify shape. Mechanical run:
  6/6 non-holdout at 5.00, holdout c7 at 5.00, zero catastrophic.

## deferred

- No persistent dedup across daily runs yet — two consecutive days could surface the
  same tool-radar candidate twice, since nothing tracks what a prior digest already
  proposed. Deferred rather than built into this founding version: the plan node's
  "rotate sources daily" instruction is a partial mitigation, not a fix. Revisit once a
  few real digests exist to show whether repetition is actually a problem worth solving.
- Linux cron leg for the Arch PC (scheduler wiring only — the trigger script's herdr/
  pane/pi calls are already OS-agnostic) was explicitly deferred to a fast-follow per
  the owner's decision during planning, not part of this founding version.
