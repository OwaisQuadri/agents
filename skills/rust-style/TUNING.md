# rust-style TUNING

## 2026-08-30: blocked/failed/pre-existing vocabulary + Blockers field

**Trigger**: `tools/gepa-due` flagged `rust-style` on usage_count (18, threshold 15) with
zero votes on file. Per the judge protocol, dispatched 5 fresh-context blind judges
(general-purpose, isolated, no shared context), one per each of the 5 most recent
`logs/usage.jsonl` lines, each given ONLY the artifact's own source and its one assigned
usage line. All 5 voted (`votes/votes.jsonl`), grading 4, 4, 4, 5, 4 out of 10.

**Failure histogram** (from the 5 votes, all independently converging):
- No vocabulary distinguishing a check that `failed` (ran, found a real problem in the
  changed scope) from one that's `blocked` (couldn't run — infra/environment failure) from
  one whose findings are `pre-existing` (crate-wide/out-of-scope debt the diff didn't
  introduce). 5/5 votes.
- No rule against silently substituting a narrower passing check for one that failed to
  compile/run, without naming the original failure. 2/5 votes (lines 09:42:26, 08:59:51).
- The `Exceptions:` report field conflates a deliberately-accepted lint exception with an
  unfixable pre-existing out-of-scope blocker. 2/5 votes (lines 07:04:46, 09:44:28).
- No escalation guidance for a recurring, structural blocker (same pre-existing crate-wide
  debt hit across 5 straight runs). 1/5 votes (line 09:44:28).
- No environment-durability/retry guidance when the working copy vanishes mid-review.
  1/5 votes (line 09:33:24) — deferred, see below.

**Mutation** (candidate `4f95ec0a`, mutated from the incumbent `e4a2126b` — no frontier
had ≥2 non-incumbent members yet, so mutated from the incumbent per the GEPA loop's
below-threshold rule):
- Step 2: a baseline rule applies to the changed lines/targets; an out-of-scope finding is
  pre-existing, not a blocker to fix now — name it under `Blockers:` instead.
- Step 4: a check is one of `failed` / `blocked` / `pre-existing`, each defined; a narrower
  passing check must never silently replace a failed one without naming the original
  failure; a repeat of the same pre-existing blocker must be named as recurring.
- Report template: split `Exceptions:` (deliberate, accepted lint exceptions) from a new
  `Blockers:` field (anything from step 4's failed/blocked/pre-existing list).

**Eval cases added** (dispatched a separate fresh-context case-author, blind to the
candidate text, to keep the fence between mutation-proposer and exam-writer honest):
`c7` (silent substitution of a narrower passing test run for a `cargo test` compile
failure) and `c8` (pre-existing crate-wide clippy debt vs. a deliberately-accepted
exception), both `source: vote`, both `holdout: false`. Existing holdout case `c6`
untouched.

**Test**: `evals/run.sh` re-run on both slices, both candidate texts, after the new cases
landed:
- Incumbent (`e4a2126b`): mean 7.86/10 nonholdout (c7 and c8 scored 3 each — gap
  confirmed), 9.00/10 holdout.
- Candidate (`4f95ec0a`): mean 9.57/10 nonholdout (c7 and c8 scored 9 each), 9.00/10
  holdout (holds, no regression). No catastrophic (0) score on either.

**Decide**: accepted. Higher mean, no new catastrophic failure, holdout holds. Frontier
line for `4f95ec0a` flipped to `accepted:true` in `evals/frontier.jsonl` in place (no
re-grade). Shipped to `skills/rust-style/SKILL.md`.

**Also shipped this pass**: `evals/run.sh` was out of contract with
`skills/ai-author/templates/eval-harness.md` — it graded only one slice per invocation and
never wrote `evals/frontier.jsonl`, so there was nowhere for a frontier candidate to land.
Rewrote it to grade both slices in the plain (no `--holdout`) form and append a
`frontier.jsonl` line + archive the candidate's full text to `evals/frontier/<id>.md`,
per the template. This is why the incumbent (`e4a2126b`) has two frontier lines: the
harness didn't exist in its current form before this pass, so its first line was
backfilled by literally running it once.

## Deferred (not this pass)

- **Environment durability / retry when the working copy vanishes mid-review** (1/5 votes,
  line 09:33:24). Narrower signal than the other four (only 1 vote), and the fix shape is
  different in kind — it's about dispatch/retry mechanics, not report vocabulary. Left for
  a later pass once more usage evidence accumulates on this specific failure mode.
- **Escalation path for a recurring blocker** (1/5 votes) — the `Blockers:` field now lets
  a reviewer name a blocker as recurring, but there's no rule yet for what happens after
  N repeats (e.g., file a followup ticket). Deferred: this repo's style favors leaving
  ticket-filing calls to the caller, and no usage line yet shows a case where naming it as
  recurring wasn't enough.
