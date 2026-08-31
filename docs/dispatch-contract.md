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

## logging hygiene (retired)

No agent writes a usage log anymore. `logs/usage.jsonl` and the self-report line this
section used to describe building safely (`jq -cn --arg`, backtick/quote/`$(...)`
escaping) are gone — usage evidence is now derived from real Pi session transcripts by
`tools/gepa-due`, never self-reported. `prompt_version` computation is unaffected and
still lives in `skills/ai-author/SKILL.md`'s "usage evidence" section — referenced from
there, not restated here.

Historical note, kept for context: the fleet lost 19 self-reported log lines to shell
interpolation and unexpanded `$(date ...)` before this section existed, and a 2026-08-24
sweep found 88 `engineer` lines with no reliable way to tell which predated the rules
being judged. Both failure classes are structurally impossible now — there is no write
path left for either to corrupt.
