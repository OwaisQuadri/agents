# Operationalize the accumulating frontier data

## Context

The previous plan shipped the plumbing: `evals/run.sh` now appends a per-candidate
score-vector line to `evals/frontier.jsonl` (+ the candidate's full text to
`evals/frontier/<id>.md`) every time it grades both slices, and `ai-author/SKILL.md`'s
GEPA loop step 2 (Propose) will sample its mutation parent from the Pareto frontier once
an artifact has ≥2 non-incumbent frontier members. That part runs itself — no new user
action needed to *produce* the data; it accumulates automatically every real Test run.

The open question is what a human (or an agent running the GEPA loop) actually *does*
with it going forward, end to end. Re-reading what got built surfaces one real gap and
one real open question, not yet resolved:

### Gap: nothing currently updates a frontier line's `accepted` field after Decide

`run.sh` writes `"accepted": false` at grading time by default (documented in the
template: it can't know the Decide verdict yet). If Decide (GEPA loop step 4) later
accepts that candidate, nothing today flips that line to `true` — the only way to
change it right now is either (a) hand-edit the JSONL line, or (b) re-run `run.sh` with
`ACCEPTED=true`, which re-grades all 17 cases through `claude -p` again just to change
one boolean. Both are bad: (a) is manual and unrecorded, (b) doubles API cost/time for
zero new information. This needs a small fix before the loop can be run for real without
babysitting it.

### Open question, resolved: accumulation-triggered, not time-triggered

You're right to reject a scheduled/cron trigger here (a `scheduled-ideation`-style
daily job) — that's the wrong shape for something that should fire on evidence
accumulating, not on the clock passing. Checked: every artifact's `logs/usage.jsonl` and
`votes/votes.jsonl` are currently EMPTY on disk (`ai-author`'s own included — 0 lines
each), confirming a time-based trigger would fire on nothing, every day, for a long
while. This is exactly `ai-author/SKILL.md`'s own "can a program do it?" test: "has this
artifact accumulated enough new usage/votes/frontier evidence since its last tune" is
decidable from files with zero taste involved — a checker, fired by the runtime on an
event, never a human remembering to check. The repo already has a working precedent for
exactly this shape: `hooks/rag-recall`, a thin Python `UserPromptSubmit` hook wired
through `config/managed-settings.json`'s `"hooks"` key, which calls out to a separately
built binary (`rag`) and stays silent (prints `{}`) when there's nothing to say.

**On the threshold itself — what the research actually supports, stated plainly:** last
turn's fetched sources (GEPA arXiv:2507.19457, ADAS arXiv:2408.08435, Reflexion
arXiv:2303.11366, the SICA/reward-hacking pair) never gave a trigger-cadence number —
that research explicitly named this as an unresolved gap: "no source found that
specifically discusses the low-volume, single-user, daily-signal-near-zero regime."
Every fetched source benchmarks against thousands of rollouts or production traffic,
not "how many new log lines justify one tuning pass for one person." So there is no
citation backing a specific number here — anything chosen is a locally-reasoned
convention, not a research-derived one, and should be labeled that way rather than
dressed up as literature-backed.

**Threshold, set by direct instruction rather than by picking a convention:** computed
the actual current mean across all 32 artifacts with an `evals/` dir in the MAIN
checkout (`~/Documents/agents`, branch `main` — this worktree's own gitignored
logs/votes were empty because worktrees don't share them, not because nothing's been
logged): usage mean = 15.81 lines (skewed hard by `anchor-verifier` at 240 — median is
closer to 1), votes mean = 0.84. Fixed thresholds, set directly (rounding the computed means to 15 and 2), held fixed
going forward (not recomputed daily — a moving-target mean that rises as artifacts get
tuned would make the bar harder to clear over time, which isn't what "threshold" should
mean here): **≥15 new usage lines OR ≥2 new votes** since last tune.

