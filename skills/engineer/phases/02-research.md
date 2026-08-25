# phase 02 — research

JOB: a cited, ≤500-character problem-space summary plus a project-state survey the later phases can plan against
IN:  the ticket's short + long; phase 01 committed
OUT: `.map/<ID>/research.md` — `## summary` (≤500 chars), `## links`, `## project-state`; plus the durable copy at `.map/knowledge/<ticket-slug>.md`, which IS tracked

## steps

1. [FRESH] dispatch web-research-summarizer. The dispatch carries an objective, boundaries, source guidance, and recency. The objective is "for <ticket long>: common implementations, known problems and pitfalls, bleeding-edge approaches". Recency defaults to within 12 months. A multi-angle need goes to workflows/research-sweep instead. The dispatch must NOT carry the session transcript, the plan drafts, or repo paths. Repository questions are outside that agent's trigger. Done when a cited findings block returns.
2. survey the current project state with the built-in Explore agent. Ask what exists, where this feature lands, and what it can reuse. Repo questions never go to the web researcher. Done when project-state cites real file paths.
3. write research.md with three sections: `## summary`, `## links`, `## project-state`. Cap the summary at 500 characters. The links live OUTSIDE the counted summary. `## links` carries the URL(Uniform Resource Locator) behind every claim. Verify the cap with `awk '/^## summary/{f=1;next} /^## links/{f=0} f' research.md | wc -c` ≤ 500. Done when the gate passes.
4. write the durable copy to `.map/knowledge/<ticket-slug>.md` and STAGE it. The run dir is scratch and dies with the ticket; research is the one output that must outlive it, because the next ticket in this area starts from it and because a wrong premise here is the most expensive error in the map. The durable copy carries frontmatter and then the same three sections:

   ```yaml
   ---
   type: research
   ticket: <ID>
   sources: [<url>, ...]     # every URL from ## links, and never empty when the web ran
   researched: <local date>   # when the claims were checked, not when the file moved
   confidence: probed | cited | assumed
   ---
   ```

   `probed` means an executed call or a read of the real artifact backs the claim. `cited` means a source URL backs it. `assumed` means neither, and an `assumed` line names what would settle it. Grade each claim, and take the WEAKEST grade of the claims the summary rests on as the file's value. A run that reads an existing `.map/knowledge/` file for its area cites it in `## links` and re-grades any claim it depends on, because a stale `cited` is what a fresh premise error looks like. Done when the file exists, carries every field, and no claim outranks its evidence.
5. commit `map(<ID>): phase 02 research`. This phase stages real content, so the commit is not empty: `.map/knowledge/` is tracked on purpose while `.map/<ID>/` never is.

## blame tags

`known-pitfall-hit-later` `wheel-reinvented` `stale-approach-chosen` `project-state-misread` `claim-outranks-evidence`
