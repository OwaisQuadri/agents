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
  - a tool-radar candidate's rationale is generic ("this seems useful") with no
    reference to the real-friction grounding block and no explicit repo-stack
    grounding either, and the Filter node lets it through anyway

Topology properties graded on every case, per workflow-author:

- no fake edges EXCEPT one real barrier: plan→mining→toolRadar→filter→digest.
  mining→toolRadar is a genuine cross-item dependency (tool-radar's grounding text is
  built from mining's actual candidate content) and is the only place two dispatch
  waves run sequentially rather than together — mining dispatches run in one parallel
  wave, tool-radar dispatches run in a second parallel wave, never dispatch-by-dispatch
  sequential
- verifier context-isolation: Filter reads the raw candidate list only
- fan-in guard: `returned` counted against `expected`, gaps named in `missingLabels`
- CAP present: 2 mining + 3 tool-radar dispatches, 10 digest survivors
- mining dispatches reuse `skills/ai-author/SKILL.md`'s bounded session evidence sweep
  by reference, never duplicate its procedure inline — duplication drifts the moment
  the source procedure is tuned by ai-author's own GEPA(Genetic-Pareto prompt
  evolution) loop
- mining's friction-hunting instruction (grep transcripts for a marker repeating 2+
  times, cite the occurrences as measured evidence) is present verbatim, not softened
  into a vague "look for patterns" note
- tool-radar dispatches receive the usage-grounding block built from mining's actual
  candidates (or the explicit zero-friction variant), never a static/generic grounding
  note authored ahead of time by the plan node