**Trigger mechanism, corrected mid-plan by direct instruction: daily, but conditionally
silent — not the SessionStart hook proposed above.** Run a cheap, zero-LLM-cost check
every day (reusing `scheduled-ideation`'s own launchd + `trigger.sh` install pattern,
which already proved the "launchd fires, `trigger.sh` calls the running herdr daemon's
socket API directly, no GUI Terminal, no TCC(Transparency, Consent, and Control) prompt"
mechanism works). The daily job runs ONLY the Rust checker (`tools/gepa-due`) — no Pi
invocation at all. If zero artifacts cross the threshold, the job exits there: no Pi
call, no cost, nothing logged. If ≥1 artifact crosses it, THEN the trigger script calls
Pi (same herdr-socket mechanism) with the due list, to surface/act on the nudge. This
replaces the `SessionStart` hook design above — dropped in favor of this, per direct
instruction, since it more directly matches "run every day IF at least one skill passed
the threshold, otherwise don't even call pi."

**Correction (caught mid-plan): the frontier condition is circular, not a leading
indicator, and cannot be the trigger.** `evals/frontier.jsonl` only gains a non-incumbent
member when a real Test round runs against a real candidate — which means an artifact
needs AT LEAST TWO tuning passes already behind it before "frontier ≥ 2" can ever
become true. A nudge meant to tell you "start tuning this" can't be gated on a
condition that only exists once you've already started tuning it (twice). So the
frontier-members count is dropped from the DUE trigger entirely — it stays useful as
information the checker reports ("N frontier members, sampling active" once true) but
never as the gate. The only genuine leading-indicator signal available before any
tuning has happened is `logs/usage.jsonl` / `votes/votes.jsonl` growth — real
operational evidence accumulating from actually using the artifact, which exists
whether or not it's ever been tuned. That means the usage/votes thresholds ARE the
trigger, not a secondary condition, and they're the ungrounded part: `≥3` / `≥2` were
pattern-matched off `scheduled-ideation`'s own correction-mining convention (≥2
occurrences before treating something as signal), not derived from anything the fetched
papers discuss — stated plainly, not dressed up as research-backed.

