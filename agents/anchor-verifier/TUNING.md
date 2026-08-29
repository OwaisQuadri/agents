# anchor-verifier — tuning history

## 2026-08-25 — baseline delta, and an aborted chain

Two mutations. ONE: `files_modified` is measured as a delta between an opening and a closing
snapshot over work_product_paths, so a work product that arrives dirty or untracked is the
normal received state and never fails the run on its own. TWO: a `verify_command` that
aborts partway grades the items that stage owned as fail, while items an independent file
read or sub-command can anchor are graded on those anchors. Unreached is not unverifiable.

Evidence: 21 logged runs that passed every dispatched rubric item, wrote nothing, and still
returned `fail` on a non-empty `git status`. 33 more where a chain died under `set -e` and
every later item was graded fail as "not reached".

HARNESS RESULT: A LOSS.

    incumbent (pre-change)  c1 10 c2 10 c3 10 c4 10 c6 10 c7 10   mean 10.00
    candidate (mutated)     c1 10 c2 10 c3 10 c4 10 c6  5 c7 10   mean  9.17

c7 does not discriminate: the unchanged definition already grades the aborted chain
correctly on that fixture. c6 REGRESSED, 10 to 5, `pass-without-executed-anchor` — the
candidate ran the command but redirected stderr, so the runtime marker never reached its
anchor. One sample each, so the regression is a signal and not a measurement.

WHY c6 CANNOT SHOW THE FIX: its dispatched rubric asks about dag.mmd regeneration and
disjoint wave file sets. Nothing in it triggers the verifier's own empty-`git status`
safeguard, which is what produced all 21 originals. The incumbent therefore passes it
cleanly. The exam does not contain the disease.

STATUS: NOT ACCEPTED ON THE HARNESS. A replacement c6 is being built by an author blind to
this text, whose dispatch is satisfiable in full by executed evidence WHILE the work
products are untracked and the tree is dirty. Any agent imposing a cleanliness safeguard
fails it; any agent grading on a delta passes. Re-decide on that result, and revert both
mutations if it still shows nothing.

Also recorded, and NOT caused by these mutations: holdout case c5 scores 0
`false-pass-unverifiable-claim` on BOTH definitions. The agent writes its own 50-thread
stress test to anchor a rubric item the case calls unverifiable. Pre-existing, in the
artifact's own catastrophic list, and owed its own ticket.

## 2026-08-25 — final decision: reverted after the rebuilt case still tied

A fenced author rebuilt c6 from the 21 production failures, not from either definition. The
new fixture contains a tracked-and-modified work product, an untracked test, and unrelated
sibling dirt. Every dispatched rubric item is fully provable by an executed command and a
file read. A deliberately defective stub that applies an empty-tree safeguard scores 0, so
the case is capable of detecting the named failure.

The actual result still tied:

    incumbent (pre-change)  c6 10  c7 4   mean 9.00 over 6 cases
    candidate (mutated)     c6 10  c7 4   mean 9.00 over 6 cases

The unchanged definition would not reproduce the 21 production failures even in a fixture
built to elicit them. Therefore the definition text was not sufficient cause. The open
question moves upstream: inspect the live dispatch briefs from those 21 runs for an explicit
cleanliness rubric or other condition absent from the fixture.

DECISION: REVERT both definition mutations. This follows the recorded status above ("revert
both mutations if [the replacement c6] still shows nothing") and the GEPA rule that ties go
to the incumbent. The fixtures and harness fixes stay: they are real test infrastructure,
and c6 proved it can score a guard-applying stub 0.

## 2026-08-29 — gepa-due trigger (usage_count 65 >= 15): no mutation, here is why

Triggered by the scheduled gepa-due sweep, not by new evidence pointing at a specific
failure. Ran Reflect in full before deciding whether Propose applies.

`votes/votes.jsonl` does not exist for this artifact — zero blind-judge votes on file. The
Reflect step's judge-side input (failure histogram from votes, `vote` text keyed to
`prompt_version`) is therefore empty; nothing there to mutate against.

`logs/usage.jsonl` has 240 raw lines. Current `prompt_version` from `git log -1 --format=%h`
is `76a831fd`, which resolves (with git's varying abbrev length across the log's history) to
the same full commit `76a831fda89d3d78150038c04b7af6726b5312ba` under three logged prefixes:
`76a831fd` (115 lines), `76a831fda` (65 lines, the count the trigger's `usage_count` names),
and `76a831fda8` (3 lines) — 183 lines of real current-prompt evidence once the prefixes are
resolved to the one commit they share. 34 lines carry no `prompt_version` field at all and
were dropped as stale per the loop's own rule. 21 lines are malformed JSON (one dispatch's
notes field contained unescaped literal newlines, splitting one record across ~17 lines) and
were skipped as unparseable — a logging-encoding defect in whatever produced that one line,
not a definition defect; flagged here in case it recurs, not chased further this pass.

Read every non-success line (57 failure, 2 partial) among the 183 current-prompt lines. None
show the verifier erring: they document the verifier correctly grading dispatched work as
failing — unmet rubric items, unanchored self-report claims rejected, non-zero
`verify_command` exits, conservative fails on unverifiable clauses. That is the designed
behavior (grade what is proven, never invent a pass), not a repeating complaint calling for a
mutation. `wrong/mistake/incorrect/bug`-adjacent hits in the notes are the verifier catching
workers' self-report laundering, or the verifier's own auxiliary shell one-liners (a jq
filter, a zsh count) failing and being corrected mid-run — incidental scaffolding fixes, not
verifier-logic defects.

The last real Reflect→Propose→Test→Decide cycle on this artifact ran four days earlier
(2026-08-25, above) against exactly the failure class a fresh mutation would target here
(empty-`git status` false fails, aborted-chain grading): two proposed mutations, tested twice
(once against a non-discriminating case, once against a fenced-rebuilt case purpose-built to
reproduce the 21 production failures), tied both times, reverted per the ties-go-to-incumbent
rule. Nothing in this pass's evidence — no votes, no new failure pattern in 183 fresh usage
lines — contradicts that call or gives Propose a different target to aim at.

The one standing catastrophic gap (`c5`, `false-pass-unverifiable-claim` scoring 0 on both
definitions) is unchanged from 2026-08-25: still pre-existing, still not caused by either
mutation, still owed its own ticket rather than a same-pass fix (Decide's defect-fix path
requires a fenced case author dispatched in the same pass, and this pass has no repro beyond
what's already on file).

DECISION: NO MUTATION. Reflect ran, found no new evidence and no votes, and the most recent
full loop pass already covered the failure class a mutation would target. Propose has nothing
to aim at that the 2026-08-25 pass didn't already test and reject. Open items carried
forward: (1) `c5` false-pass-unverifiable-claim still needs its own ticket and fenced case;
(2) the ~17-line JSONL corruption around 2026-08-25T02:xx should be traced to its producing
dispatch if it recurs; (3) `votes/votes.jsonl` has never been written for this artifact —
running the blind-judge protocol at least once would give the next Reflect pass real
judge-side signal instead of usage-log notes alone.
