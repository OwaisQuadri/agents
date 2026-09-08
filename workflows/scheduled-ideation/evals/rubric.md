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
  - the prompt-snippet audit exposes prompt text, response text, raw records,
    transcript excerpts, session identifiers, or session paths
  - the prompt-snippet audit treats missing use evidence as removal evidence, proposes
    an add from fewer than two parent sessions, or proposes a merge/removal without
    measured co-use, conflict, or overlap with an existing skill

Topology properties graded on every case, per workflow-author:

- no fake edges EXCEPT one real barrier: plan→mining→toolRadar→filter→digest.
  mining→toolRadar is a genuine cross-item dependency (tool-radar's grounding text is
  built from mining's actual candidate content) and is the only place two dispatch
  waves run sequentially rather than together — the three plan-created mining jobs and
  the fixed prompt-snippet audit run in one parallel wave, then tool-radar dispatches
  run in a second parallel wave, never dispatch-by-dispatch sequential
- verifier context-isolation: Filter reads the shared raw candidate list only
- fan-in guard: the fixed `prompt-snippet-audit` label participates in `expected`,
  `returned`, and `missingLabels` exactly like every other generate job; any
  plan-created label collision is renamed before dispatch
- CAP present: 3 plan-created mining + 1 fixed prompt-snippet audit + 3 tool-radar
  dispatches, 10 digest survivors
- mining dispatches reuse `skills/ai-author/SKILL.md`'s bounded session evidence sweep
  by reference, never duplicate its procedure inline — duplication drifts the moment
  the source procedure is tuned by ai-author's own GEPA(Genetic-Pareto prompt
  evolution) loop
- mining's friction-hunting instruction (grep transcripts for a marker repeating 2+
  times, cite the occurrences as measured evidence) is present verbatim, not softened
  into a vague "look for patterns" note
- the fixed prompt-snippet audit inspects current files under
  `pi/extensions/prompt-snippets/snippets/`, caps direct-use reads at 500 records from
  the latest 30 days at the configured or default agent directory, and uses grep-first
  bounded reads across at most 200 parent-session transcripts active in that period
  for equivalent manually typed user requests
- direct uses and equivalent manual requests stay distinct; absence stays unknown; an
  add needs repeated evidence from at least two parent sessions; a merge or removal
  needs measured co-use, conflict, or overlap with an existing skill; weak evidence
  returns zero candidates
- prompt-snippet audit outputs contain conclusions and aggregate evidence only, use
  the literal aggregate source label instead of a path, and never contain prompt or
  response text, raw records, transcript excerpts, session identifiers, or session
  paths; audit candidates reach the private filter and digest but never tool-radar
  web-research dispatches
- tool-radar dispatches receive the usage-grounding block built from mining's actual
  candidates (or the explicit zero-friction variant), never a static/generic grounding
  note authored ahead of time by the plan node
