# phase 02 — research

JOB: two compressions the later phases plan against — a cited web summary and a file:line-anchored codebase snapshot — human-signed and preserved after the run
IN:  the ticket's short + long; phase 01 committed
OUT: `.map/<ID>/research.md` — `## summary` (≤500 chars, web research), `## links`, `## codebase`; durable `.map/knowledge/<topic-slug>.md`; `state.json.gates.R`

Two words, never bare "research": WEB research asks the outside world how others solve this problem. CODEBASE research asks the repo itself how the system works today. They use different agents, sources, and failure modes, and this phase does both.

## steps

1. [FRESH] web research: dispatch web-research-summarizer. The dispatch carries an objective, boundaries, source guidance, and recency. The objective is "for <ticket long>: common implementations, known problems and pitfalls, bleeding-edge approaches". Recency defaults to within 12 months. A multi-angle need goes to workflows/research-sweep instead. The dispatch must NOT carry the session transcript, plan drafts, or repo paths. Repository questions are outside that agent's trigger. Done when a cited findings block returns.
2. [FRESH] codebase research: name the vertical slices the ticket touches — the subsystems, providers, or layers its short + long imply — then dispatch one Explore agent per slice. Each dispatch carries the ticket text and its slice only, never the transcript or another slice's findings. Each slice returns what exists, how it works, and what the ticket can reuse, every claim anchored to a file:line. One slice suffices for a small ticket; deciding the slice count happens here, on the record. Done when every slice's findings carry real file:line anchors.
3. merge into research.md with three sections: `## summary`, `## links`, `## codebase`. The summary compresses the web findings; cap it at 500 characters and verify with `awk '/^## summary/{f=1;next} /^## links/{f=0} f' research.md | wc -c` ≤ 500. `## links` carries the URL(Uniform Resource Locator) behind every web claim and lives OUTSIDE the counted summary. `## codebase` merges the slice findings into one snapshot: what is true in the code, grounded in file:line anchors, with zero speculation and zero bug-hunting. A wrong line here poisons every phase that plans against it. Done when both compressions read as self-contained for a stranger.
4. merge the durable copy into `.map/knowledge/<topic-slug>.md` and STAGE it. The topic slug names stable subject matter, never a ticket id or title; update an existing topic file instead of creating one document per run. The run dir is scratch and dies with the ticket; research must outlive it because the next ticket in this area starts from it, and a wrong premise here is the most expensive error in the map. The durable copy carries frontmatter and then the same three sections:

   ```yaml
   ---
   type: research
   topics: [<stable domain term>, ...]
   source_tickets: [<ID>, ...]
   sources: [<url>, ...]
   researched: <local date>
   confidence: probed | cited | assumed
   ---
   ```

   `source_tickets` appends this run's ID as provenance without making the document ticket-specific. `sources` carries every URL from `## links` and is never empty when web research ran. `researched` records when the claims were checked, not when the file moved. `probed` means an executed call or read of the real artifact backs the claim. `cited` means a source URL backs it. `assumed` means neither, and an assumed line names what would settle it. Grade each claim and take the WEAKEST grade of the claims the summary rests on as the file's value. A run that reads an existing `.map/knowledge/` file for its area cites it in `## links` and re-grades every claim it depends on, because a stale `cited` is what a fresh premise error looks like. Done when the file exists, carries every field, and no claim outranks its evidence.
5. HUMAN GATE R: apply STANDING APPROVAL from SKILL.md over research.md plus the durable file. Show the web summary, codebase snapshot, source list, date, and confidence through /show-me. Wait for the human's verdict. Nine phases plan against this material before the next full human review at GATE B, so a wrong snapshot caught here is the cheapest possible catch. Write `state.json.gates.R` on clear.
6. commit `map(<ID>): phase 02 research`. This phase stages real content, so the commit is not empty: `.map/knowledge/` is tracked on purpose while `.map/<ID>/` never is.

## blame tags

`known-pitfall-hit-later` `wheel-reinvented` `stale-approach-chosen` `codebase-misread` `slice-missed` `claim-outranks-evidence`
