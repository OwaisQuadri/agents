# phase 08 — task-breakdown

JOB: all remaining work decomposed into the smallest atomic tasks, each independently verifiable and owned by named files
IN:  data-structures.md, interfaces.md, call-stacks.md, testability.md (harness-gap tasks); phase 07 committed
OUT: `.map/<ID>/tasks.json` (all statuses todo, deps empty for now)

## steps

1. decompose the work: one concern per task, completable by one agent in one sitting, independently verifiable. A `long` that joins two concerns with "and" is two tasks. Done when no task fails the one-concern test.
2. write each task as a task-graph object into tasks.json. The ids are `<ID>.T<NN>` in creation order. `kind` is `code` or `command`. Each task carries a short, a long, and a `files` array. The long is fresh-agent-executable: it names the files, the contracts, and the edge conditions. `files` is MANDATORY and non-empty for code tasks, because parallelism and the stash content check both ride on it. Done when every task parses against the schema.
3. include the harness-gap tasks from phase 04 and the FIX lines from phase 05. The FIX lines file as `kind: command`. Include nothing speculative. Done when both lists are covered and nothing else crept in.
4. count the tasks. Over 25, run conditional HUMAN GATE SPLIT. Apply STANDING APPROVAL from SKILL.md over the task count, proposed boundary, tasks on both sides, and which side this ticket keeps. STOP and put that material to the user; he decides split or continue, and `state.json.gates.SPLIT` records the verdict, approval number, and immutable snapshot. At 25 or fewer tasks the gate does not fire and no entry is written. A `continue` verdict keeps tasks.json intact. A `split` verdict rewrites tasks.json to the side this ticket keeps and appends the other side, its boundary, and both dependency directions to `.map/<ID>/parked-ticket-candidates.json`; phase 21 must carry that parked candidate into exploration.md, and phase 22 alone may file it after GATE D. Phase 08 never writes roadmap.json. Continue phase 08 with the retained task set. It exists because ticket size is what drives replan cost: the four recorded runs opened 3, 3, 11, and 32 walk-backs against 5, 7, 20, and 107 tasks.
5. check the coverage: every struct and signature delta from phases 06-07 is covered by ≥1 task. Every side-effect that call-stacks.md records needs a task too. A side-effect with no task is work nobody owns. Commit `map(<ID>): phase 08 task-breakdown`.

## blame tags

`task-too-big` `coverage-gap` `files-list-wrong` `ticket-too-big`