**Design (final, per direct instruction — daily cheap-check, conditional escalation):**
- A new Rust checker, `tools/gepa-due` (computation — counting/filtering jsonl lines —
  belongs in Rust per AGENTS.md's tooling-language rule), that, for every artifact under
  `skills/`, `agents/`, `workflows/` with an `evals/` dir, computes the SAME
  filtered-by-`prompt_version` count Reflect (step 1) already computes by hand:
  surviving `logs/usage.jsonl` lines and surviving `votes/votes.jsonl` lines since the
  last recorded `TUNING.md` mutation for that prompt_version. Compares each against the
  fixed thresholds above (≥15 usage, ≥2 votes). Prints ONLY the due artifacts (name +
  which condition + the actual count) as JSON to stdout, or nothing if none are due —
  "reports only its failures, never its passes" per `ai-author/SKILL.md`'s own rule.
  Exit code 0 with empty output when nothing's due; exit 0 with a due list otherwise
  (never a nonzero exit for "nothing found" — that's a valid, honest result, not an
  error).
- A new `workflows/gepa-due/launchd/com.owaisquadri.gepa-due.plist` +
  `workflows/gepa-due/scripts/trigger.sh`, mirroring `scheduled-ideation`'s own install
  pattern exactly (same `StartCalendarInterval` shape, same
  `$HOME/.claude/gepa-due/trigger.log` output convention). Daily, cheap-first:
  1. Run `tools/gepa-due` directly against the repo — no worktree, no Pi, no herdr call
     yet. This is the ONLY step that runs every single day.
  2. If it prints nothing (no artifact due), `trigger.sh` exits immediately. No Pi
     invocation, no worktree created, no cost incurred — not the common case today,
     though: at ≥15/≥2, 10 of the 32 real artifacts already qualify (see Verification), so
     the first live run is expected to escalate, not stay silent.
  3. If it prints ≥1 due artifact, THEN (and only then) proceed exactly like
     `scheduled-ideation`'s `trigger.sh` does from its step 3 onward: create a fresh
     worktree via the herdr daemon socket, seed a kickoff prompt naming the specific due
     artifacts and their counts, and let that session decide what to do (nudge the user,
     or actually run the GEPA loop — the SEEDED PROMPT decides, this workflow doesn't
     auto-dispatch a tuning pass unattended without a Pi turn in the loop).
- This still doesn't unattendedly ship a mutation — the daily job's only unattended
  actions are (a) the free Rust check, and (b) opening a Pi session that SEES the due
  list; an actual GEPA Decide (accepting a mutation) still requires that session's own
  judgment, same protection the existing GEPA loop already has.

## User experience: before vs after

**Before (today, nothing in this plan shipped):**
- Every Test run's per-case scores exist only for the seconds `run.sh` is running, then
  vanish — a rejected candidate's text and scores are gone the moment `run.sh` exits.
  Only a hand-written mean and a prose sentence survive, in `TUNING.md`.
- Nothing tells you an artifact is worth tuning. You'd only find out by manually running
  `wc -l` across every `logs/usage.jsonl` yourself, the way this session just did — or by
  not finding out at all. Right now, real counts already sit at 240/67/62/53/43 usage
  lines and 8/8/7 votes for several artifacts (`anchor-verifier`, `code-reviewer`,
  `debugger`, `rust-style`, `mouthpiece`, `ai-author`, `web-research-summarizer`), and
  nothing has surfaced that to you.
- Tuning any artifact is entirely on you to remember and initiate — open `ai-author`,
  name the artifact, run the loop by hand, every time.
- If you'd built the frontier feature from last turn's plan without this one, marking a
  winning candidate `accepted` would mean either hand-editing JSON or paying for a full
  17-case re-grade just to flip one boolean.

**After (once this plan ships):**
- Nothing changes about how you run `evals/run.sh` — you (or an agent) still runs it the
  same way during Test. The difference is invisible at the moment you run it: a
  `frontier.jsonl` line and a `frontier/<id>.md` file just appear alongside the usual
  stdout/stderr output, whether the candidate wins or loses. You don't do anything extra
  to make this happen.
- Once an artifact has 2+ real rejected-but-viable candidates on file, the next time you
  (or an agent) run Propose for it, the mutation starts from whichever past candidate
  covers the most distinct cases best — not always blindly from the incumbent. You don't
  choose this; it's just how Propose behaves once there's enough history to choose from.
- Every day at 3pm, unattended, a nearly-free check runs. On a day nothing has
  accumulated past the bar, you see nothing — no notification, no cost, no new worktree.
  Right now, though, 10 of your 32 artifacts already clear the bar
  (`anchor-verifier`, `code-reviewer`, `debugger`, `rust-style`, `mouthpiece`, `byline`,
  `maestro-tester`, `create-pr`, `ai-author`, `web-research-summarizer`) — so the very
  first real 3pm run is expected to actually fire, not stay silent.
- When it fires, you get what you already know from `scheduled-ideation`: a fresh
  worktree shows up in your workspace list, seeded with a Pi session that already knows
  exactly which artifacts are due and their real counts. You look at it whenever you next
  check in — nothing forces your attention immediately. That session can nudge you or
  actually walk a GEPA loop, but it never ships a mutation without a live turn deciding
  to — same protection the loop already has today, just no longer gated on you
  remembering to open it in the first place.
- When a Decide accepts a candidate, one `jq` one-liner marks it in `frontier.jsonl` —
  no re-grade, no manual JSON surgery.

## Will this ever recommend a Pi extension, and does a pre/post-skill hook exist?

**Part 1 — does GEPA already route to a Pi extension when warranted? Yes, already built,
nothing new needed.** `ai-author/SKILL.md`'s GEPA loop step 2 (Propose) already states:
"Every mutation states whether it is PROSE, a CHECKER, or a PI EXTENSION, against
question one above." Question one is the "can a program do it?" test that every mutation
already has to pass before it's allowed to just patch prose. So the day a real GEPA pass
finds a repeated, mechanizable inefficiency in how a skill runs, it's already required to
recommend a Pi extension (or a checker) instead of a prose fix — this plan doesn't need
to add that; it's inherited from the loop as it already stands.

**Part 2 — does a pre/post-SKILL hook exist in Pi's extension API today? Checked
directly against the installed package, not assumed: NO.** Read
`~/.bun/install/cache/@earendil-works/pi-coding-agent@0.84.2@@@1/dist/core/extensions/types.d.ts`
(the actual shipped type definitions) and every extension already in `pi/extensions/`
(`logpath-guard.ts`, `config-write-guard.ts`, etc.) to confirm the real event vocabulary.
The full list Pi's extension bus exposes: `tool_call`, `tool_execution_start/update/end`,
`agent_start/end/settled`, `before_agent_start`, `turn_start/end`,
`message_start/update/end`, `session_start/shutdown/before_compact/before_fork/
before_switch/before_tree/compact/tree/info_changed`, `model_select`,
`thinking_level_select`, `before_provider_request/headers`, `after_provider_response`,
`resources_discover`. `skill` appears in the types file exactly twice — as a config field
(`skillPaths?: string[]`) and in a doc comment about reloading skills — never as an
event. There is no `pre_skill` / `post_skill` / `skill_invoke` hook point.

The nearest thing buildable TODAY, entirely inside this repo's own `pi/extensions/`, is a
`tool_call` listener filtered to a `Read` call whose path matches a `SKILL.md` — the
exact pattern `logpath-guard.ts` already uses to filter `tool_call` down to `bash` calls
only. That's a real, shippable approximation, but it's not a true skill hook: it fires
when the file gets READ, which conflates "the skill's markdown was loaded" with "the
skill was actually invoked as a capability and ran its full multi-step recipe" — it can't
wrap the skill's whole execution the way `subagentStart`/`subagentStop` wrap a
dispatched agent's. A true pre/post-skill lifecycle event doesn't exist at the platform
level; this repo's `pi/extensions/` can only subscribe to events the underlying
`@earendil-works/pi-coding-agent` package already emits — adding a genuinely new event
type is a change to that package itself, not something addable from inside this repo.

**Recommendation on filing a ticket: not yet, and not speculatively.** This repo's own
rule (`AGENTS.md`: "Narrowing needs an observed false positive, never an imagined one")
applies here by the same logic in reverse — requesting a new platform capability on a
hypothetical need is the mirror-image mistake of narrowing on an imagined case. Right
now there is no LOGGED instance of a GEPA pass reaching a PI-EXTENSION verdict that
specifically needed skill-lifecycle granularity (vs. the tool_call-on-SKILL.md-read
approximation being good enough). The moment one does, that's real evidence for a ticket
with a concrete repro, not a guess. Until then: record this as a DEFERRED verdict in
`ai-author/TUNING.md`'s deferred heading — exactly the mechanism `ai-author/SKILL.md`'s
own "what arrived?" section already prescribes for a verdict reached but not executed
("tracked to execution or explicitly dropped, in writing... the artifact's `TUNING.md`
under its deferred heading"), so a later Reflect pass rediscovers it instead of it living
only in this chat.

## Approach (final)

1. Add a tiny **mark-accepted** step: a one-line `jq` (or python) command that flips a
   specific `candidate_id`'s `accepted` field to `true` in `evals/frontier.jsonl`
   in-place, no re-grading. Document it as the explicit last action of GEPA loop step 4
   (Decide) in `ai-author/SKILL.md`, right next to "note the accepted mutation ... in
   TUNING.md" (step 5) — same moment, same place a Decide already writes something down.
2. Write the actual **future workflow**, end to end, as a short new subsection in
   `ai-author/SKILL.md`'s GEPA loop (or its own short "## applying frontier data"
   section) so this exact question has a permanent, load-bearing answer instead of
   living only in this session's chat:
   - Reflect (step 1) already reads `frontier.jsonl` — nothing new here.
   - Propose (step 2) already frontier-samples once ≥2 non-incumbent members exist —
     nothing new here either; it just starts firing once real volume shows up.
   - Test (step 3): run `evals/run.sh <candidate>` (no `--holdout` flag, so both slices
     grade and a frontier line gets appended automatically).
   - Decide (step 4): apply the Holdout gating rule as today; if accepted, run the new
     mark-accepted one-liner against that run's `candidate_id` (printed by `run.sh` to
     stderr: `frontier: recorded candidate <id> (accepted=False)`) instead of a re-grade.
   - Record (step 5): unchanged, TUNING.md entry as always.
   - Pruning: once `evals/frontier.jsonl` exceeds 20 lines for an artifact, apply the
     drop-oldest-dominated rule from the template — this is currently prose only; decide
     below whether it needs a script or stays a manual "do this next time you're in
     there" instruction, given it'll be rare (20 real tested candidates per artifact is
     a lot at current volume).
