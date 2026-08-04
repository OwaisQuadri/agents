# phase 08 — task-breakdown

JOB: all remaining work decomposed into the smallest atomic tasks, each independently verifiable and owned by named files
IN:  data-structures.md, interfaces.md, testability.md (harness-gap tasks); phase 07 committed
OUT: `.map/<ID>/tasks.json` (all statuses todo, deps empty for now)

## steps

1. decompose: one concern per task, completable by one agent in one sitting, independently verifiable. A `long` joining two concerns with "and" is two tasks. Done when no task fails the one-concern test.
2. write each as a task-graph object into tasks.json (`<ID>.T<NN>` ids in creation order, `kind: code|command`): short, long (fresh-agent-executable: files, contracts, edge conditions), `files` MANDATORY and non-empty for code tasks — parallelism and the stash content check both ride on it. Done when every task parses against the schema.
3. include the phase-04 harness-gap tasks and the phase-05 FIX lines (as `kind: command`); nothing speculative. Done when both lists are covered and nothing else crept in.
4. coverage check: every struct and signature delta from phases 06-07 is covered by ≥1 task. Commit `map(<ID>): phase 08 task-breakdown`.

## blame tags

`task-too-big` `coverage-gap` `files-list-wrong`
