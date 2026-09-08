---
name: tool-author
description: Use when authoring a new Rust checker in tools/, a new Pi runtime extension in pi/extensions/, or a new git hook in hooks/ symlinked via install.sh — or rewriting any of these. ai-author's "question one" routes decidable work here once it has already ruled out prose. Skip for a skill, agent, or workflow (skill-author, agent-author, workflow-author own those), and skip for a one-off script with no reuse (ai-author's "should it exist" gate owns that call).
metadata:
  minimum-tier: T4
  short-description: The craft of authoring checkers, Pi extensions, and git hooks
---

# tool-author

Three destinations, one craft shell. ai-author's "question one" names two: a checker
an agent invokes and an extension that the runtime fires. The runtime has two backends in
this repository: Pi's event bus and Git. All three destinations are code, not prose.
Correctness comes from a test suite, not a GEPA(Genetic-Pareto prompt evolution) loop.

## 0. Dedupe and re-check the destination first

Read every existing `tools/*/Cargo.toml` name, every `pi/extensions/*.ts` file, and
every file in `hooks/`. If one owns the capability, extend it. Do not author a sibling
that does the same check.

Then re-run ai-author's own test before writing a line: which quantity does this measure,
and is it the quantity you actually care about? A checker or extension that is
deterministic about the wrong thing carries false authority and fires on every run — worse
than nothing. Say the quantity out loud before touching `tools/` or `pi/extensions/`.

## 1. Pick the destination

All three trigger on the same question: can a program decide this? They split on who
calls it and which runtime acts.

- **an agent invokes it deliberately, on a file or diff already in front of it** → a
  checker, Rust, `tools/<name>/` (section 2).
- **the Pi coding-agent runtime must fire it on its own event** (a tool call about to
  execute inside a Pi session, a session starting, a render) → a Pi extension,
  TypeScript wiring, `pi/extensions/<name>.ts` (section 3).
- **git itself must fire it on a git lifecycle event** (`checkout`, `merge`, `rewrite`),
  independent of any agent or CLI → a git hook, `hooks/<name>`, symlinked into
  `.git/hooks/` by `install.sh` (section 3.5).

The `rust-style` skill governs every `.rs` file regardless of destination; nothing here
repeats its baseline. Read `~/Documents/agents/docs/code-style.md` before writing any of
the three languages involved (Rust, TypeScript, and Bash), same as every other artifact
in this repository.

## 2. Author a checker (`tools/<name>/`)

Layout, learned from `tools/gepa-due` and `tools/ste-check` — two of the most legible
examples in the repo:

```
tools/<name>/
  Cargo.toml     # name, version, edition — no dependencies unless the stdlib can't do it
  src/main.rs    # doc comment: what bug this catches + the root cause, in 3-5 lines
                 # pure logic functions first, main() last, #[cfg(test)] mod tests inline
```

- **Report only failures.** `println!("pass  {name}")` is optional noise; a silent
  `ExitCode::SUCCESS` on a clean run and one `FAIL <path>: <reason>` line per real failure
  is the contract every eval harness and every human reads. A 20-line report to say three
  things is three things and seventeen wasted (ai-author's own rule; it applies here too).
- **Exit code carries the verdict.** `ExitCode::SUCCESS` / `ExitCode::FAILURE` — no other
  channel. A caller (agent or CI) checks `$?`, never parses prose to find out if it passed.
- **Tests live inside `main.rs`** under `#[cfg(test)] mod tests`, not a separate `tests/`
  file, unless the checker grows past what one file should hold. Cover: the pass case, the
  failure case, and the edge the bug you are guarding against actually hit — a case that
  can't fail on the fixed code isn't testing the bug.
- **Before shipping:** `cargo fmt -- --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test`, all three clean. Quote the exit codes in your report; a claimed
  pass with no command run is not evidence.
- **No registration step.** A checker isn't loaded by anything — it's built and invoked on
  demand (`cargo run --release --manifest-path tools/<name>/Cargo.toml -- <args>`). The
  only place it needs to be findable is named in the skill/agent contract that calls it
  (ai-author's "Done when" is the pattern: name the checker, don't restate its logic).

### Repair tier — apply it before the checker just reports

Per ai-author's three tiers, in order: **IMPOSSIBLE** (the artifact literally cannot take
the bad shape — prefer this whenever the producer is also under your control), then
**DETERMINISTIC REPAIR** (the checker fixes the one-correct-answer case itself instead of
just failing), then **SPAN-SCOPED REPAIR** (report the failing span for a cheap model to
patch). A checker that only ever reports, when it could apply the one correct fix, is
doing tier-3 work at tier-1 cost for no reason.

