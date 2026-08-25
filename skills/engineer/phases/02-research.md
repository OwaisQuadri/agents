# phase 02 — research

JOB: two compressions the later phases plan against — a cited web-research summary of the problem space, and a codebase-research snapshot of the code that the ticket touches — human-signed before anything builds on them
IN:  the ticket's short + long; phase 01 committed
OUT: `.map/<ID>/research.md` — `## summary` (≤500 chars, web research), `## links`, `## codebase` (file:line-anchored snapshot); `state.json.gates.R`

Two words, never bare "research": WEB research asks the outside world how others solve this problem. CODEBASE research asks the repo itself how the system works today. They use different agents, different sources, and different failure modes, and this phase does both.

## steps

1. [FRESH] web research: dispatch web-research-summarizer. The dispatch carries an objective, boundaries, source guidance, and recency. The objective is "for <ticket long>: common implementations, known problems and pitfalls, bleeding-edge approaches". Recency defaults to within 12 months. A multi-angle need goes to workflows/research-sweep instead. The dispatch must NOT carry the session transcript, the plan drafts, or repo paths. Repository questions are outside that agent's trigger. Done when a cited findings block returns.
2. [FRESH] codebase research: name the vertical slices the ticket touches (the subsystems, providers, or layers its short + long imply), then dispatch one Explore agent per slice. Each dispatch carries the ticket text and its slice only — never the transcript, never another slice's findings. Each slice returns what exists, how it works, and what the ticket can reuse, every claim anchored to a file:line. One slice suffices for a small ticket; deciding the slice count happens here, on the record. Done when every slice's findings carry real file:line anchors.
3. merge into research.md with three sections: `## summary`, `## links`, `## codebase`. The summary compresses the web findings; cap it at 500 characters and verify with `awk '/^## summary/{f=1;next} /^## links/{f=0} f' research.md | wc -c` ≤ 500. `## links` carries the URL(Uniform Resource Locator) behind every web claim and lives OUTSIDE the counted summary. `## codebase` merges the slice findings into one snapshot — a statement of what is true in the code, grounded in file:line anchors, with zero speculation and zero bug-hunting. A wrong line here poisons every phase that plans against it. Done when both compressions read as self-contained for a stranger.
4. HUMAN GATE R: show research.md through /show-me and wait for the human's verdict. Nine phases plan against this file before the next full human review at GATE B, so a wrong snapshot approved here is the cheapest possible catch. Write `state.json.gates.R` on clear.
5. commit `map(<ID>): phase 02 research`.

## blame tags

`known-pitfall-hit-later` `wheel-reinvented` `stale-approach-chosen` `codebase-misread` `slice-missed`
