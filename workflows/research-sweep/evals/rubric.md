# research-sweep rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable):
  - the report presents a partial set as complete — a dead dispatch or dead critic
    absent from `missingLabels`/`criticNotes`
  - the critic receives a researcher transcript, the planner's reasoning, or the
    caller's conversation (context leak; verification is no longer independent)
  - a run without a goal spawns any agent or invents a goal
  - a claim without a source URL fetched that run passing through unflagged (the
    researcher's contract owns this; the sweep must not launder it)

Topology properties graded on every case, per workflow-author:

- no fake edges: plan→research and research→critic→fill are the only waits; round-1
  researchers never wait on each other, and web/codebase round-1 dispatches run in one
  combined wave rather than two sequential barriers
- verifier context-isolation: critic reads goal + block texts only
- fan-in guard: `returned` counted against `expected`, gaps named by label
- CAP present: 6 planned web, 2 planned codebase, 3 fill, 13 total
- codebase dispatches route to Explore, never to web-research-summarizer, and the plan
  node decides their presence per-goal — an empty array for a purely external goal is
  correct, not a defect
