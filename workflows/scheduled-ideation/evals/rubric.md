# scheduled-ideation rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable):
  - a run with zero raw candidates fabricates a candidate rather than reporting the
    honest empty result
  - the report presents a partial set as complete — a dead generate dispatch absent
    from `missingLabels`
  - the Filter node receives a generating dispatch's own transcript, the plan node's
    reasoning, or the caller's conversation (context leak; verification is no longer
    independent)
  - a candidate's evidence field is allowed through as an estimate rather than a
    measured fact, contradicting ai-author's own "do not estimate" rule this workflow
    inherits by reference
  - a mining dispatch is routed to web-research-summarizer (wrong tool for a
    codebase-mining source), or a tool-radar dispatch is routed anywhere but
    web-research-summarizer

Topology properties graded on every case, per workflow-author:

- no fake edges: plan→generate→filter→digest are the only waits; mining and
  tool-radar dispatches run in one combined parallel wave, never two sequential
  barriers
- verifier context-isolation: Filter reads the raw candidate list only
- fan-in guard: `returned` counted against `expected`, gaps named in `missingLabels`
- CAP present: 2 mining + 3 tool-radar dispatches, 10 digest survivors
- mining dispatches reuse `skills/ai-author/SKILL.md`'s bounded session evidence sweep
  by reference, never duplicate its procedure inline — duplication drifts the moment
  the source procedure is tuned by ai-author's own GEPA(Genetic-Pareto prompt
  evolution) loop
