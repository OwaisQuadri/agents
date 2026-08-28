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

- 2026-08-28, real-run correction after the first live 3pm launchd fire. Two owner-
  reported gaps against the same live run: (1) `CANDIDATE_SCHEMA.category` only had
  `skill|agent|tool`, so a mining dispatch that correctly concluded "this is really a
  checker" or "this is really a workflow" per ai-author's own step-5 routing had no
  slot to report it in — the digest could never actually answer "which linters, checks,
  skills, agents, workflows, or other", the exact question the owner asked. Expanded the
  enum to `skill|agent|workflow|checker|extension|tool` and updated the Plan node's
  dispatch instructions, `dispatchPrompt`'s per-candidate category instruction, the
  Filter node's category count, and the Digest node's heading list (six headings now,
  each omitted when empty) to match. (2) Both mining dispatches (`skill-evidence-sweep`,
  `agent-candidate-scan`) failed outright on the live run — `ok: false` in the workflow
  journal, no error text captured. Reproduced live with a standalone Explore dispatch:
  it correctly located `~/.pi/agent/sessions/`, then aborted ("This operation was
  aborted") while reading several full session transcripts, which can run multi-MB each
  — confirmed by this very session's own transcript size. Added an explicit reading-
  boundedness rule to both mining dispatch objectives (grep for markers, tail the last
  ~200 lines, or bounded offset/limit reads — never an unbounded whole-file read).
  Re-verification of this fix (a second live launchd-triggered run) is the next real
  signal on whether it holds.

- 2026-08-28, second real-run correction, same evening. The bounded-reads fix above did
  NOT hold: a real re-run still showed both mining dispatches returning nothing, and a
  standalone reproduction confirmed why — `agentType: 'Explore'` aborted a second time
  ("This operation was aborted"), this time on a run doing nothing but bounded
  directory/filename listing with zero large reads, ruling out content size as the cause.
  Dropped `agentType: 'Explore'` from both mining dispatches entirely; they now run on
  the same default (session) model every other node in this workflow already uses
  successfully in every one of these same live runs (Plan, Filter, Digest never once
  failed). Updated SKILL.md's GRAPH SPEC and the evals/ mechanical routing check (c5)
  to match — c5 previously asserted mining routes to Explore, which would now be wrong
  on its own terms. A third live run is the next real signal.

## deferred

- No persistent dedup across daily runs yet — two consecutive days could surface the
  same tool-radar candidate twice, since nothing tracks what a prior digest already
  proposed. Deferred rather than built into this founding version: the plan node's
  "rotate sources daily" instruction is a partial mitigation, not a fix. Revisit once a
  few real digests exist to show whether repetition is actually a problem worth solving.
- Linux cron leg for the Arch PC (scheduler wiring only — the trigger script's herdr/
  pane/pi calls are already OS-agnostic) was explicitly deferred to a fast-follow per
  the owner's decision during planning, not part of this founding version.
