# phase 03 — ux-research

JOB: every user-facing decision named in precise UX(user experience) terms, with one conventional and one novel option on the record
IN:  research.md, the ticket; phase 02 committed
OUT: `.map/<ID>/ux.md` — term map, chosen pattern, rejected alternates

## steps

1. invoke /vocabulary on every fuzzy UX intent in the ticket, such as "make it smoother" or "feels off". Record the precise term, its contrasting near-synonym, and the adjustment it implies. Quote vocabulary.md verbatim. Done when no feel-word survives unmapped.
2. survey the existing implementations of this interaction pattern. Reuse the phase-02 links. Use the mobbin search tools for mobile surfaces when they are available. Done when at least one shipped example is cited.
3. HUMAN GATE UX. This gate always runs, with web search or without it, and nothing bypasses it. Run it after the survey and before any direction is chosen. STOP and present an EXPERIENCE BRIEF, never a findings dump. The brief carries three parts:
   (a) how this ticket changes the user's experience, before → after, in the mapped terms.
   (b) the proposed directions for that experience, each one tagged with its lineage. Prior inspiration is what has inspired THE USER before when he designed interfaces and experiences. Search the personal RAG(Retrieval Augmented Generation) store with `rag search`, the repo's `.map/inspiration.md`, and this skill's inspiration-seed.md for his recorded inspirations. Say which one each direction draws on. Never substitute a generic shipped example. Industry standard is the convention. Industry-disrupting is what the boldest players ship. Novel innovation is what nobody ships. Add at least one fresh-inspiration candidate: a reference NEW to his record that could inspire this surface, marked as new-to-him. The brief never only mirrors his past taste. His history anchors one direction, and the alternatives stand beside it every time. Append a new-to-him reference he adopts at the gate to the repo's `.map/inspiration.md`, so his record grows.
   (c) how we want the user to FEEL at this surface, and what we want them to DO because of it.
   Web findings, when any exist, are the citations behind the directions. Their absence changes nothing about the gate. Wait for the user to pick or adjust a direction. The choice lands in ux.md as THEIRS, and `state.json.gates.UX` records their verdict with a timestamp and their words. Done when the user has selected.
4. draft ≥1 conventional option and ≥1 novel option. Choose one and justify it in mapped terms, never in feel-words. Done when ux.md carries the term map, the choice, and the rejected alternates.
5. commit `map(<ID>): phase 03 ux-research`. A non-UI ticket records "no user-facing surface" with one line of evidence, then commits. The phase still runs.

## blame tags

`wrong-interaction-pattern` `vague-term-caused-misbuild` `near-synonym-confusion`
