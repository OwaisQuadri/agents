# phase 02 — research

JOB: a cited, ≤500-character problem-space summary plus a project-state survey the later phases can plan against
IN:  the ticket's short + long; phase 01 committed
OUT: `.map/<ID>/research.md` — `## summary` (≤500 chars), `## links`, `## project-state`

## steps

1. [FRESH] dispatch web-research-summarizer. The dispatch carries an objective, boundaries, source guidance, and recency. The objective is "for <ticket long>: common implementations, known problems and pitfalls, bleeding-edge approaches". Recency defaults to within 12 months. A multi-angle need goes to workflows/research-sweep instead. The dispatch must NOT carry the session transcript, the plan drafts, or repo paths. Repository questions are outside that agent's trigger. Done when a cited findings block returns.
2. survey the current project state with the built-in Explore agent. Ask what exists, where this feature lands, and what it can reuse. Repo questions never go to the web researcher. Done when project-state cites real file paths.
3. write research.md with three sections: `## summary`, `## links`, `## project-state`. Cap the summary at 500 characters. The links live OUTSIDE the counted summary. `## links` carries the URL(Uniform Resource Locator) behind every claim. Verify the cap with `awk '/^## summary/{f=1;next} /^## links/{f=0} f' research.md | wc -c` ≤ 500. Done when the gate passes.
4. commit `map(<ID>): phase 02 research`.

## blame tags

`known-pitfall-hit-later` `wheel-reinvented` `stale-approach-chosen` `project-state-misread`