3. Build the accumulation-triggered nudge (checker + hook + wiring), per the resolved
   design above.

## Files to modify (final)

- `skills/ai-author/SKILL.md` — GEPA loop step 4 (mark-accepted one-liner), and either a
  new short subsection or an expansion of step headers documenting the full
  Reflect→Propose→Test→Decide→Record cycle now that frontier data is part of it.
- `skills/ai-author/templates/eval-harness.md` — mirror the mark-accepted convention so
  every future artifact's copy of the harness carries it too, not just ai-author's own.
- `skills/ai-author/TUNING.md` — note this as a small follow-on fix once shipped.
- `tools/gepa-due/` (new) — Rust checker, per AGENTS.md's `tools/` convention. Computes
  surviving usage/votes counts per artifact filtered by current `prompt_version` (same
  logic Reflect step 1 already does by hand), against fixed thresholds ≥15 / ≥2.
- `workflows/gepa-due/launchd/com.owaisquadri.gepa-due.plist` (new) — mirrors
  `workflows/scheduled-ideation/launchd/com.owaisquadri.scheduled-ideation.plist`.
- `workflows/gepa-due/scripts/trigger.sh` (new) — mirrors
  `workflows/scheduled-ideation/scripts/trigger.sh`'s worktree-creation/herdr-socket
  mechanism, gated behind the cheap Rust check running first.

