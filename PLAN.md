# Replace self-reported usage logging with mechanically-derived usage evidence

## Before vs after: what "usage logging" means (direct answer to plan review)

**Before:** every artifact's own definition carries a `## logging` section instructing
the agent, at the end of a use, to hand-write and append one JSON line to its own
`logs/usage.jsonl` (ts, trigger, excerpt, prompt_version, outcome, notes). Unenforced,
self-reported, frequently skipped. `TUNING.md` records mutation history and no-mutation
Reflect outcomes. Judge votes are supposed to follow "after logging" but almost never
do in practice.

**After:** no artifact writes anything, ever. `logs/usage.jsonl` and `TUNING.md` are
both eliminated from the authoring contract entirely. `tools/gepa-due` counts real `read`
tool_call hits on an artifact's own definition path directly from Pi session
transcripts (bounded scan, filtered to hits after the artifact's last-modification
timestamp — see "defining 'since last modification' precisely" below — and after
`reviewed_through` from the new dedup state file, whichever is later). `ai-author`'s Reflect step does the same
scan for its own failure-histogram/mutation-parent analysis, at tuning time, instead of
reading pre-summarized lines. `evals/frontier.jsonl` remains the only durable
per-artifact record (score vectors of tested candidates only — no narrative, no
no-mutation record, an accepted gap). Votes: see "When do votes get generated" below —
this is the one place the mechanism needs a genuinely new trigger, not just a removal.

## When do votes get generated? (real gap, fixed below)

Today's trigger ("after logging, dispatch a fresh-context judge", `ai-author/SKILL.md`'s
judge protocol section) loses its hook point entirely — there's no more logging step to
follow. `gepa-due`'s existing zero-vote fallback (dispatch N judges over the most recent
*usage lines* when `vote_count=0`, built earlier this session) also breaks, since its
input source (`logs/usage.jsonl`) is gone.

**Fix: votes move entirely to `gepa-due`'s Reflect-time dispatch**, and stop trying to
happen live/in-session. When a due artifact's Reflect pass runs (triggered by the
transcript-hit count crossing threshold, same as today), the dispatched session samples
`JUDGE_SAMPLE_SIZE` (default 5, unchanged) of the most *recent real transcript hits*
found by the same scan `tools/gepa-due` used to count evidence — not usage lines, since
those don't exist — and dispatches one fresh-context judge per sampled hit, each judge
reading the actual transcript excerpt around that use and voting via
`scripts/submit_vote.py` exactly as today (unchanged: script-only, blind, prefixed with
`prompt_version`). This makes vote generation *only* happen at tuning time, for a bounded
sample, rather than a live per-use action nobody reliably did anyway — arguably closer to
how it worked in practice already, just made the sole path instead of an unreliable
fallback.

