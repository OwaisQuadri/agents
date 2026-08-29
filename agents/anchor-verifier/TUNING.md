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

## 2026-08-29 — scheduled gepa-due Reflect: no new mutation, votes gap named

Due list flagged anchor-verifier at usage_count 65, vote_count 0 (threshold: 65 >= 15).
prompt_version at reflect time: `76a831fda` (current). `logs/usage.jsonl` holds 240 raw
lines; 65 carry this prompt_version exactly (34 predate it as `None`, 115 carry a stale
8-char truncation `76a831fd`, 3 carry a stale 10-char `76a831fda8` — all dropped as stale
per the loop's filter). `votes/votes.jsonl` does not exist: zero blind-judge votes on this
artifact at any prompt_version, despite the contract calling for one after logging. No
`evals/frontier.jsonl` exists either, so Propose has no frontier to sample from — mutation
would come from the incumbent only.

Of the 65 current-version lines: 42 success, 22 failure, 1 partial. Read every failure/
partial note (22) and categorized by pattern:

- **8 — dispatched verify_command itself is malformed** (unquoted jq selector under zsh,
  awk escaping, a `status` var collision, a command matching zero tests) and the run fails
  wholesale. This is the exact shape the first 2026-08-25 mutation targeted and the harness
  tied on (see above) — REVERTED as not-sufficient-cause. Current spec's own
  failure-mode watch-list explicitly mandates this exact behavior ("a broken command is a
  quoted-error FAIL plus a note — any other handling is suspect"), so these 8 are the
  artifact doing what it is written to do, not a new defect.
- **6 — conservative fail on a structurally unverifiable rubric item** (a historical
  no-network claim, a prior verifier's unanchored pass claim, a durable cross-run spend
  total, an exact digest with no reproducible input). This is the same shape as holdout
  case `c5` (`false-pass-unverifiable-claim`), already named in the 2026-08-25 entry as
  "pre-existing, in the artifact's own catastrophic list, and owed its own ticket." These 6
  production instances reinforce that the ticket is still open; they are not new evidence
  beyond what c5 already covers.
- **2 — invalid-dispatch on a missing required field** (verify_command, work_product_paths).
  Correct per the input contract; not a defect.
- **3 — the verifier caught a real problem in the work under test** (stale contract value,
  malformed JSONL, a dishonest response-only relabel). The artifact working as intended.
- **4 — ambiguous from the note alone**, not clearly any of the above; none repeat a
  distinct shape on their own.

DECISION: NO MUTATION THIS PASS. Every recurring shape in the 22 failures maps onto a
pattern this artifact's history already tested (and reverted as a tie) or already logged as
a known, ticketed gap (c5). Shipping a mutation now would either re-litigate a tied result
with no new evidence, or narrow/widen a behavior the current holdout case already exists to
catch — both against the loop's own narrowing rule ("needs an observed false positive,
never an imagined one") and against re-running a harness for zero new information.

NAMED FOR THE OWNER: the real gap this pass exposes is process, not prompt text — 65 real
uses and 0 blind-judge votes. The next high-leverage action on this artifact is running the
judge protocol (`skills/ai-author/SKILL.md` "judge protocol") against a sample of these 65
logged uses to get independent grades before the next Propose step, and building the c5
ticket (a `false-pass-unverifiable-claim` fixture/rule) as a fenced case + mutation pair —
not re-testing the already-tied verify_command-abort behavior again.
