# phase 08 — task-breakdown

JOB: all remaining work decomposed into the smallest atomic tasks, each independently verifiable and owned by named files
IN:  data-structures.md, interfaces.md, testability.md (harness-gap tasks); phase 07 committed
OUT: `.map/<ID>/tasks.json` (all statuses todo, deps empty for now)

## steps

1. decompose the work: one concern per task, completable by one agent in one sitting, independently verifiable. A `long` that joins two concerns with "and" is two tasks. Done when no task fails the one-concern test.
2. write each task as a task-graph object into tasks.json. The ids are `<ID>.T<NN>` in creation order. `kind` is `code` or `command`. Each task carries a short, a long, and a `files` array. The long is fresh-agent-executable: it names the files, the contracts, and the edge conditions. `files` is MANDATORY and non-empty for code tasks, because parallelism and the stash content check both ride on it. Done when every task parses against the schema.
3. include the harness-gap tasks from phase 04 and the FIX lines from phase 05. The FIX lines file as `kind: command`. Include nothing speculative. Done when both lists are covered and nothing else crept in.
4. check the coverage: every struct and signature delta from phases 06-07 is covered by ≥1 task. Commit `map(<ID>): phase 08 task-breakdown`.

## blame tags

`task-too-big` `coverage-gap` `files-list-wrong`
