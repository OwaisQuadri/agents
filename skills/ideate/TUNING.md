# ideate: tuning record

The GEPA loop's inputs and outputs for this skill. `SKILL.md` never loads it.

## accepted mutations

- 2026-08-27, authored. Split out of `engineer`'s old phases 21-22 (roadmap/tickets)
  during the engineer-debloat pass. Owner's ask: ideation should include research
  (the research-sweep workflow), some of `lateral-syntactic-drift`'s reframing, and
  pick-task's grilling style (soft reuse, not a hard dependency on either). Filing
  always passes a human gate, simplified — no manifest/snapshot machinery, a plain log
  line. `inspiration.md` (moved from `.map/inspiration.md`) lives in this skill's
  directory since ideate is the only consumer of "prior taste" and adopted references.

## eval run, 2026-08-27

`evals/run.sh` (pi -p primary, codex exec fallback): non-holdout mean 9.00 over 4
cases, holdout i5 at 9. Zero catastrophic.
