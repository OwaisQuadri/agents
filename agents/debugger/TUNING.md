# debugger — tuning history

## 2026-08-29 — no mutation: evidence too thin under the live prompt

GEPA-due trigger fired on usage_count=28, vote_count=0. Reflect pass, filtered to the
current `prompt_version` (76a831fda, computed via this artifact's own logging-section
command): 8 lines carry no `prompt_version`, 19 are `82f5edc`, 4 are `82f5edc0`, 25 are
`76a831fd` (truncated, 8 chars), 3 lines are malformed JSON — all stale under the Reflect
filter's exact-match rule, all dropped. Surviving evidence for the live prompt is 3 usage
lines (2026-08-28), outcomes success / success / partial. Zero failures.

`agents/debugger/votes/votes.jsonl` does not exist — the whole `votes/` directory is
absent, so there is zero blind-judge signal for this artifact, live prompt or otherwise.
No `evals/frontier.jsonl` exists either.

The one `partial` line (AGNT-0032.T153) is the debugger correctly halting at a named
out-of-scope file (`service.rs`) after fixing the dispatched timeout — not a defect in
this artifact's behavior, and not a repeating shape (n=1).

DECISION: no mutation. Three surviving lines with zero failures gives no failure
histogram to target and no logged false positive to narrow against. The GEPA loop's own
Decide gate rejects churn on noise (skills/ai-author/SKILL.md:200); proposing a mutation
here would be exactly that. This artifact needs more logged runs under the current prompt
(76a831fda) and its first blind-judge vote before the next Reflect pass has enough signal
to propose from. Deferred: nothing actionable — next GEPA-due pass should re-run Reflect
fresh rather than resume from here.

## 2026-08-25 — baseline delta on arrival, and `fixed-tests-stale`

Two mutations. ONE: "working tree untouched" is graded as a zero DELTA from the baseline
stamp (docs/dispatch-contract.md), never as a clean tree, so pre-existing dirt and a sibling
agent's concurrent edits are reported in notes and left alone. TWO: a new status
`fixed-tests-stale` with a `stale_tests:` field, for a fix that is correct and proven and
orphans a test asserting the old behaviour. Still zero test edits.

Evidence: 16 logged runs arriving in a repository already dirty, where the untouched-tree
line was unverifiable. 12 more in the orphaned-test shape, which on 2026-08-24 produced four
consecutive `failure` lines in 70 seconds as the parent re-asked and the debugger correctly
re-refused.

HARNESS RESULT: A TIE, and a tie is not a win.

    incumbent (pre-change)  c6 9  c7 9   mean 8.33 over 6 cases, 0 catastrophic
    candidate (mutated)     c6 9  c7 9   mean 8.33 over 6 cases, 0 catastrophic

Identical, case for case, on fixtures built by an author blind to these mutations. The
unchanged definition already handles both described situations correctly in a clean-room
fixture. `skills/ai-author/SKILL.md:200` says ties go to the incumbent, and `:210` says
reporting a tie as a harness win is the failure that clause exists to stop.

PATH USED: the DEFECT-FIX path, which the acceptance rule allows for a change no existing
case measures, on a reproduction plus execution evidence rather than on a mean. The 28
logged occurrences are the reproduction. The owner accepted this framing on 2026-08-25.

THE GAP, stated so nobody has to rediscover it: the case reproduces the DESCRIPTION of the
failure, not the conditions that produced it. Every one of the 28 originals happened in a
live dispatch against a live repository. In a clean fixture the incumbent behaves well, so
the exam cannot see the difference. A discriminating case has to make the failure available,
not merely describe the setting it happened in. That limit applies to this whole tuning
pass, not only to these two mutations.
