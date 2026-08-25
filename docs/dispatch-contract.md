# dispatch contract

Shared clauses for every agent a parent dispatches into a repository. An agent
definition references this file. It never restates a clause.

## the baseline stamp

A dispatched agent arrives in a repository that is already dirty. That is the normal
state, not an error: the worker before it left untracked work products, a sibling agent
holds unrelated edits, the user has his own uncommitted work, and a branch tip can move
under a running review.

Four fleet definitions once graded themselves against a clean tree, and the logs recorded
55 runs where that assumption misfired — 21 anchor-verifier runs that passed every
dispatched rubric item and still returned `fail` on a non-empty `git status`, 16 debugger
runs whose untouched-tree line was unverifiable on arrival, 18 code-reviewer runs split
between dirty arrivals and mid-run ref movement, and one maestro-tester run where a delta
the agent did not cause was attributed to it and the resulting vote outlived its own
retraction.

So:

- THE DISPATCHER STAMPS THE BASELINE into the brief with `dispatch-baseline stamp --repo
  <path> --out <file>`, captured before the agent starts. Write the stamp OUTSIDE the
  repository, or it becomes a delta against itself. The tool owns what a baseline holds;
  never restate its fields here.
- A BRIEF WITH NO STAMP is not invalid. The agent runs the same command as its own first
  act and prints the result, and that capture becomes the baseline.
- THE AGENT PROVES ITS OWN DELTA with `dispatch-baseline check --repo <path> --stamp
  <file>`, which exits 0 on an empty delta and 1 on a real one. That exit code is the
  anchor, and no prose claim of "I changed nothing" substitutes for it.
- THE AGENT REPORTS AGAINST THE STAMP. Only a delta from the baseline belongs to the
  agent. A path already dirty in the baseline is reported in notes and never graded as
  the agent's own write.
- A BASELINE THAT MOVES MID-RUN is a finding, never a silent regrade. Name the old and
  new hash, say which part of the work the movement invalidates, and let the dispatcher
  decide.

The clean-tree rule stays exactly where it belongs: an agent whose role forbids writing
still writes nothing, and its own delta must be empty. What changed is the measurement,
from the state of the tree to the change in it.

## outcome semantics

`outcome` grades THIS RUN'S EXECUTION OF THE ROLE. It never grades the deliverable, and
it never grades the code under test.

- `success` — the role executed correctly. A correct refusal is a success. So is
  `invalid-dispatch`, a `blocked` verdict naming its precondition, a reproduction that
  did not reproduce, and a verdict of `fail` that the evidence supports.
- `failure` — the role misfired. It improvised past a missing input, graded on a
  self-report, edited a file its contract bars, or claimed a pass with no anchor.
- `partial` — the run was cut short before the role finished.

A run that grades itself `success` while holding a known coverage hole is a `partial`,
and the hole is named. Eight blind-judge votes across the fleet were spent on this one
distinction.

## logging hygiene

Build the `logs/usage.jsonl` line with `jq -cn --arg`, never with a shell string that
interpolates the excerpt. jq escapes the value, so a backtick, a quote, a newline, or a
`$(...)` inside the excerpt survives intact and the line still parses.

Every line carries `prompt_version`: the short commit of the last change to the files the
artifact LOADS.

```sh
git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md' ':(exclude)**/logs/**' ':(exclude)**/votes/**'
```

The exclusions matter both ways. `evals/` is excluded because a new test case changes no
behaviour, and a version that moved on test edits would discard log lines that are still
valid. `TUNING.md` is excluded because it is written after a change, only sometimes, and it
never loads into the body — commit ec7ca48 changed `skills/byline/SKILL.md` and wrote no
byline TUNING entry, so a TUNING-watching stamp would have called that prompt unchanged.

A Reflect pass drops every line whose `prompt_version` no longer describes the artifact it
is about to tune. Without it a stale line and a fresh one look identical, which is the hole
the 2026-08-24 sweep sat in: 88 engineer lines with no way to tell which predated the rules
being judged.

The fleet lost 19 log lines to this: 16 anchor-verifier lines truncated or blanked by
backtick interpolation, and 3 debugger lines, two of which carry a literal unexpanded
`$(date +%Y-%m-%dT%H:%M:%S%z)` where the timestamp belongs. These logs are the reflective
dataset the GEPA(Genetic-Pareto prompt evolution) loop reads. A corrupted line is a use
that never happened.
