# Self-Reflection Pi Extension ("reflect" — working name)

## Context

Build a Pi extension that runs an implicit, psychologically-grounded self-reflection
practice embedded in real work — not a separate journaling app. Full design history is
in the preceding conversation; the load-bearing decisions:

- No self-report questions ever. Prompts elicit narrative about concrete actions/events
  (including third-person/documentary framing, decision cross-examination, projective
  writing about others) and an AI infers patterns from the narrative — never asks "what
  skills do you have" directly.
- Two entry points: (1) **passive** — fires when a background subagent has been running
  a while and you've gone idle; (2) **active** — a manual command you invoke anytime.
- Every session is a fully interactive, open-ended conversation (min word count is a
  floor on the opening answer, not a cap on the exchange) — the AI keeps digging until
  something real surfaces.
- No MCP server. Data stays inside Pi; the extension exposes retrieval as a native Pi
  tool instead.
- Inference (narrative-identity themes, implicit motive imagery, LIWC-style linguistic
  markers) is always shown with a confidence/data-volume label — never asserted as fact.
- North star / success metric: surfacing content/project/startup ideas worth being proud
  of. Memoir compilation and a multi-perspective fiction/worldbuilding tool are related
  but explicitly backlogged, separate products.
- Duration prediction for the passive trigger is deferred — no historical (prompt →
  duration) data exists to calibrate against yet (`~/.pi/agent/run-history.jsonl` only
  keeps a redacted task hash). Ship a simple idle + elapsed-time threshold; log real
  (prompt, duration) pairs going forward so prediction becomes viable later.
- Single user (you) for v1. No auth, no multi-tenant concerns.

## Repo & location (revised — standalone repo, not the agents monorepo)

**Decision: this ships as its own standalone repo at `~/Documents/Github/reflect/`**,
with its own GitHub remote, not inside `~/Documents/agents`. This is a real, already-used
pattern, not a workaround: `pi-subagents` itself is a fully standalone package
(`~/.cache/tool-sync/pi-subagents/`, its own `package.json` declaring
`"pi": {"extensions": ["./src/index.ts"]}`) that Pi loads independently of the agents
monorepo's `config/tools.toml` machinery. `reflect` will follow that same shape:
standalone `package.json` + `src/index.ts`, installed via `pi install <path-or-git-url>`
(confirmed real CLI command from `pi --help`) — no dependency on the agents repo at all.

**This means the repo does not exist yet.** Creating it (`mkdir`, `git init`, `gh repo
create` or equivalent, first commit, remote push) is real filesystem/git work that
happens as **Step 1 of implementation, after this plan is approved** — plan mode only
writes markdown, so nothing below has been executed yet.

### Installation: registered in the agents repo's `config/tools.toml` as a remote source

Corrected per your steer — not a bare `pi install <path>`. Verified real precedent,
`rag` (`~/Documents/agents/config/tools.toml`):
```toml
[[tools]]
name = "rag"
platforms = ["macos", "linux"]
commands = ["rag"]
mcp_server = "rag"
pi_extension = "pi/extensions/rag.ts"
source = { url = "https://github.com/OwaisQuadri/rag.git", revision = "e3fa88d5..." }
installer = { command = "./install.sh", args = [], preview_args = ["--dry-run"] }
```
`tool-sync` clones the repo, `git checkout --detach <revision>` (pinned SHA, bumped on
release), runs `installer.command` inside that checkout, then symlinks `pi_extension`
into `~/.pi/agent/extensions/`. `reflect` follows this exact shape:
```toml
[[tools]]
name = "reflect"
platforms = ["macos", "linux"]
commands = []
pi_extension = "src/index.ts"
source = { url = "https://github.com/OwaisQuadri/reflect.git", revision = "<pinned sha, bumped each release>" }
installer = { command = "./install.sh", args = [], preview_args = ["--dry-run"] }
```
**Resolved:** Rust checker kept — `install.sh` runs `cargo build --release --manifest-path reflect-linguistics/Cargo.toml` so the compiled binary is ready before `pi_extension` gets symlinked in.
So **two repos are touched**: `reflect` (new, all real code/PRD) is the source of truth;
`agents` gets exactly one new `[[tools]]` entry pointing at it. `tool-sync` doesn't read
`package.json` at all — it only needs the named `pi_extension` file to exist post-install,
so `reflect` doesn't strictly need `pi-subagents`'s `package.json` `pi.extensions` field
unless we want `pi install` to also work directly against the repo standalone.

