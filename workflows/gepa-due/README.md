# gepa-due

Daily, accumulation-gated nudge: has any skill/agent/workflow accumulated enough real,
unacted-on usage/vote evidence since its last GEPA tune to be worth a tuning pass? Not a
`scheduled-ideation`-style multi-agent workflow — this is a checker (`tools/gepa-due`,
Rust, per AGENTS.md's tooling-language rule) plus a launchd-fired trigger script, the
same shape `tools/logpath-check` + `pi/extensions/logpath-guard.ts` already use for a
different check. It carries no `evals/`/`TUNING.md`/`logs/`/`votes/` of its own — that
authoring contract governs skills/agents/workflows ai-author authors, not a deterministic
checker+trigger pair.

## why daily, if the trigger is accumulation-based and not time-based?

The clock only decides WHEN to look, never whether to act. Every day at 3pm,
`workflows/gepa-due/scripts/trigger.sh` runs `tools/gepa-due` — a free, sub-second,
zero-LLM check. On a day nothing has accumulated past the threshold, the script exits
right there: no herdr call, no worktree, no Pi invocation, no cost. It only escalates to
opening a Pi session when the checker's own output is non-empty. See
`skills/ai-author/TUNING.md`'s note on why a pure event-driven trigger (a hook firing on
`logs/usage.jsonl` growing) isn't available today: Pi's extension API has no event for
"a file changed on disk" or "a skill finished running" — only `tool_call`/session/agent
lifecycle events. A daily poll of a free deterministic check is the closest available
approximation to "fire when evidence accumulates" without that event existing.

## threshold

An artifact is "due" when, filtered to its CURRENT `prompt_version` (same filter GEPA
loop step 1/Reflect already applies by hand): `logs/usage.jsonl` has ≥15 surviving
lines, OR `votes/votes.jsonl` has ≥2 surviving lines. Fixed constants, not derived from
any research (see `PLAN.md` from the session that built this, and
`skills/ai-author/TUNING.md`, for why — no source discussed a trigger cadence for a
single-user, low-volume regime).

## what happens when it fires

Up to `MAX_CONCURRENT` (default 3, override via `GEPA_DUE_MAX_CONCURRENT`) due
artifacts each get their OWN fresh herdr worktree and live Pi session, run in
parallel — never one session handed the whole due list to work through serially.
Artifacts due beyond the cap are named in the trigger log and left for the next fire,
never silently dropped. Selection when capping favors the highest `usage_count` (then
`vote_count`) first — the artifacts with the most unread evidence go first.

Each session's worktree gets that ONE artifact's real `logs/usage.jsonl` and
`votes/votes.jsonl` copied in before the kickoff prompt fires (worktree creation uses
the same mechanism `workflows/scheduled-ideation/scripts/trigger.sh` already proved
live — no GUI Terminal, no TCC(Transparency, Consent, and Control) prompt, every call
over the herdr daemon's socket API). That copy step matters: `logs/`/`votes/` are
gitignored, so a fresh git worktree never inherits them on its own — git worktrees
only share committed history, and `hooks/post-checkout` explicitly copies only
untracked NON-ignored files. Without the copy, the session has no real evidence to
read and nothing stops it from confabulating plausible-sounding numbers instead — a
real failure mode this mechanism hit and had to fix (see deferred list below).

The seeded session decides what to do — run a real GEPA tuning pass, or just record a
short "no mutation, here is why" note — but it never ships a mutation without going
through the GEPA loop's own Decide gate. This is a nudge, not an unattended
prompt-editor. Whatever it concludes, it's told to commit a dated `TUNING.md` entry
before finishing; the trigger checks for exactly that commit afterward to decide
whether to rotate the reviewed evidence (see "rotation" below).

## rotation (stops re-firing on evidence already reviewed)

Once a session's branch has a real commit touching `<artifact>/TUNING.md`, the trigger
treats that as proof the evidence was actually read and reasoned about — mutation
shipped or not — and moves that artifact's `logs/usage.jsonl` and `votes/votes.jsonl`
in the MAIN checkout to dated `.reviewed-<stamp>` siblings, then verifies the move
(destination exists, source gone) before logging it. A move, never a `rm`, per this
repo's own "never rm before a verified move" rule — the reviewed evidence stays on
disk, just out of `tools/gepa-due`'s exact-filename count (it only ever reads
`logs/usage.jsonl` / `votes/votes.jsonl` literally, so a renamed sibling is
automatically excluded with no checker code change needed). No commit found on the
branch → nothing rotates, and the next fire will see the same evidence again — a
session that ran but produced no reviewable record shouldn't get to silently mute
future fires on that artifact.

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

- **Rotation happens per-branch, not per-merge.** A session's `TUNING.md` commit
  triggers rotation as soon as the trigger script sees it on that branch — the PR
  doesn't need to be merged first. Deliberate: a "no mutation" record is
  documentation, not a shipped change, so it isn't gated behind the same harness-win
  Decide rule an actual mutation needs. But it does mean a PR that later gets rejected
  or heavily edited on human review has already had its source evidence rotated away
  from the live count. Not fixed — the alternative (wait for merge) can stall
  indefinitely on human review timing, which defeats the point of the dedup fix. Worth
  revisiting if a rejected `TUNING.md`-only PR is ever observed in practice.
- **No cleanup of `.reviewed-<stamp>` archive files.** They accumulate on the main
  checkout's disk forever. Harmless at current volume (gitignored, never pushed,
  local disk only) but unbounded. Not worth its own script yet.

## history

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
     wasted, duplicate work. Fixed with the rotation mechanism above.
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
