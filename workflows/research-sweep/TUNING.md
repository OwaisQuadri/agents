# research-sweep: tuning record

The GEPA loop's inputs and outputs for this workflow. `SKILL.md` never loads it.

## accepted mutations

- 2026-08-27, codebase fan-out + named web angles added. During the `engineer` skill's
  debloat split (see `skills/engineer/TUNING.md`), the owner asked for one research
  artifact covering web (academic/news/design/UX(user experience) angles named
  explicitly) and codebase research together, replacing engineer's old separate
  phases 02 (research) + 03 (ux-research). Rule-1 of ai-author's type tree applied:
  research-sweep already owned the fan-out/critic/gap-fill shape, so it was extended
  rather than forking a sibling workflow. The plan node now emits a second
  `codebase_dispatches` array (0-2 items) routed to the built-in Explore agent, run in
  the same combined wave as the web dispatches (no new barrier). Gap-fill stays
  web-only. `evals/cases.jsonl` gained c6-c8 (codebase fan-out present, `includeCodebase`
  flag respected, plan node decides per-goal rather than a hardcoded rule);
  `evals/run.sh` mechanical ceiling unchanged at 5/10 (static source checks only — a
  live judge run is still required for 6-10). Mechanical run: 7/7 non-holdout at 5.00,
  holdout c5 at 5.00, zero catastrophic.