## Removed from scope (per direct instruction, superseding earlier design)

- `hooks/gepa-due` and the `config/managed-settings.json` `SessionStart` wiring are
  DROPPED — replaced by the daily launchd trigger above. Keeping both would mean two
  independent triggers computing the same due-list redundantly.

## Reuse

- `jq` is already installed (`/usr/bin/jq`) — a mark-accepted one-liner needs nothing new
  installed: `jq --arg id "$1" '(select(.candidate_id==$id) | .accepted) = true' -c` style
  filtering, or simpler, a tiny python one-liner matching `run.sh`'s existing style (it
  already does hashlib/json/os inline, no new dependency).
- `skills/ai-author/evals/run.sh`'s existing frontier-write block (just built) is the
  thing being extended, not replaced.
- `workflows/scheduled-ideation/scripts/trigger.sh` (read in full this turn, first ~60
  lines) is the exact mechanism to mirror: herdr daemon reachability check, worktree
  creation via `herdr worktree create`, no GUI Terminal / no TCC(Transparency, Consent,
  and Control) prompt because every call goes over the herdr socket API directly. The new
  `gepa-due` trigger.sh reuses this verbatim for its escalation path, just gated behind
  step 1 (the free Rust check) that `scheduled-ideation` doesn't have (it always spins up
  a worktree; `gepa-due` mostly won't).
- `workflows/scheduled-ideation/launchd/com.owaisquadri.scheduled-ideation.plist` (read
  in full this turn) is the exact plist shape to mirror — same
  `ProgramArguments`/`EnvironmentVariables`/`StartCalendarInterval`/log-path structure,
  new `Label` and script path only.
- Reflect (GEPA loop step 1, `ai-author/SKILL.md`) already defines the exact
  prompt_version-filtering rule the Rust checker needs to reimplement in Rust — no new
  logic to invent, just port the existing filter.

## Steps

- [ ] `skills/ai-author/templates/eval-harness.md`: add the mark-accepted convention —
      a `jq` one-liner that flips one `candidate_id`'s `accepted` field to `true` in
      `evals/frontier.jsonl` in place, no re-grade, run as the explicit last action of a
      Decide that accepts.
- [ ] `skills/ai-author/SKILL.md`, GEPA loop step 4 (Decide): add the mark-accepted
      one-liner as the explicit mechanical follow-up when a candidate is accepted.
