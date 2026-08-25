# phase 21 — roadmap

JOB: the next-feature candidates explored in four buckets, innovation included, per the project's recorded taste
IN:  roadmap.json, `.map/inspiration.md` (project override; absent → `skills/engineer/inspiration-seed.md`), research.md when a run precedes this. Ideate entry: the user's ideas seed the pool, output goes to `.map/ideation/YYYY-MM-DD.md` instead
OUT: `.map/<ID>/exploration.md` (or the ideation file)

## steps

1. read the roadmap state: what is done, what this ticket or the user's ideas unlocked, and where the dependency frontier sits. Done when the unlock picture is stated.
2. generate the candidates in four ordered buckets. make-it-work holds the missing basics. make-it-right holds the refactors owed. make-it-fast holds performance. innovation asks "what would iron man do": the ambitious version nobody asked for. The weights and the taste come from inspiration.md, and the project file beats the seed. Read the run's own accumulation FIRST, before you generate anything: the ideas the user raised in session, the gate outcomes parked "for the roadmap", `.map/<ID>/parked-ticket-candidates.json` from any GATE SPLIT verdict, the adopted inspirations, and the deviations that pointed at future work. A split candidate keeps its boundary and both dependency directions verbatim until GATE D; phase 21 may enrich it and never drop it. Sweep this run's `.map/<ID>/` records and the session, so nothing he said is dropped. Done when every bucket has ≥1 candidate or a one-line justification, and every parked in-session idea appears in the pool.
3. optional: [FRESH] dispatch web-research-summarizer for a web scan on two angles. Angle one is what teams commonly build next after they ship this kind of feature. Angle two is the new innovations at the bleeding edge. Keep the objective tight to the feature domain, and keep the recency tight. The dispatch must NOT carry roadmap internals beyond the domain. Done when the scan is cited or skipped on the record.
4. write the exploration file. Each candidate carries a one-line value and its rough dependency notes. Commit `map(<ID>): phase 21 roadmap` in a run, or a plain commit in ideate mode. Done when every candidate names what it depends on.

## blame tags

`roadmap-blindspot` `refactor-debt-ignored`