This needs: `ai-author/SKILL.md`'s judge protocol section rewritten (drop "after
logging", the sampling now always comes from `gepa-due`'s dispatch, not a live-use
trigger); `workflows/gepa-due/scripts/trigger.sh`'s existing zero-vote-fallback kickoff
prompt updated to sample transcript hits instead of usage lines (its `vote_count=0`
branch already exists, just needs its input source swapped); and keeps the existing threshold: judge-dispatch fires only when `vote_count` is still
low (same bar `trigger.sh`'s current zero-vote fallback already uses), not on every
Reflect pass — confirmed, matches the existing cost-conscious design rather than adding
unconditional judge spend to every tuning pass. The transcript window sampled is
the same time-cutoff window `tools/gepa-due` computes for counting (see "defining
'since last modification' precisely" below), not a separate window.

### Due-trigger simplification: usage-only, vote_count drops out of the threshold

Since votes can now *only* be generated as an output of an already-due Reflect pass
(there is no other path left to create one), a `vote_count >= 2` due-trigger is
structurally circular — exactly the same shape as the frontier-count circularity caught
and removed earlier this session ("non-incumbent frontier members only arise after
tuning has already occurred"). `tools/gepa-due`'s due decision becomes **usage_count
alone** (`>= 15`, unchanged constant): `VOTE_THRESHOLD`/`vote_due` is removed from the
due-decision branch entirely. `vote_count` stays computed and included in the JSON
output — informational context for the dispatched session (whether to run the zero-vote
judge-sampling branch), never a trigger, same treatment the frontier count already got.

### Full pipeline automation, confirmed: human gate is PR review only

Confirmed direction: once a Reflect pass runs (with judge sampling first if
`vote_count` is low), the *same* dispatched session continues autonomously through
Propose → Test (`evals/run.sh`) → Decide (mark `accepted:true` in `frontier.jsonl` if it
wins) → push branch → open PR, with **no intermediate human checkpoint** before running
evals. This already matches how `trigger.sh` dispatches today (one continuous session
to PR-open, never auto-merging) — nothing structurally new to build here, just making
it explicit in `ai-author/SKILL.md`'s rewritten GEPA loop description and
`gepa-due`'s kickoff prompt wording so it isn't ambiguous: the sign-off gate is
reviewing/merging the PR, not a pause anywhere earlier in the loop.

## Context

Self-reported logging (every artifact's `## logging` section instructs the agent to
append a JSON line to its own `logs/usage.jsonl` at the end of a use) is unenforced and
unreliable: an audit found several real artifacts with plausible use and zero logged
lines. The user's direction: remove the self-report requirement from skill/agent/
workflow definitions entirely, across all authors — not just add a gap-detecting
checker alongside the existing self-report instruction.

This supersedes the earlier, narrower `skill-log-check` plan (a checker that only
*flagged* missing logs while leaving self-report in place). That checker is no longer
the end state; the mechanism it would have used (scanning `read` tool_call hits on an
artifact's definition path in real Pi transcripts) becomes the *replacement* source of
usage evidence, not just an auditor of it.

**Confirmed scope of what "remove logging from skill definitions" touches:**

- **38 artifacts** currently carry a `## logging` section (`grep -rl "^## logging"
  skills agents workflows --include="*.md"` in the main checkout) — every one needs its
  section removed or replaced.
- `skills/ai-author/templates/eval-harness.md` — the paste-ready `## logging` section
  every new artifact currently copies. Needs to stop instructing self-report.
- `skills/ai-author/SKILL.md` — the "authoring contract" file tree currently lists
  `logs/usage.jsonl # appended per the artifact's "## logging" section` as a required
  file, and the GEPA loop's Reflect step (step 1) reads `logs/usage.jsonl` filtered by
  `prompt_version` as its primary evidence source. Both need rewriting.
- `tools/gepa-due` — currently counts `logs/usage.jsonl` lines matching the current
  `prompt_version` as its due-threshold signal (`>=15` triggers a tuning pass). If
  `usage.jsonl` stops being written, this counting logic needs a new evidence source.
- `pi/extensions/logpath-guard.ts` + `tools/logpath-check` — exist specifically to
  validate the *path* of a self-reported `usage.jsonl` write. If nothing ever
  self-reports a write again, this extension has nothing left to guard.
- **`votes/votes.jsonl` and the blind-judge protocol are UNAFFECTED** — votes are a
  distinct, explicitly-invoked mechanism (`scripts/submit_vote.py`) separate from usage
  logging, and nothing here proposes changing them.

## What actually fires when a skill gets used (verified against a real transcript)

A skill invocation is an ordinary `read` tool call:

```json
{"type":"toolCall","name":"read","arguments":{"path":"/Users/owaisquadri/.agents/skills/ai-author/SKILL.md"}}
```

No distinct skill-invocation event exists in Pi's extension API (checked the installed
`@earendil-works/pi-coding-agent@0.84.2` type defs — no `session_end`, no
`skill_invoke`; `skill` appears only in `skillPaths` config). `arguments.path` matching
a known artifact definition path (`skills/*/SKILL.md`, `agents/*.md`,
`workflows/*.workflow.js`) is the only reliable, already-available signal — no platform
change needed. `tools/session-stats` already proves the transcript-walking pattern
works at scale; it just extracts token/cost fields today, never `toolCall` bodies.

## The real design tension (needs your call before I go further)

Self-reported logging is cheap (near-zero marginal cost — the same agent that just did
the work writes a 2-line summary immediately, with full context) but unenforced.
Mechanically-derived logging from transcripts is enforced (impossible to skip — a
Rust scan can't forget) but a bare `read` tool_call only proves *that* an artifact was
read, never *what happened* — it can't tell you the outcome (success/failure/partial),
what corrected it, or why. That qualitative signal is exactly what today's `outcome` and
`notes` fields carry, and it currently costs nothing extra to capture because the same
agent that just finished the work is still in context.

Two real options:

### Option A — Eliminate `usage.jsonl` entirely; Reflect reads transcripts directly

No file gets written at all. `tools/gepa-due`'s due-check and `ai-author`'s own Reflect
step both scan real recent Pi transcripts for `read` hits on the artifact's definition
path, using the artifact's last `TUNING.md` commit time (instead of `prompt_version`
line-matching) as the "since last tune" cutoff.

- Mechanical usage *counting* (does gepa-due's threshold trigger) becomes free of any
  write step — genuinely impossible to skip.
- Qualitative analysis (outcome/notes/failure histogram) moves from "written cheaply at
  use-time" to "read expensively at tune-time" — deferred to exactly the point
  `gepa-due` already dispatches a real agent session, which can read the real transcript
  excerpts around each hit directly instead of trusting a stale self-written summary.
  This is arguably a *better* signal (a fresh read of what actually happened, not what
  the busy agent remembered to jot down) at the cost of being pay-per-tune instead of
  pay-per-use.
- Retires `pi/extensions/logpath-guard.ts` and `tools/logpath-check` entirely — nothing
  ever writes a self-reported log line again, so there's nothing left to guard the path
  of.
- 38 artifact definitions + the eval-harness template + `ai-author/SKILL.md`'s
  authoring contract and Reflect step all drop the logging requirement/file from their
  required file tree.

### Option B — Keep `usage.jsonl`, but populate it mechanically instead of by self-report

A new mechanism (checker run periodically, or a live extension hooked to `tool_call`
like `logpath-guard` already is) auto-writes a `usage.jsonl` line the moment it detects
a qualifying `read` hit — same file format `gepa-due` and Reflect already know how to
read, so neither needs rewriting. The written line is necessarily thin (artifact, ts,
prompt_version, session pointer) since a bare tool_call can't supply outcome/notes.
Qualitative fields are either left empty permanently, or backfilled later by an agent
that revisits the line (adds a second write path and a "has this been enriched yet"
state to track — real added complexity).

- Smaller blast radius: `gepa-due`, Reflect, and the file format stay as they are.
- But it's mechanized bookkeeping bolted onto a format designed for self-report —
  outcome/notes either go permanently empty (weakens the failure histogram Reflect
  currently builds) or need a second enrichment pass (real complexity, arguably not
  simpler than Option A's "just read the transcript at tune time").
- `logpath-guard`/`logpath-check` would still have a job (validating the mechanical
  writer's own path), just no longer validating an agent-authored write.

**Refinement to Option A, prompted by a real question: does GEPA/golden-case mining
need logs at all?** Yes to evidence, but "evidence" doesn't have to mean self-reported
`usage.jsonl` specifically. Two distinct needs currently ride on it:

- **Reflect's trigger/histogram** — already has a zero-action evidence source: the Pi
  transcript itself, written by the platform regardless of what any artifact does.
- **Golden eval-case mining** — real and important (`anchor-verifier/TUNING.md`'s own
  history shows case `c6` rebuilt "from the 21 production failures" — that needed
  concrete extractable failure detail, which self-reported `excerpt`/`notes` cheaply
  supplied). This is the part Option A can't just drop.

**Superseded below**: the original sharpened-Option-A design routed durable output
into `TUNING.md`. The user then decided (see "Decision: TUNING.md is removed entirely"
below) to drop `TUNING.md` too, so that routing no longer applies — `evals/
frontier.jsonl` is the only durable per-artifact record left, and it does NOT capture
Reflect narrative or no-mutation outcomes. Golden-case mining going forward works from
a fresh transcript read at tuning time (whenever that pass happens to run, while the
transcript still exists) rather than from any persisted narrative — an accepted
tradeoff, not a solved one.

**Chosen: Option A** (`usage.jsonl` eliminated; transcripts are the live evidence
source) — confirmed by the user's follow-up decision to remove `TUNING.md` as well,
which only makes sense on top of Option A. Retires `pi/extensions/logpath-guard.ts` and
`tools/logpath-check` (nothing left to guard the path of).

## Decision: TUNING.md is removed entirely (confirmed)

User accepted the gap this creates: no record of a Reflect pass that found nothing to
mutate, and no persisted deferred/open-items list. `evals/frontier.jsonl` +
`evals/frontier/<candidate_id>.md` become the only durable record, and their actual job
is narrower than TUNING.md's was:

- **Test** appends a score-vector record + full candidate text automatically on every
  real `evals/run.sh` run, win or lose — the unconditional ledger of every candidate
  tried.
- **Propose** reads it to pick a mutation parent from the Pareto frontier once ≥
  2 non-incumbent candidates exist, instead of always restarting from the incumbent.
- **Decide** flips `accepted:true` in place on the winning line post-hoc.

It does **not** capture mutation rationale narrative, no-mutation Reflect outcomes, or
deferred items — all of that is the accepted gap. Consequence for `gepa-due`
specifically: with no durable per-artifact "already reviewed, nothing to do" marker,
a due-threshold artifact that gets a no-mutation verdict has nothing to prevent it
refiring and being fully re-analyzed again once new evidence accumulates past the same
threshold — this was the exact repeat-fire bug fixed earlier this session using
TUNING.md-freshness as the dedup signal (see `workflows/gepa-due/README.md`'s recorded
history). Removing TUNING.md removes that dedup signal too — replaced below.

### gepa-due's dedup replacement: a gitignored local state file, checked against live PR state

Dedup moves off the artifact entirely (artifacts carry zero tuning-status bookkeeping
now) and onto `gepa-due` itself, in `workflows/gepa-due/scripts/trigger.sh` — not the
dependency-free Rust checker, which stays pure evidence-counting with no network calls.

- New path, added to `.gitignore`: `workflows/gepa-due/state/reviewed.jsonl`. Lives in
  the **main checkout only** (not a per-artifact worktree, which is ephemeral and
  wouldn't persist across daily runs) — `trigger.sh` already runs from
  `GEPA_DUE_REPO`/the main checkout, so this is a natural fit.
- One line appended per dispatched review: `{"artifact":"agents/anchor-verifier",
  "pr_number":168,"branch":"gepa-due/agents-anchor-verifier/...","dispatched_at":
  "..."}`.
- Before dispatching a due artifact, `trigger.sh` looks up its most recent state entry.
  If none exists, proceed normally. If one exists, run `gh pr view <pr_number> --json
  state` (live check, since a locally-cached "open" could be stale) — if still `OPEN`,
  skip this artifact for this run (log "skipped: already has open PR #N") rather than
  dispatching a duplicate review. If `MERGED`/`CLOSED`, proceed and append a fresh
  state entry once the new run opens its own PR.
- This does add one `gh` call per due artifact with prior history — acceptable, since
  it only fires for artifacts already past the evidence threshold (rare in practice,
  same volume `gepa-due` already dispatches sessions for), and `trigger.sh` already
  depends on `gh` for opening PRs.

**Gap caught before implementation: the PR-open check alone doesn't prevent stale
re-triggering.** It only blocks a *duplicate* dispatch while a review is pending — once
that PR merges, nothing stops the SAME already-reviewed evidence from re-crossing the
threshold tomorrow, since (unlike `usage.jsonl`, which got physically rotated out after
review) transcript hits are never consumed. Fix: the state file's entry also carries a
`reviewed_through` timestamp, and `tools/gepa-due`'s own counting logic reads that same
gitignored state file (a local file read, no network call — stays consistent with the
checker's zero-network-cost design) and only counts transcript hits *after* that
timestamp as "new since last tune." No entry for an artifact yet → count everything, as
today.

## Resolved

1. ~~Option A or B~~ — **Option A**, confirmed.
2. ~~Exact artifact definition path shape~~ — confirmed by listing real directories:
   `skills/<name>/SKILL.md`, `agents/<name>/<name>.md`, `workflows/<name>/SKILL.md`
   (not `<name>.workflow.js` — that file is machine-executed by `SubagentWorkflow`,
   never agent-"read" the way a skill/agent definition is). `gepa-due` itself has none
   of these (no `evals/`), so it's naturally excluded from this whole effort.
3. ~~Transcript retention risk~~ — checked: `cleanupPeriodDays` retention is documented
   only for Claude Code sessions, not Pi. Residual risk (Pi could still prune
   internally, unconfirmed either way) is accepted, narrowed to the window between
   evidence accumulating and the next threshold-triggered Reflect pass.
4. ~~gepa-due's dedup replacement~~ — gitignored local state file +
   live `gh pr view` check in `trigger.sh`, see above.

## Open questions (need your answer before finalizing steps)

1. **Migration of the 38 existing `## logging` sections**: bulk-edit all of them in one
   pass now, or only touch each one the next time `gepa-due`/a GEPA pass naturally
   visits that artifact (slower, but each removal is verified against real evidence for
   that specific artifact rather than a blind sweep)?
2. **False positives from incidental reads** (an agent reading a sibling's `SKILL.md`
   while authoring, not "using" it) — proposing a default below; flag if you want
   something different.

### Proposed default for false positives (please confirm or override)

Accept the false-positive rate rather than try to disambiguate intent from a bare
`read` tool_call — there's no reliable signal in a single tool_call for "was this read
*as* the active skill for this turn's work, or incidentally." State this plainly as a
known limitation in `ai-author/SKILL.md`'s rewritten Reflect step and in `gepa-due`'s
own docs, the same way `gepa-due`'s README already states other known limitations
rather than silently absorbing them.

## Files to modify

- `tools/gepa-due/src/main.rs` — replace `logs/usage.jsonl` line-counting with bounded
  Pi-transcript scanning for `read` tool_call hits on each artifact's definition path,
  filtered by an explicit **time cutoff**, not a hash-equality match (see "defining
  'since last modification'" below). Due decision becomes `usage_count >= 15` alone —
  drop `VOTE_THRESHOLD`/`vote_due` from the due-decision branch (circular: votes can
  now only be produced by an already-due pass). `vote_count` stays computed against
  `votes/votes.jsonl` (unaffected format, unaffected `submit_vote.py`) and reported in
  the output JSON as informational context only, same treatment as the frontier count.

### Defining "since last modification" precisely (needed — hash matching no longer works)

Today's filter is string equality: a self-reported `usage.jsonl` line carries its own
`prompt_version` hash, so "does this line count" was just "does its hash match the
current one." A transcript's `read` tool_call has no `prompt_version` field at all —
only a timestamp — so equality-matching doesn't carry over. The filter becomes a time
cutoff instead:

1. Compute the artifact's current definition commit exactly as today
   (`git log -1 --format=%h -- <artifact_rel> :(exclude)**/evals/** :(exclude)**/votes/**`
   — drop the now-nonexistent `:(exclude)**/TUNING.md` and `:(exclude)**/logs/**`
   patterns since those paths no longer exist under the new contract; keep `evals/**`
   and `votes/**` excluded, since those still change on every test run and every vote
   without the artifact's own content changing).
2. Additionally fetch that same commit's **timestamp**, not just its hash:
   `git log -1 --format=%cI -- <same paths>` (ISO 8601 commit date).
3. Also read the gitignored `workflows/gepa-due/state/reviewed.jsonl`'s
   `reviewed_through` timestamp for this artifact, if an entry exists.
4. The effective cutoff is `max(last_modification_timestamp, reviewed_through)`. A
   fresh mutation naturally resets the count (its timestamp becomes newer than any
   prior `reviewed_through`, so counting restarts from the new version). An unchanged
   artifact that was already reviewed keeps counting only hits after that review,
   because `reviewed_through` becomes the later of the two. No `reviewed_through` entry
   yet → cutoff is just the modification timestamp, as today's semantics intended.
5. Count only `read` tool_call hits on the artifact's definition path whose transcript
   timestamp is strictly after that cutoff.
- `workflows/gepa-due/scripts/trigger.sh` — add the PR-open live check
  (`gh pr view <pr_number> --json state`) before dispatching a due artifact; append a
  `reviewed.jsonl` state entry (artifact, pr_number, branch, `reviewed_through`
  timestamp) once a session settles, regardless of outcome; rewrite the existing
  zero-vote-fallback kickoff prompt (low-`vote_count` branch) to sample recent real
  transcript hits instead of `logs/usage.jsonl` lines, since that file no longer exists.
- `.gitignore` — add `workflows/gepa-due/state/`.
- `skills/ai-author/SKILL.md` — rewrite: authoring contract file tree drops
  `logs/usage.jsonl` and `TUNING.md`; GEPA loop step 1 (Reflect) scans real transcripts
  instead of reading `logs/usage.jsonl`; step 5 (Record) is removed (nothing left to
  write to); routing section's TUNING.md-deferred-verdict references removed; "applying
  frontier data" section updated to drop TUNING.md/Record references; judge protocol
  section rewritten to drop the "after logging" trigger — judge dispatch now happens
  only via `gepa-due`'s Reflect-time sampling (low `vote_count` branch), never live.
- `skills/ai-author/templates/eval-harness.md` — remove the paste-ready `## logging`
  section entirely; update the required-file-tree diagram to drop `logs/`, `votes/`
  stays, `TUNING.md` drops.
- **38 artifact definition files** (bulk sweep) — remove each one's `## logging`
  section. List from `grep -rl "^## logging" skills agents workflows --include="*.md"`
  in the main checkout at plan time; re-run at implementation time since this is a
  living repo.
- **Every artifact's own `TUNING.md`** (currently exists per-artifact where authored) —
  removed as part of the same sweep. Confirm count and paths at implementation time
  (`find skills agents workflows -name TUNING.md`).
- `pi/extensions/logpath-guard.ts` + `tools/logpath-check` — retire. Confirm nothing
  else references `logpath-check`'s binary before removing (check
  `config/managed-settings.json` wiring and any other extension that might share it).

## Reuse

- Transcript-scanning approach mirrors `tools/session-stats`'s existing line-by-line
  JSONL walk of `~/.pi/agent/sessions/*.jsonl` (`tools/session-stats/src/main.rs`) —
  same bounded-read pattern, different extracted field (`toolCall` path instead of
  token/cost).
- Hand-rolled JSON field extraction (no parser dependency) reuses the pattern already
  in `tools/gepa-due/src/main.rs`'s `extract_json_string_field`.
- `trigger.sh`'s existing `gh` usage (already opens PRs) extends naturally to the new
  `gh pr view` state check — no new external dependency.

## Steps

- [ ] Confirm final list of 38 (re-run the grep at implementation time) and the
      per-artifact `TUNING.md` list; note any discrepancy from this plan's counts.
- [ ] Rewrite `skills/ai-author/SKILL.md`: authoring contract, GEPA loop steps 1 and 5,
      routing section's deferred-verdict language, "applying frontier data" section.
- [ ] Rewrite `skills/ai-author/templates/eval-harness.md`: drop the logging section,
      update the required-file-tree diagram.
- [ ] Bulk-remove `## logging` sections from all 38 artifact definitions.
- [ ] Bulk-remove all per-artifact `TUNING.md` files (git rm, not just content-empty).
- [ ] Rewrite `tools/gepa-due/src/main.rs`: transcript-hit counting instead of
      `usage.jsonl` line-counting, using the `max(last_modification_timestamp,
      reviewed_through)` time cutoff defined above (not hash equality — transcript hits
      carry no `prompt_version` field), updated tests (mirroring existing test shape in
      that file).
- [ ] Update `workflows/gepa-due/scripts/trigger.sh`: PR-open check before dispatch,
      state-file append after settlement, zero-vote-fallback prompt resampling from
      transcript hits instead of usage lines; update `workflows/gepa-due/README.md` to
      match.
- [ ] Add `workflows/gepa-due/state/` to `.gitignore`.
- [ ] Retire `pi/extensions/logpath-guard.ts` + `tools/logpath-check` (confirm no other
      reference first).
- [ ] State the incidental-read false-positive limitation explicitly in `ai-author`'s
      rewritten Reflect step and `gepa-due`'s README.
- [ ] Rewrite `ai-author/SKILL.md`'s judge protocol section: drop the "after logging"
      trigger; document that votes now originate solely from `gepa-due`'s low-
      `vote_count` Reflect-time sampling; document that once due, the same session runs
      Reflect → (judge sampling if low vote_count) → Propose → Test → Decide → PR
      autonomously, with the human gate at PR review/merge only, not any earlier pause.
- [ ] Remove `VOTE_THRESHOLD`/vote-based due branch from `tools/gepa-due/src/main.rs`;
      update its tests accordingly; `vote_count` remains in output, informational only.

## Verification

- `cargo test --release` + `cargo clippy --release` clean for the rewritten
  `tools/gepa-due`.
- `bash -n` on the rewritten `trigger.sh`.
- Run the rewritten `tools/gepa-due` against the main checkout with real transcripts;
  confirm it reports a plausible due list without reading any `logs/usage.jsonl`
  (confirm by checking it doesn't even look for that path anymore in the diff).
- Manually exercise the PR-open dedup: with a real open `gepa-due` PR pending (e.g.
  #168, if still open), confirm a due-check for that same artifact does not dispatch a
  second session.
- Confirm the rewritten zero-vote-fallback prompt in `trigger.sh` references transcript
  hits, not `logs/usage.jsonl`, and that a real low-`vote_count` due artifact still
  triggers judge sampling as before.
- Confirm `skills/ai-author/templates/eval-harness.md`'s file-tree diagram, a spot-check
  of 3-5 of the 38 edited artifact definitions, and `git status` show no stray
  `logs/`/`votes/`/`TUNING.md` references left behind.
- Confirm nothing else in the repo references `pi/extensions/logpath-guard.ts` or
  `tools/logpath-check` before removing them (`rg` sweep across `config/*.json` and
  other extensions).

## Explicitly out of scope for this pass

- Any change to `votes/votes.jsonl` or the blind-judge protocol.
- A live/real-time nudge (`turn_end`-based or otherwise).
- Filing an upstream Pi platform ticket for a native skill-invocation event.