- The `tool-author` skill's Rust-vs-TS computation split (LIWC-style counting → Rust,
  orchestration → TS) is a craft principle worth keeping regardless of which repo this
  lives in — open question below on whether to keep it here (determines the installer
  command: no-op if pure TS, a cargo build step if the Rust checker is kept).
- Dedupe check already run against the agents monorepo (clear, nothing named
  journal/reflect/introspect/diary/mentor exists there) — confirms no naming collision.
- **Data must never live in either repo.** Both have GitHub remotes. Journal entries are
  personal, sensitive narrative content — storage must live under `~/.pi/agent/` (like
  `telemetry.jsonl` does today), never inside a git-tracked tree.

## Approach

### Architecture: the extension is thin wiring, Pi itself is the "mentor" — in an independent, context-seeded session

Verified platform constraint: `newSession`/`switchSession`/`fork` exist ONLY on
`ExtensionCommandContext` (the context a registered slash command gets when the user
invokes it). Event handlers (`pi.events.on`, `pi.on`) receive a plain `ExtensionContext`
with no session-management capability, and there is no way to synthesize it — this is a
deliberate boundary in Pi itself (an extension silently switching your active session
out from under you mid-task would be genuinely dangerous). Resolved architecture:

- **Passive trigger fires a nudge, not a session switch.** The event handler can only
  `sendMessage` into your *current* session ("background task's been running a while —
  `/reflect`?"). It cannot and should not force a context switch on its own.
- **The actual reflection session is independent, seeded with context, and only starts
  when you act** — via the `/reflect` command (whether you typed it unprompted or in
  response to the nudge). The command handler has real `newSession` access:
  ```ts
  ctx.newSession({
    parentSession: currentSessionPath,
    setup: async (sessionManager) => {
      // seed with a short summary of what you were just working on
    },
  })
  ```
  This gets you "some context carried over, but independent" exactly as specified — the
  carry-over is deliberate and scoped, not passive bleed-through.
- Once inside that new session, it's just you talking to Pi — "fully interactive" comes
  free from Pi's own turn loop, no bespoke conversation manager needed. The extension's
  job is: (a) decide when to nudge, (b) seed the new session's opening context, (c)
  supply the *tools* Pi's agent calls during/after the conversation to persist an entry
  and retrieve past ones, (d) supply deterministic linguistic-marker stats as grounding
  context so the narrative inference isn't just LLM vibes.

### Passive trigger

- Listen on the existing `pi.events` bus (`subagents:started` / `subagents:completed` /
  `subagents:failed` — these are synthesized by the already-installed `pi-subagents`
  extension, not core `ExtensionEvent` types; confirmed live via `telemetry.ts`'s
  identical listener pattern).
- On `subagents:started`, record `{runId, startedAt}` in memory.
- Poll (or re-check on `ctx.isIdle()` transitions) whether: the run is still open (no
  matching `completed`/`failed` yet), elapsed time has passed a threshold, and
  `ctx.isIdle()` is true. **Caveat to confirm with you:** `isIdle()` means "not
  streaming," not "you've stepped away" — it's an approximation, not true attention
  detection. Fire the nudge via `sendMessage(..., {deliverAs: "followUp"})` once
  conditions hold — the nudge tells you to run `/reflect`, it does not switch sessions
  itself (see architecture note above).

**Elapsed-time threshold — concrete default, not left abstract:** a fixed constant for
v1, **90 seconds**, configurable (not derived from any per-agent-type prediction — the
historical `run-history.jsonl` data showed `agent_type` alone is a weak predictor, min
durations for several types are under 10s while medians span 40s–420s, so no type-based
rule would be reliable). 90s is short enough to catch genuinely long runs without firing
on quick single-file lookups, but this is a starting guess, not a tuned value. This
threshold is explicitly separate from, and a placeholder for, the deferred duration
-prediction work in FR7/Context above: once `reflect` has logged enough of its own
(prompt, duration) pairs, the fixed 90s constant can be replaced with a real per-task
estimate — v1 ships the constant, not the predictor.

### Active trigger

- `pi.registerCommand("reflect", {...})` (name TBD) — manually invokable anytime,
  independent of subagent state. Calls `ctx.newSession({parentSession, setup})` to open
  the independent, context-seeded reflection session and inject the opening prompt
  there.

### Data & inference split (Rust vs TS, per tool-author's own rule)

- **Deterministic linguistic markers** (LIWC-style word-category counts: pronoun use,
  affect words, cognitive-process words) are real computation → a new Rust checker,
  `tools/reflect-linguistics/`, takes text on stdin, emits category counts as JSON.
  Wired into the extension via `pi.exec()`, following the existing shell-out pattern
  (`live-diff.ts`, `herdr-activity/state.ts`).
- **Narrative-identity/motive-theme coding** is inherently interpretive (semantic, not
  countable) — no Rust checker fits this; it happens naturally when Pi's own agent reads
  back retrieved entries + linguistic stats through the retrieval tool, not as a separate
  inference service. No new "ML pipeline" to build.
- **Storage**: append-only JSONL under `~/.pi/agent/reflect/entries.jsonl`, one record per
  entry (prompt type, narrative text, modality, linguistic-marker stats, timestamp,
  session id) — same `mkdir(recursive) + appendFile(JSON+"\n")` pattern as
  `telemetry.ts:appendRecord`.
- **Retrieval tool**: `pi.registerTool` exposing something like `search_journal` /
  `get_reflection_patterns` — reads the JSONL, returns raw entries + linguistic stats +
  an explicit confidence/data-volume note (e.g. "12 entries over 9 days — early signal
  only") for Pi's own agent to reason over live when you ask a question.

## Resolved: modality tagging (voice vs. typed) — deferred

Verified: `pi-transcribe` inserts transcribed text via `editor.insertTextAtCursor(text)`
— once in the buffer it is indistinguishable plain text, no provenance field survives.
**Decision: drop modality tagging for v1.** Ship the core loop first; revisit (fork
pi-transcribe for a provenance side-channel, or build reflect's own dedicated dictation
path) once there's a validated need to distinguish voice from typed narrative.

## Dependencies

- **`pi-subagents` — hard, required dependency for the passive trigger only.** It's the
  extension that actually emits `subagents:started`/`completed`/`failed` on `pi.events`
  (confirmed: these are not core `ExtensionEvent` types, they're synthesized entirely by
  `pi-subagents`). If it's not installed/loaded, those events never fire and the passive
  path silently does nothing. `reflect` will **detect this defensively rather than
  assume it**: on `session_start`, check `pi.getAllTools()`/`getCommands()` for evidence
  `pi-subagents` is registered; if absent, log a one-time notice ("passive reflect
  triggering needs pi-subagents installed — `/reflect` still works standalone") instead
  of silently doing nothing forever. Documented as a required companion install in
  `README.md` — not declared as an npm `dependency` in `package.json` since it's a Pi
  extension, not an importable package.
- **`pi-transcribe` — NOT a dependency for v1.** Modality tagging is explicitly
  deferred (see below), so `reflect` does not hook into `pi-transcribe` at all currently
  — raised here only to close the question explicitly rather than leave it ambiguous.
  If modality tagging is picked up later, this would become a real integration point
  (see the three options already weighed in that section).
- No other extension dependencies. `reflect`'s own tool registration
  (`search_journal`/`get_reflection_patterns`) and command (`/reflect`) are
  self-contained.

## Files to modify / create

All paths relative to new repo root `~/Documents/Github/reflect/`:

- `PRD.md` — product requirements doc (drafted now, see `PRD.md` in this same plans
  output — will be copied into the new repo as part of Step 1).
- `package.json` — declares `"pi": {"extensions": ["./src/index.ts"]}`, mirroring
  `pi-subagents`'s real package shape.
- `src/index.ts` — wiring: event listeners, tool registration, command registration,
  `pi.exec()` calls into the Rust checker (if kept — open question below).
- `src/index.test.ts` — `node --test` against the wiring.
- `src/policy.ts` — pure prompt-selection/session-seeding logic, unit-testable without an
  `ExtensionAPI` mock (mirrors `config-write-guard.ts`/`policy.ts` split).
- `reflect-linguistics/` (Rust, if kept) — `Cargo.toml` + `src/main.rs`, LIWC-style
  category counter, inline `#[cfg(test)] mod tests`.
- `README.md` — what the extension does, pointer to PRD.
- **`~/Documents/agents/config/tools.toml`** — one new `[[tools]]` entry (see above),
  the only change made to the agents repo itself.

## Reuse

- `telemetry.ts` — event-listener pattern (`pi.events.on("subagents:started"|"completed"|"failed", ...)`) and JSONL append pattern (`appendRecord`).
- `pi-subagents/src/index.ts` — confirms exact event payload shape for `subagents:*` events.
- `config-write-guard.ts` + `config-write-guard/policy.ts` — wiring/policy split pattern to mirror for `reflect.ts` / `reflect/policy.ts`.
- `live-diff.ts` / `herdr-activity/state.ts` — `pi.exec()` shell-out pattern for the Rust checker.
- `herdr-state.ts` — `pi.registerCommand` pattern for the manual trigger.

## Steps

- [ ] **Step 1 (repo bootstrap, real filesystem/git work — execute only after plan approval):**
      create `~/Documents/Github/reflect/`, `git init`, private GitHub repo via `gh repo
      create --private`, first commit, push, remote confirmed.
- [ ] Copy `PRD.md` (drafted below) into the new repo root; file initial GitHub Issues
      from it (one per major component: passive trigger, active trigger, storage,
      retrieval tool, Rust linguistics checker, narrative prompt bank).
- [ ] Scaffold `reflect-linguistics/` Rust checker + `install.sh` + tests
- [ ] Scaffold `src/index.ts` + `src/policy.ts` + tests
- [ ] Wire passive trigger (subagent event listeners + idle/elapsed check → nudge only)
- [ ] Wire active trigger (`/reflect` command → `ctx.newSession({parentSession, setup})`)
- [ ] Implement JSONL storage under `~/.pi/agent/reflect/entries.jsonl`
- [ ] Implement `search_journal`/`get_reflection_patterns` tool
- [ ] Write the narrative prompt bank (implicit/projective prompts) as data, not hardcoded logic
- [ ] Add `[[tools]]` entry to `~/Documents/agents/config/tools.toml`, pinned to first tagged revision
- [ ] Install via `tool-sync`/`pi install`, confirm `/reflect` resolves in a real Pi session

## Verification

- `cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && cargo test --manifest-path reflect-linguistics/Cargo.toml`
- `node --test src/index.test.ts`
- `tools/tool-sync/target/release/tool-sync --repository-root "$HOME/Documents/agents" --manifest config/tools.toml --home "$HOME" --check` (run from the agents repo, validates the new entry resolves)
- Manual: dispatch a real background subagent, confirm the nudge fires in the current session after threshold once idle; run `/reflect` manually, confirm it opens an independent session seeded with current-session context; confirm an entry lands in `~/.pi/agent/reflect/entries.jsonl` and NOT in `git status` in either repo.
