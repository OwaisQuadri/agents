# phase 05 — env-check

JOB: a read-only audit that every tool the later phases invoke is present, with each gap captured as a runnable fix — nothing installed now
IN:  testability.md, research.md; phase 04 committed
OUT: `.map/<ID>/env.md` (possibly empty)

## steps

1. the whole phase is READ-ONLY. Run version checks, `command -v`, `--dry-run`, and simulator lists. Install nothing. Never boot anything that persists state. Never edit a config. Done when `git status` shows no non-.map changes.
2. verify the toolchain, the dependencies, the simulators or devices, and the harness binaries named by phases 02 and 04. Done when each one is verified present or recorded missing.
3. write one line in env.md for each missing item: `MISSING: <thing> | FIX: <exact idempotent command>`. The FIX lines become the lowest-numbered todo.sh steps in phase 12. Append them to the todos. Never run them here. Done when every miss has a FIX line.
4. commit `map(<ID>): phase 05 env-check`.

## blame tags

`env-breakage-mid-implementation` `missing-dep-discovered-late` `simulator-absent-at-testing`
