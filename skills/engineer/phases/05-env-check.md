# phase 05 — env-check

JOB: a read-only audit that every tool the later phases invoke is present, with each gap captured as a runnable fix — nothing installed now
IN:  testability.md, research.md; phase 04 committed
OUT: `.map/<ID>/env.md` (possibly empty)

## steps

1. READ-ONLY, the whole phase: version checks, `command -v`, `--dry-run`, simulator lists. No installs, no boots that persist state, no config edits. Done when `git status` shows no non-.map changes.
2. verify the toolchain, dependencies, simulators or devices, and harness binaries named by phases 02 and 04. Done when each is verified present or recorded missing.
3. each missing item → one line in env.md: `MISSING: <thing> | FIX: <exact idempotent command>`. The FIX lines become the lowest-numbered todo.sh steps in phase 12 — appended to todos, never run here. Done when every miss has a FIX line.
4. commit `map(<ID>): phase 05 env-check`.

## blame tags

`env-breakage-mid-implementation` `missing-dep-discovered-late` `simulator-absent-at-testing`