## 3. Author a Pi extension (`pi/extensions/<name>.ts`)

The extension HOST only loads `.ts` — every one of the 32 entries in `config/tools.toml`
points `pi_extension` at a `.ts` file, no exceptions, because that field is what the
runtime evaluates in-process. That is not a style preference; it is what the loader
accepts. It binds the WIRING file only — the `pi.on(...)` registration — never the
decision logic behind it.

Two shapes for where that logic lives, pick by whether the repository's Rust rule reaches it
("computation: checkers, parsers, scanners, anything whose runtime is its own work" vs.
"shell that only orchestrates other processes"):

- **Pure TypeScript policy**, when the check is a few string/path comparisons with no
  real computation — learned from `pi/extensions/config-write-guard/`:

  ```
  pi/extensions/<name>.ts          # wiring only: pi.on(...) calls, imports the policy
  pi/extensions/<name>/policy.ts   # pure functions, unit-testable with no ExtensionAPI mock
  pi/extensions/<name>.test.ts     # node:test + node:assert/strict against the pure functions
  ```

- **Rust binary behind a thin TS shell-out**, when the logic already exists as a `tools/`
  checker, or is real computation (parsing, a non-trivial scan) that belongs in Rust per
  the repository guidance — confirmed possible: `pi.exec(binary, args, options)` is a first-class
  `ExtensionAPI` method, and `observational-memory/src/spawn/launch.ts`, `herdr-activity/
  state.ts`, and `live-diff.ts` already shell out rather than compute inline:

  ```
  tools/<name>/                    # the Rust checker (section 2), exit code carries the verdict
  pi/extensions/<name>.ts          # wiring: pi.on(...) calls pi.exec on the built binary,
                                    # translates its exit code into {block, reason} or undefined
  pi/extensions/<name>.test.ts     # node:test against the wiring, using a fixture binary or
                                    # the real one built in a beforeEach
  ```

  Prefer this shape whenever a `tools/` checker for the same rule already exists — the
  runtime guard becomes a wrapper around it, never a second implementation that can drift
  from the first.

Whichever shape, register the wiring file in `config/tools.toml` — nothing loads without an entry there, one `[[tools]]`
block per file that needs to resolve at runtime (the wiring file, and the policy
subdirectory if it is imported as its own module):

```toml
[[tools]]
name = "<name>"
pi_extension = "pi/extensions/<name>.ts"
source = { path = "." }
installer = { command = "/usr/bin/true", args = [], preview_args = [] }
```

The event contract that matters most — intercepting a tool call before it runs:

```ts
import { isToolCallEventType, type ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function myExtension(pi: ExtensionAPI): void {
  pi.on("tool_call", (event) => {
    if (isToolCallEventType("bash", event)) {
      const reason = myPolicyCheck(event.input); // event.input: {command: string}
      if (reason !== undefined) return { block: true, reason };
    }
  });
}
```

- `tool_call` fires **before** the tool executes — the only hook that can prevent a bad
  write from ever landing, not just detect it after. Reach for it over any other event
  whenever the goal is "this must never happen," not "notice when it happened."
  `isToolCallEventType("edit"|"write"|"bash", event)` narrows `event.input` to the right
  shape; check the exact tool names the extension needs, never all of them.
- Returning nothing (`undefined`) allows the call. Returning `{block: true, reason:
  string}` stops it and shows `reason` to the agent that tried it — write `reason` as an
  instruction the agent can act on immediately (what to do instead), not a description of
  what went wrong.
- Other events exist for other timing needs — `session_start`, `session_shutdown`,
  `agent_settled`, `model_select`, `after_provider_response`, `message_end`,
  `feedback_received`, `resources_discover` — grep an existing extension using the one you
  need rather than guessing its event shape.
- **Tests:** `node --test pi/extensions/<name>.test.ts` (documented in `docs/testing.md`).
  Pure-TypeScript shape: test the `policy.ts` functions directly with mock inputs — never
  the wiring file, which needs no test once the policy it calls is covered. Rust-shell-out
  shape: test the wiring file itself against the built binary (or a fixture standing in
  for one), since the wiring is now where the exit-code-to-`{block, reason}` translation
  happens and that translation is the only logic left in TypeScript.
