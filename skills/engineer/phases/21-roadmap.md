# phase 21 — roadmap

JOB: the next-feature candidates explored in four buckets, innovation included, per the project's recorded taste
IN:  roadmap.json, `.map/inspiration.md` (project override; absent → `skills/engineer/inspiration-seed.md`), research.md when a run precedes this. Ideate entry: the user's ideas seed the pool, output goes to `.map/ideation/YYYY-MM-DD.md` instead
OUT: `.map/<ID>/exploration.md` (or the ideation file)

## steps

1. read the roadmap state: what is done, what this ticket (or the user's ideas) unlocked, where the dependency frontier sits. Done when the unlock picture is stated.
2. generate candidates in four ordered buckets — make-it-work (missing basics), make-it-right (refactors owed), make-it-fast (performance) — then innovation: "what would iron man do", the ambitious version nobody asked for. Weights and taste come from inspiration.md; the project file beats the seed. Done when every bucket has ≥1 candidate or a one-line justification.
3. optional [FRESH] web-research-summarizer for a bleeding-edge scan — objective tight to the feature domain, recency tight; the dispatch must NOT carry roadmap internals beyond the domain. Done when cited or skipped on the record.
4. write the exploration file: each candidate with a one-line value and its rough dependency notes. Commit (`map(<ID>): phase 21 roadmap` in a run; a plain commit in ideate mode). Done when every candidate names what it depends on.

## blame tags

`roadmap-blindspot` `refactor-debt-ignored`