- [ ] `skills/ai-author/SKILL.md`: add a short "applying frontier data" walkthrough
      (or fold into the GEPA loop's own step headers) that states, standalone, the full
      cycle now that frontier data exists: Reflect reads it → Propose samples from it
      once ≥2 non-incumbent members exist → Test (`run.sh`, no `--holdout`) appends to
      it automatically → Decide applies the unchanged holdout-gating rule, then runs
      mark-accepted if it ships → Record in TUNING.md as always.
- [ ] `skills/ai-author/TUNING.md`: add a deferred-verdicts entry — no true pre/post-SKILL
      lifecycle hook exists in `@earendil-works/pi-coding-agent@0.84.2`'s extension API
      (confirmed by reading its `types.d.ts` this turn); nearest approximation is a
      `tool_call` listener filtered to a `Read` of a `SKILL.md` path, same pattern as
      `pi/extensions/logpath-guard.ts`. File an upstream ticket only once a real GEPA
      Propose pass reaches a PI-EXTENSION verdict the approximation can't satisfy.
- [ ] `tools/gepa-due/`: new Rust checker. Input: repo root (default cwd). For every
      artifact under `skills/`, `agents/`, `workflows/` with an `evals/` dir: compute
      current `prompt_version` (same git-log command Reflect already uses), count
      surviving `logs/usage.jsonl` lines and `votes/votes.jsonl` lines matching it.
      Print a JSON array of `{artifact, usage_count, vote_count, reason}` for entries
      where `usage_count >= 15 OR vote_count >= 2`; print `[]` (and exit 0) when none
      qualify — never treat an empty result as an error.
- [ ] `workflows/gepa-due/launchd/com.owaisquadri.gepa-due.plist`: copy
      `scheduled-ideation`'s plist, new `Label`
      (`com.owaisquadri.gepa-due`), new script path, new log path
      (`~/.claude/gepa-due/trigger.log`). Same `StartCalendarInterval` (Hour=15,
      Minute=0) as `scheduled-ideation` — one daily 3pm cadence, two independent jobs.
- [ ] `workflows/gepa-due/scripts/trigger.sh`: step 1 runs
      `cargo run --release --manifest-path tools/gepa-due/Cargo.toml -- "$REPO"` (or the
      built binary directly once compiled) and captures stdout. If the JSON array is
      empty, log "nothing due" and exit 0 — no further steps run. If non-empty, proceed
      with `scheduled-ideation`'s trigger.sh steps 1 (herdr reachability) through worktree
      creation, but seed the kickoff prompt with the due list (artifact names, which
      condition, actual counts) instead of the fixed ideation mission text.
- [ ] Install docs (mirroring `scheduled-ideation`'s own "install (macOS)" section) in
      `workflows/gepa-due`'s own short doc: `cp` the plist, `launchctl bootstrap`,
      `launchctl kickstart` for an immediate test run.

## Verification

- [ ] Run the mark-accepted one-liner against one of the two real candidate_ids already
      in `skills/ai-author/evals/frontier.jsonl` from last turn's verification run
      (`2ec7834e` or `1a50ad24`) and confirm only that line's `accepted` field changes,
      nothing else in the file is touched or reordered, and no re-grade API call happens.
- [ ] Re-read the new "applying frontier data" section end to end and confirm it
      actually answers, standalone, the exact question you asked — what do I do, in
      order, the next time I tune an artifact that has frontier data.
- [ ] Run `tools/gepa-due` directly against the MAIN checkout (`~/Documents/agents`) and
      confirm its output matches the 10 artifacts already hand-verified this turn at the
      ≥15/≥2 threshold: `anchor-verifier` (240/0), `code-reviewer` (67/0), `debugger`
      (62/0), `rust-style` (53/0), `mouthpiece` (43/0), `byline` (15/0), `maestro-tester`
      (6/7), `create-pr` (3/3), `ai-author` (1/8), `web-research-summarizer` (0/8) —
      NOTE these are raw counts; the checker's real output should be lower where an
      artifact's `prompt_version` filter drops stale lines, so exact numbers may differ
      from this raw baseline — that's expected, not a bug, and worth confirming which
      artifacts actually drop out once filtering is applied.
- [ ] Run `workflows/gepa-due/scripts/trigger.sh` once manually (not via launchd) against
      the main checkout and confirm it reaches the worktree-creation step (since real
      artifacts ARE due today) — don't need to wait for a real launchd fire to prove the
      escalation path works.
- [ ] `launchctl kickstart` the installed job once and confirm the trigger log shows
      either "nothing due" (fast exit) or an actual escalation, matching what the direct
      `tools/gepa-due` run showed.
