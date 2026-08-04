# phase 02 — research

JOB: a cited, ≤500-character problem-space summary plus a project-state survey the later phases can plan against
IN:  the ticket's short + long; phase 01 committed
OUT: `.map/<ID>/research.md` — `## summary` (≤500 chars), `## links`, `## project-state`

## steps

1. [FRESH] dispatch web-research-summarizer with objective ("for <ticket long>: common implementations, known problems and pitfalls, bleeding-edge approaches"), boundaries, source guidance, recency (default: within 12 months). Multi-angle need → workflows/research-sweep instead. The dispatch must NOT carry the session transcript, plan drafts, or repo paths — repository questions are out of that agent's trigger. Done when a cited findings block returns.
2. survey the current project state with the built-in Explore agent: what exists, where this feature lands, what it can reuse. Repo questions never go to the web researcher. Done when project-state cites real file paths.
3. write research.md: `## summary` ≤500 characters (links live OUTSIDE the counted summary), `## links` carrying every claim's URL(Uniform Resource Locator), `## project-state`. Verify the cap: `awk '/^## summary/{f=1;next} /^## links/{f=0} f' research.md | wc -c` ≤ 500. Done when the gate passes.
4. commit `map(<ID>): phase 02 research`.

## blame tags

`known-pitfall-hit-later` `wheel-reinvented` `stale-approach-chosen` `project-state-misread`
