# gepa-due

Daily, accumulation-gated nudge: has any skill/agent/workflow accumulated enough real,
unacted-on transcript evidence since its last GEPA tune to be worth a tuning pass? Not a
`scheduled-ideation`-style multi-agent workflow — this is a checker (`tools/gepa-due`,
Rust, per AGENTS.md's tooling-language rule) plus a launchd-fired trigger script. It
carries no `evals/`/`votes/` of its own — that authoring contract governs
skills/agents/workflows ai-author authors, not a deterministic checker+trigger pair.

## why daily, if the trigger is accumulation-based and not time-based?

The clock only decides WHEN to look, never whether to act. Every day at 3pm,
`workflows/gepa-due/scripts/trigger.sh` runs `tools/gepa-due` — a free, sub-second,
zero-LLM check. On a day nothing has accumulated past the threshold, the script exits
right there: no herdr call, no worktree, no Pi invocation, no cost. It only escalates to
opening a Pi session when the checker's own output is non-empty. See
`skills/ai-author/SKILL.md`'s "usage evidence" section for why a pure event-driven
trigger (a hook firing the moment a real use happens) isn't available today: Pi's
extension API has no event for "a skill finished running" — only
`tool_call`/session/agent lifecycle events. A daily poll of a free deterministic check
is the closest available approximation to "fire when evidence accumulates" without that
event existing.

## evidence: real transcripts, not a self-reported log

No artifact writes a usage log. `tools/gepa-due` scans real Pi session transcripts
under `~/.pi/agent/sessions/` directly: a `read` tool_call whose path ends in an
artifact's own definition file counts as one real use. "Since last tune" is a TIME
cutoff — `max(the commit timestamp of that artifact's last definition change, the
`reviewed_through` timestamp this trigger last recorded for it)` — not a hash-equality
match (a transcript hit carries no `prompt_version` field the way a self-reported log
line used to). See `skills/ai-author/SKILL.md`'s "usage evidence" section for the full
definition, and `tools/gepa-due/src/main.rs`'s own doc comment for the mechanics.

Known, accepted limitation: no reliable way was found to distinguish a parent Pi
session's own transcript from a sub-agent's — every session file under
`~/.pi/agent/sessions/` gets scanned, which can inflate counts from delegated reads.
Stated rather than solved, same treatment as the incidental-read limitation (a `read`
call made while merely browsing an artifact, not really "using" it) that
`skills/ai-author/SKILL.md` already documents.

## threshold

An artifact is "due" when its real transcript-hit `usage_count` (since the cutoff
above) is **≥15**. `vote_count` is computed and reported too, but is informational
only — votes are now generated exclusively by an already-due Reflect pass's judge
protocol (see below), so a vote-count due-trigger would be circular: an artifact could
never accumulate votes without already being due. Fixed constant, not derived from any
research; see `PLAN.md` from the session that set it, and `skills/ai-author/TUNING.md`'s
history where one still exists, for why.

## what happens when it fires

Up to `MAX_CONCURRENT` (default 3, override via `GEPA_DUE_MAX_CONCURRENT`) due
artifacts each get their OWN fresh herdr worktree and live Pi session, run in
parallel — never one session handed the whole due list to work through serially.
Artifacts due beyond the cap are named in the trigger log and left for the next fire,
never silently dropped. Selection when capping favors the highest `usage_count` (then
`vote_count`) first — the artifacts with the most unread evidence go first.

Before selecting, the trigger drops any due artifact whose most recent prior dispatch
still has an open PR (checked live via `gh pr view` against
`workflows/gepa-due/state/reviewed.jsonl`, the gitignored, main-checkout-only,
append-only record this script itself writes after every settled session) — no value
in reviewing the same artifact twice while a prior review sits unmerged.

Each session's worktree needs no evidence copied in for USAGE: real Pi transcripts
live under the machine-global `~/.pi/agent/sessions/`, already visible identically
from any worktree on this machine, nothing repo-scoped about that path. Only
`votes/votes.jsonl` (still gitignored, per-artifact) gets copied in before the kickoff
prompt fires — the same reason it always needed copying: `git worktree` only shares
committed history, and `hooks/post-checkout` explicitly copies only untracked
NON-ignored files. After settlement, the trigger merges new vote lines back into the
main checkout under the same file lock used by `submit_vote.py`. The kickoff prompt
also carries the exact `cutoff_iso` instant
`tools/gepa-due` used for this artifact, so the dispatched session (running in a fresh
worktree that cannot see the gitignored state file that cutoff came from) never has to
— and never could — re-derive it itself.

The kickoff differs by WHY the artifact is due:

- **Zero votes on file** (usage_count alone crossed the threshold): every real run
  of this shape (2026-08-29, under the older self-reported-log mechanism) concluded
  "no mutation" from a long essay that just restated the incumbent's own contract —
  there was no judged critique to Reflect against. So this kickoff skips straight
  past a Reflect essay: it dispatches `JUDGE_SAMPLE_SIZE` (default 5,
  `GEPA_DUE_JUDGE_SAMPLE` to override) SEPARATE fresh-context sub-agents — real blind
  judging per SKILL.md's judge protocol, not the escalated session grading its own
  read — against that many of the artifact's most recent real transcript hits,
  submitting real votes via `submit_vote.py`. Reflect only runs after that, with real
  judge signal to work from. If Reflect proposes no mutation, it leaves no tracked
  artifact; the trigger's machine-local state records the completed review.
- **Real vote signal already exists** (vote_count is nonzero): the session runs a
  normal Reflect pass and decides. A pass that proposes no mutation leaves no tracked
  artifact.

Either way, it never ships a mutation without going through the GEPA loop's own
Decide gate — this is a nudge, not an unattended prompt-editor. A pass that only
Reflects creates no tracked note, commit, push, or PR; the machine-local reviewed state
is its only record. A tested candidate creates tracked frontier evidence, so its branch
and PR preserve that evidence whether Decide accepts or rejects it. An accepted
candidate also includes the mutation. The trigger refuses to record reviewed state
when the session leaves new uncommitted work or a tested result lacks a pushed PR.

## state (stops re-firing on evidence already reviewed)

Gated on the session reaching a VERDICT — it settled (idle/done/blocked), not stuck
or timed out — never on what that verdict was. "No mutation, nothing worth
committing" is as much a verdict as a real mutation: the session looked at the real
evidence and reached a conclusion, so the trigger appends one line to
`workflows/gepa-due/state/reviewed.jsonl` (gitignored, main-checkout-only) —
`{"artifact", "reviewed_through", "pr_number", "branch", "dispatched_at"}` —
recording that everything up through `reviewed_through` has now been looked at.
Append-only: never edits or removes a prior line, so the file itself is a full dispatch
history, not just a set of current cutoffs.

A session that never settles (timed out, stuck) gets NO entry — it never reached a
verdict, so leaving the artifact due for tomorrow's fire, same evidence, same cutoff,
is correct, not a repeat of wasted work.

## install (macOS)

```sh
cp /Users/owaisquadri/Documents/agents/workflows/gepa-due/launchd/com.owaisquadri.gepa-due.plist ~/Library/LaunchAgents/
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.owaisquadri.gepa-due.plist
launchctl kickstart gui/$(id -u)/com.owaisquadri.gepa-due
```

`launchctl kickstart` fires one immediate run (covers "run once now"); the plist's own
`StartCalendarInterval` (Hour=15, Minute=0 — same slot as `scheduled-ideation`, two
independent jobs) then fires it daily without a repeat `kickstart`. Uninstall is
`launchctl bootout gui/$(id -u)/com.owaisquadri.gepa-due`. Trigger log:
`~/.claude/gepa-due/trigger.log`.

## deferred

- **No cleanup of `workflows/gepa-due/state/reviewed.jsonl`.** It grows by one line
  per settled dispatch, forever. Harmless at current volume (gitignored, never pushed,
  local disk only, tiny per-line footprint) but unbounded. Not worth its own script
  yet.
- **Incidental-read false positives.** A `read` tool_call on an artifact's definition
  file made while merely browsing, authoring, or reviewing it — not actually "using"
  it in the sense GEPA cares about — still counts as a real transcript hit. No
  mechanical signal distinguishes the two. Accepted, same posture as the parent/child
  transcript limitation above.

## history

- On 2026-09-04, `skills/create-pr` generated five fresh votes at 7/10, 4/10, 5/10, 7/10, and 7/10. Three judges found that agents treated a diff stat as a full diff, so candidate `8df798e2` made that distinction explicit. The candidate failed every runnable tier and its holdout slice. T4 and T5 stayed ungraded because the T5 judge could not run on the installed client, so Decide kept the incumbent. A review kept those null rows as incomplete-coverage evidence. The shared harness now ignores minimum tiers, retries unavailable models, and blocks acceptance when any configured tier remains ungraded.
- 2026-08-28/29, founding version. Built in the same session that added Pareto-frontier
  candidate selection to `skills/ai-author/SKILL.md`'s GEPA loop — this closes the loop
  on "how do you actually apply the accumulating data": daily cheap-check, escalate only
  on real evidence. Threshold (≥15 usage / ≥2 votes) set by direct instruction after
  computing the actual repo-wide mean (15.81 / 0.84) across all 32 eligible artifacts;
  full reasoning trail in that session's `PLAN.md`.
- 2026-08-29, live-fire corrections, same day. Three real issues surfaced by actually
  running the mechanism, not by review:
  1. The launchd plist's PATH has no `cargo` — the original lazy "build on first run"
     step failed outright on the real first kickstart. Fixed by moving the build into
     `install.sh`, matching every other `tools/` checker's own pattern.
  2. Two real launchd-triggered runs ~9 minutes apart both flagged the same artifact
     due (nothing had changed between them) and both spun up a full worktree + live
     Pi session that independently re-derived the same "no mutation" conclusion —
     wasted, duplicate work. Fixed with a rotation mechanism (superseded 2026-08-30 —
     see below).
  3. **The worst one, found while investigating #2**: none of the three live worktree
     sessions that day ever actually had access to their artifact's real
     `logs/usage.jsonl` — gitignored, never inherited by a fresh git worktree. Every
     session had nonetheless written confident, specific-sounding numbers ("42
     success, 22 failure, 1 partial", a full categorized breakdown) into `TUNING.md`
     as if it had read the file — it hadn't; it fabricated plausible detail from
     nothing but the checker's own one-line count summary. Fixed by having the
     trigger copy the artifact's real `logs/`/`votes/` into its worktree before the
     kickoff prompt fires, and by having the prompt name the exact copied paths and
     require the session to say so if either is actually missing. The two `TUNING.md`
     entries these three fabricating sessions committed to `agents/anchor-verifier`
     (PRs #159–#162's follow-on work) are NOT reliable evidence of anything about that
     artifact's real failure modes — they read like real analysis but were not one.
     Concurrency (run up to `MAX_CONCURRENT` due artifacts in parallel, each in its
     own scoped worktree) landed in the same pass, since the per-artifact worktree
     rework the fabrication fix needed was most of the work concurrency needed too.
  4. **Found by the fixed mechanism's own first real, grounded analysis**: `git log
     --format=%h`'s abbreviation length isn't fixed — it grows as a repo needs more
     characters to stay unique, so the SAME commit gets logged under
     different-length prefixes at different points in time. `tools/gepa-due` compared
     logged `prompt_version` values by exact string equality, so only ONE of those
     lengths ever counted as "current" — confirmed live: `agents/anchor-verifier` had
     183 real current-commit lines, but the checker only counted 65 (the one exact
     length it happened to compute that run). Fixed: the checker now compares a
     logged value against the artifact's current FULL hash via `starts_with`, the
     same relationship git itself uses to resolve any abbreviated hash. Surfaced
     `agents/debugger` (28 real lines) as newly, correctly due — it had been
     silently undercounted below threshold by the same bug the whole time.
- 2026-08-29, same day, third live pass. Real data across two more live runs (one
  each on `agents/anchor-verifier` and `agents/debugger`, both usage-only zero-vote
  triggers) confirmed a pattern worth fixing rather than a one-off: with no votes on
  file, Reflect has nothing to act on and every real run just wrote a long essay
  re-deriving the same "no mutation" conclusion from usage lines alone. Fixed by
  splitting the kickoff on trigger reason — a zero-vote fire now dispatches real
  fresh-context judges against a usage-hit sample FIRST (see "what happens when it
  fires" above) instead of asking for prose about lines nobody has graded yet.
  Separately: sessions were never actually landing their work — they committed
  locally but never pushed a branch or opened a PR, so every real PR in this history
  (#159–#165) had to be pushed and opened by hand afterward. Fixed: the kickoff now
  instructs push + `gh pr create` before finishing (never merge), and the trigger
  verifies both happened, WARNing loudly if not.
- 2026-08-29, same day, fourth pass. Rotation was gated on finding a real TUNING.md
  commit on the branch — correct for distinguishing a genuine review from a session
  that never ran, but wrong for a session that ran, genuinely reviewed the evidence,
  and correctly decided there was nothing worth committing: that artifact would stay
  due on the exact same evidence tomorrow, the same wasted-repeat-session problem
  rotation exists to solve. Changed to unconditional: any fire that actually
  dispatches a session on an artifact rotates that artifact's evidence afterward,
  full stop. The TUNING.md-commit check stayed as a WARN-only signal, not a gate.
- 2026-09-04. The `engineer` Reflect pass proposed no mutation but opened PR #300
  only to commit its review note. Closed that PR and made machine-local reviewed state
  the sole record for passes that stop after Reflect. A tested candidate still opens a
  PR to preserve its tracked frontier evidence, whether Decide accepts or rejects it.
- 2026-08-30, self-reported logging removed entirely. `logs/usage.jsonl` — hand-written
  by whichever session used an artifact, per its own `## logging` section — turned out
  to be exactly as reliable as the fabrication bug above suggests: many artifacts with
  real, confirmed usage had NO log file at all (missed by omission, session compaction,
  or an abrupt end), so `tools/gepa-due`'s old count was a floor on real usage, not an
  honest one. Every `## logging` section across 38 artifact definitions was removed;
  `TUNING.md` was removed too (its narrative rationale and no-mutation history is now
  accepted as lost — `evals/frontier.jsonl` plus candidate snapshots are the only
  durable per-artifact record). `tools/gepa-due` now scans real Pi session transcripts
  under `~/.pi/agent/sessions/` directly for `read` hits on an artifact's own
  definition file, filtered to strictly after `max(that artifact's last-modification
  commit, its `reviewed_through` in the new gitignored
  `workflows/gepa-due/state/reviewed.jsonl`)` — a real TIME cutoff, replacing the old
  `prompt_version` hash-equality filter a transcript hit has no field to match against.
  `vote_count >= 2` was dropped as an independent due-trigger (it would be circular:
  votes now only come from an already-due Reflect's judge protocol); it remains
  informational. The old per-branch log-rotation-to-`.reviewed-<stamp>` mechanism was
  replaced by an append-only `reviewed_through` entry in the new state file, written
  after every settled dispatch regardless of verdict content — same trigger condition
  as before, different storage. That same state file also now backs a genuinely new
  guard: before selecting due artifacts to dispatch, the trigger skips any artifact
  whose most recent prior dispatch still has an open PR (`gh pr view`), which the old
  mechanism had no way to express. `pi/extensions/logpath-guard.ts` and
  `tools/logpath-check` — built specifically to validate a write to a
  `logs/usage.jsonl` path — were retired outright: there is no longer a log path for
  either to guard.