- **No shared root `package.json`.** Each extension is a standalone `.ts` file (or a small
  subdirectory) unless it needs its own dependencies, in which case it gets its own
  `package.json` + `vitest` the way `pi/extensions/observational-memory/` does — that shape
  is the exception, not the default.

## 3.5 Author a git hook (`hooks/<name>`, symlinked into `.git/hooks/`)

Fires on a git lifecycle event, independent of any agent or CLI — exempt from the Rust
rule by the repository guidance's carve-out ("shell that only orchestrates other processes", and
it names "the git hooks" as the example). Bash, in `hooks/`.

```
hooks/<name>          # bash, doc comment at the top: which git event, why, what it must
                       # never do (e.g. "never runs build_run or copies files between
                       # worktrees" — hooks/post-checkout's header is the model)
hooks/test.sh          # or a new companion script in the same shape if hooks/test.sh's
                       # existing scratch-repo harness doesn't cover the new hook
```

- **Wiring lives in `install.sh`**, not a config file — one `link "$REPO_TARGET/.git/
  hooks/<git-event>" "$REPO_TARGET/hooks/<name>"` line, alongside the existing block
  (search `install.sh` for `post-checkout` to find it).
- **Name the git event exactly** (`post-checkout`, `post-merge`, `post-rewrite`, ...) —
  the filename in `.git/hooks/` is fixed by git, not chosen; `hooks/<name>` in this repo
  should match it 1:1 so the `link` line stays legible.
- **Safety invariants belong in the header comment, not just the code** — a git hook
  runs unattended on every matching git operation in every worktree; state explicitly
  what it must never touch (existing example: "never runs build_run or copies files
  between worktrees").
- **Tests:** no Rust/Node runner reaches a git hook — build a scratch git repo, run the
  hook against it, assert the resulting tree/branch state, the way `hooks/test.sh` does
  against `hooks/post-checkout` (`PASS`/`FAIL` lines, a `$FAILS` counter, exit nonzero on
  any failure).

## 4. The authoring contract for THIS skill's own artifacts

This section is intentionally NOT the skill/agent/workflow eval-harness template. A
checker or extension is graded by its own test suite (section 2 and 3 above), never by a
GEPA loop scoring transcript excerpts — there is no "prompt" to reflect on, no trigger to
sharpen, no judge vote that makes sense for a function that either returns the right exit
code or doesn't. Do not paste `templates/eval-harness.md` into a checker or extension; it
is built for prose artifacts and does not fit.

What ships instead, every time:
- The test suite from the relevant section (2, 3, or 3.5), passing, with the exact
  commands and their exit codes quoted in the report — never a claimed pass with no
  command shown.
- For a checker: `cargo fmt -- --check`
  and `cargo clippy --all-targets -- -D warnings` clean, per `rust-style`.
- For a Pi extension: registered in `config/tools.toml`, or an explicit note that it is
  not yet wired in and why.
- For a git hook: the `link` line added to `install.sh`'s hook-wiring block, and the git
  event name matching the `.git/hooks/` filename exactly.
- One sentence in the calling skill/agent naming the tool and what it enforces — never
  restating its internal logic (ai-author's rule: "Where a tool owns a rule, the prose
  NAMES the tool and never restates its constant").

## evals

`evals/run.sh` grades the dry PLAN this skill produces — destination picked, file
layout, test command, wiring step — against `evals/cases.jsonl` using `evals/rubric.md`;
never writes a real file. A Pi-backed replacement runner disables extension discovery and
loads `pi-anthropic-auth` as the bare minimum extension. Run it after editing this skill;
`--holdout` runs the held-out slice.

## Done when

- Destination picked with the section-1 test stated, not assumed.
- Checker: `Cargo.toml` + `src/main.rs` with inline tests; `cargo fmt -- --check`, `cargo
  clippy --all-targets -- -D warnings`, `cargo test` all clean, exit codes quoted.
- Pi extension: policy/wiring split (or Rust-shell-out split); `node --test` passing;
  registered in `config/tools.toml` or the gap named.
- Git hook: wired via an `install.sh` `link` line; a `PASS`/`FAIL` test script against a
  scratch git repo, exit nonzero on any failure.
- The calling skill/agent names the tool in one sentence, no restated logic.
