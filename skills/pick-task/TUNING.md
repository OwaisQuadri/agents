# pick-task: tuning record

The GEPA loop's inputs and outputs for this skill. `SKILL.md` never loads it.

## accepted mutations

- 2026-08-27, authored. Split out of `engineer`'s old phase 01 (setup/ticket-selection)
  during the engineer-debloat pass (see `skills/engineer/TUNING.md`). Owner's ask:
  task picking should be interactive and standalone, based on what's needed in the
  moment — grilled, not read off a roadmap — and willing to work on bigger, coarser
  tasks for higher velocity than the old atomic-ticket model. Probes GitHub
  Issues/Projects and Linear before falling back to root `roadmap.json`. Filing a new
  item is explicitly out of scope here (that's `/ideate`'s gate) — pick-task only
  selects among what already exists or hands off to ideate when nothing does.

## eval run, 2026-08-27

`evals/run.sh` (pi -p primary, codex exec fallback): non-holdout mean 8.75 over 4
cases, holdout p5 at 9. Zero catastrophic.
