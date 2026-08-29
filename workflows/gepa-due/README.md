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

The trigger script creates a fresh herdr worktree (same mechanism
`workflows/scheduled-ideation/scripts/trigger.sh` already proved live — no GUI Terminal,
no TCC(Transparency, Consent, and Control) prompt, every call over the herdr daemon's
socket API) and seeds a kickoff prompt naming exactly which artifacts are due and their
real counts. That session decides what to do — run a real GEPA tuning pass, or just
surface a note for later — but it never ships a mutation without going through the GEPA
loop's own Decide gate. This is a nudge, not an unattended prompt-editor.

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

- **No dedup across consecutive fires on the same due artifact.** Live-verified
  2026-08-29: two real launchd-triggered runs, ~3 minutes apart, both flagged
  `agents/anchor-verifier` due (its `usage_count`/`prompt_version` hadn't changed
  between them, since neither Reflect pass shipped a mutation) and both spun up a full
  worktree + live Pi session that independently reached the same "no mutation
  warranted" conclusion. Until either a real mutation ships (changing
  `prompt_version`) or new votes/usage lines accumulate, `gepa-due` will keep
  re-firing on the same artifact every single day, each time paying for a full
  worktree + Pi session to re-derive a conclusion already on record in that artifact's
  `TUNING.md`. Same shape `scheduled-ideation`'s own deferred list already names for
  itself ("no persistent dedup across daily runs") — deferred here for the same
  reason: no fix attempted yet, revisit once real repeated-day evidence shows how much
  it actually costs. A cheap first mitigation, not yet built: `tools/gepa-due` could
  read the due artifact's own `TUNING.md` for a dated "no mutation, reason: X" entry
  newer than its last usage/vote line and skip re-firing on it — deterministic, no
  new judgment needed, but out of scope for this pass.

## history

- 2026-08-28/29, founding version. Built in the same session that added Pareto-frontier
  candidate selection to `skills/ai-author/SKILL.md`'s GEPA loop — this closes the loop
  on "how do you actually apply the accumulating data": daily cheap-check, escalate only
  on real evidence. Threshold (≥15 usage / ≥2 votes) set by direct instruction after
  computing the actual repo-wide mean (15.81 / 0.84) across all 32 eligible artifacts;
  full reasoning trail in that session's `PLAN.md`.
