# phase 03 — ux-research

JOB: every user-facing decision named in precise UX(user experience) terms, with one conventional and one novel option on the record
IN:  research.md, the ticket; phase 02 committed
OUT: `.map/<ID>/ux.md` — term map, chosen pattern, rejected alternates

## steps

1. invoke /vocabulary on every fuzzy UX intent in the ticket ("make it smoother", "feels off"): record the precise term, its contrasting near-synonym, and the adjustment it implies, quoting vocabulary.md verbatim. Done when no feel-word survives unmapped.
2. survey existing implementations of this interaction pattern — reuse phase-02 links; mobbin search tools when available for mobile surfaces. Done when at least one shipped example is cited.
3. draft ≥1 conventional option and ≥1 novel option; choose and justify in mapped terms, not feel-words. Done when ux.md carries the term map, the choice, and the rejected alternates.
4. commit `map(<ID>): phase 03 ux-research`. Non-UI tickets: record "no user-facing surface" with one line of evidence and commit — the phase still runs.

## blame tags

`wrong-interaction-pattern` `vague-term-caused-misbuild` `near-synonym-confusion`
