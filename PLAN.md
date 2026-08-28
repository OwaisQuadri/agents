# Intercept find/grep, redirect agents to fd/rg (Pi runtime only)

## Context

GitHub issue #84 ("Expose fd as the default file-finder over find") started as a
prose-guidance change to `CLAUDE.md` (drafted, then reverted). The owner re-scoped it
mid-session: instead of documentation, actively **intercept** `find`/`grep` bash
calls before they run and teach the agent the better `fd`/`rg` way to do the same
thing in the block reason — enforcement, not a hope that agents read `CLAUDE.md`.
Scope was also expanded from `find`→`fd` only to include `grep`→`rg`.

Both `fd` (sharkdp/fd) and `rg` (ripgrep) are already installed via Homebrew on this
machine (`fd 10.4.2`, `ripgrep 15.2.0`) but nothing in the repo steers agents toward
them. **This is also the first of what the owner expects to be more preferred-CLI
swaps over time** ("exposing CLI tools becomes easier") — the checker is built as a
small rule table from day one, not a `find`/`grep`-only special case, so the next
swap (e.g. `sed`→`sd`, `cat`→`bat`, `ls`→`eza`, whatever comes up) is one table entry,
not a rewrite.

## Decisions made in conversation (owner)

1. **Block + reason, not silent rewrite.** `fd`/`rg` flags are not 1:1 with
   `find`/`grep` — both skip `.gitignore`d and hidden paths by default, which
   `find`/`grep` never did; `fd`'s pattern is regex by default vs `find`'s glob. A
   transparent rewrite risks quietly changing what a search returns. Blocking with a
   reason teaches the agent to redo the call correctly itself.
2. **Catch `grep` everywhere**, including pipe-filter usage (`ps aux | grep foo`),
   not only grep-as-primary-search-command.
3. **Exempt `git grep`** — a distinct tool (searches git's index/tree, different flag
   set), not a 1:1 `grep`→`rg` swap.
4. **Also catch `xargs`-launched grep** (`find . -name '*.txt' | xargs grep pattern`)
   — this is exactly the shape `rg` replaces natively (recursive content search
   across files). The reason text for this shape says the whole `find | xargs grep`
   pipeline collapses to one `rg pattern <dir>` call, not a piecemeal swap.
5. **Pi coding-agent runtime only.** Claude Code/Codex CLI-hook wiring
   (`config/managed-settings.json`'s `PreToolUse:Bash`) is deferred — those harnesses
   may be deprecated (owner's words); a CLI-hook parity ticket gets filed separately
   if they stick around.
6. **Teach the idiomatic replacement, not a mechanical "equivalent."** A literal
   flag-for-flag translation (`find -name '*.rs'` → some `fd` flag that happens to
   match) is the wrong framing — the point is teaching the agent fd/rg's own better
   way to express the same intent, which is sometimes a completely different flag
   shape (`fd`'s `-e rs` extension filter instead of a `-name '*.rs'`-style glob
   translation) or, for `find | xargs grep`, dropping the pipeline entirely in favor
   of one native `rg` call. The reason text names common idioms, not derived
   translations.
7. **Build it as an extensible rule table**, not a find/grep-only implementation —
   this is the first of an expected series of preferred-CLI swaps, and adding the
   next one should mean adding a table entry, not touching the tokenizer.

## Approach

A **Pi extension** (`pi/extensions/preferred-cli-guard.ts`) hooks Pi's `tool_call`
event for the `bash` tool. On every bash call it shells out to a **Rust checker**
(`tools/preferred-cli-guard/`) that tokenizes the command string, walks it against a
small **rule table**, and decides allow/block.

### Rule table (the extensibility point)

```rust
struct Rule {
    banned: &'static str,                    // e.g. "find", "grep"
    preferred: &'static str,                 // e.g. "fd", "rg"
    also_catch_after_pipe: bool,              // grep: true: `| grep` also matches
    also_catch_via_xargs: bool,               // grep: true: `xargs grep` also matches
    exempt_prefix: Option<&'static str>,      // grep: Some("git") excludes `git grep`
    idioms: &'static [(&'static str, &'static str)], // shape hint -> idiomatic replacement,
                                               // e.g. ("-name '*.EXT'", "fd -e EXT")
    fallback_note: &'static str,              // shown when no idiom hint matches;
                                               // points at CLAUDE.md + `--help`
}

const RULES: &[Rule] = &[
    Rule { banned: "find", preferred: "fd", also_catch_after_pipe: false,
           also_catch_via_xargs: false, exempt_prefix: None,
           idioms: &[("-name", "fd -e <ext>  or  fd <pattern>"),
                      ("-type f", "fd -t f"), ("-type d", "fd -t d")],
           fallback_note: "..." },
    Rule { banned: "grep", preferred: "rg", also_catch_after_pipe: true,
           also_catch_via_xargs: true, exempt_prefix: Some("git"),
           idioms: &[("-r", "rg (recursive by default, no -r needed)"),
                      ("-i", "rg -i")],
           fallback_note: "..." },
];
```

Adding the next swap (`sed`→`sd`, `cat`→`bat`, ...) is one more `Rule` entry — the
tokenizer (program-name detection, pipe/xargs/env-assignment/quote handling) stays
untouched, matching `tools/no-ai-attribution`'s own const-array-of-rules shape
(`PATTERNS`, `WRITE_COMMANDS`) rather than introducing new config-file-parsing
infrastructure the repo doesn't already have (no TOML/JSON config dependency added —
"no dependencies unless the stdlib can't do it," per `tool-author`).

A block returns `{block: true, reason}` built from the matched `Rule`: the idiom hint
for the detected shape if one matches, otherwise the fallback note, plus the one
behavioral gotcha that matters for `fd`/`rg` (`.gitignore`/hidden-file skipping) and
the flag to opt back in — verified against `fd --help`/`rg --help` on this machine,
not recalled from memory (repo's `invariants.md` AGNT-INV-002: an external flag/API
semantic relied on by a checker needs a hand-built known-answer check before it's
trusted).

Rust, not pure TypeScript, because the detection needs command-position-aware
tokenization — walking past leading env-var assignments (`FOO=bar grep ...`), pipe/
`&&`/`;`/subshell boundaries, quoting (so `echo "grep foo"` doesn't trigger), `xargs`'s
own flags, and per-rule exemption prefixes (`git grep`). AGENTS.md's tooling-language
rule binds "parsers" to Rust explicitly, and `tools/no-ai-attribution`'s
`is_verb_sequence` + `VALUE_OPTIONS` tokenizer already solves this exact class of
problem for detecting `git commit` / `gh pr create` past intervening flags — reusing
that shape avoids a second, weaker implementation drifting from the proven one.

The checker degrades to **allow** on any internal parse failure or if the binary
isn't built yet (fresh checkout, cargo not run) — same posture as
`pi/extensions/logpath-guard.ts` and `hooks/rag-recall`: a broken guard must never
wedge every bash call in a session.

### Rejected alternatives

- **Prose-only `CLAUDE.md` guidance** (the original pass) — no enforcement, silently
  ignorable.
- **Silent argv rewrite** — correctness risk from the fd/rg vs find/grep flag
  mismatch (see decision 1), and it can't teach idiom over mechanical translation
  (decision 6) if it never surfaces to the agent at all.
- **Pure-TypeScript regex policy** (the `config-write-guard` shape) — insufficient
  for command-position-aware tokenization across pipes/xargs/quoting; would either
  over-block (false positives on substrings/quoted text) or under-block (miss
  `xargs`-launched grep, piped grep after flags).
- **A `find`/`grep`-only implementation with no rule table** — would work for this
  task alone but contradicts the owner's stated expectation of more swaps to come;
  refactoring a hardcoded two-command checker into a table later is strictly more
  work than building the table now.

## Files to modify

- `tools/preferred-cli-guard/Cargo.toml` — new checker crate.
- `tools/preferred-cli-guard/src/main.rs` — tokenizer + `RULES` table + block-reason
  logic, inline `#[cfg(test)] mod tests`.
- `pi/extensions/preferred-cli-guard.ts` — wiring: `pi.on("tool_call", ...)` →
  `pi.exec`/`spawnSync` the built binary → translate exit code to `{block, reason}`.
- `pi/extensions/preferred-cli-guard.test.ts` — `node --test` against the built
  binary (or a fixture, matching `logpath-guard`'s test shape).
- `config/tools.toml` — register the new Pi extension (`pi_extension` entry), same
  shape as every other entry (e.g. `telemetry`, `statusline`).

## Reuse

- `tools/no-ai-attribution/src/main.rs` — `is_verb_sequence` (anchors on a program
  name, walks past its options) and `VALUE_OPTIONS` (skips the next token for flags
  that consume a value, e.g. `git -C <path> commit`) are the tokenizer shapes to
  mirror for `find`/`grep`/`xargs` detection; its flat `PATTERNS`/`WRITE_COMMANDS`
  const-array shape is the model for `RULES`.
- `pi/extensions/logpath-guard.ts` — the exact Rust-shell-out wiring pattern:
  `spawnSync` the built binary, `existsSync` guard to degrade to allow when unbuilt,
  translate exit code to `{block, reason}`.
- `pi/extensions/logpath-guard/policy.ts` + `.test.ts` — shape for splitting pure
  decision logic from wiring, and the `node --test` fixture style to copy.
- `config/managed-settings.json`'s `PreToolUse:Bash` entry for `no-ai-attribution` —
  reference for the (deferred) CLI-hook wiring shape, not built this pass.

## Steps

- [ ] Write `tools/preferred-cli-guard/src/main.rs` tests first (TDD — clear
      input/output contract): must-block (`find . -name '*.rs'`,
      `grep -r pattern src/`, `ps aux | grep foo`, `cmd1 && grep foo bar`,
      `FOO=bar grep -i pattern`, `find /`, `find . -name '*.txt' | xargs grep
      pattern`), must-allow (`git grep pattern`, `fd . -e rs`, `rg pattern`,
      `echo "grep foo"`), must-not-false-positive (`cargo build --release # grep for
      errors`, `cat src/grep_utils.rs`).
- [ ] Implement the tokenizer + `RULES` table (starting with `find`→`fd`,
      `grep`→`rg`) + idiom-hint matching + fallback reason in
      `tools/preferred-cli-guard/src/main.rs`, stdin `{"command": "<bash command
      string>"}`, exit 0 = allow, exit 1 = block with reason on stdout.
- [ ] `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`,
      `cargo test` — all clean, quote exit codes.
- [ ] Write `pi/extensions/preferred-cli-guard.ts` wiring (mirrors
      `logpath-guard.ts`) and `pi/extensions/preferred-cli-guard.test.ts`.
- [ ] `node --test pi/extensions/preferred-cli-guard.test.ts` clean.
- [ ] Register `preferred-cli-guard` in `config/tools.toml`.
- [ ] Re-verify the `fd --help`/`rg --help` flag claims baked into the reason text
      match the installed versions before landing.

## Follow-ups (not this task)

- **File a GitHub issue**: auto-start Plannotator plan mode whenever `/engineer`
  reaches its Plan step (step 2), instead of relying on the skill's own
  "write plan.md, submit through plannotator_submit_plan if available" prose. Raised
  by the owner while this very plan was being written in Plannotator. Owner confirmed:
  file it immediately once this plan is approved and plan mode exits, before Implement
  starts on preferred-cli-guard (filing a new backend item goes through `/ideate`'s
  filing gate, and `gh issue create` is a side effect plan mode doesn't permit right
  now).
- CLI-hook parity (`config/managed-settings.json` PreToolUse:Bash) for Claude
  Code/Codex — deferred pending the harness-deprecation decision (owner's words);
  file as its own GitHub issue if those harnesses stick around.
- The prose `CLAUDE.md` `## file search` section (drafted then reverted) could still
  ship as a companion once the interception exists, since the block-reason text
  alone doesn't cover a human's own shell habits, only agent tool calls. Not
  building it in this pass.
- Future rule-table entries (`sed`→`sd`, `cat`→`bat`, `ls`→`eza`, or whatever the
  owner names next) go through the same `/pick-task` → `/engineer` flow, each as its
  own small task — this plan only seeds the table with `find`/`grep`.

## Verification

- `cargo test --manifest-path tools/preferred-cli-guard/Cargo.toml` — all fixtures
  pass.
- `cargo fmt --manifest-path tools/preferred-cli-guard/Cargo.toml -- --check` and
  `cargo clippy --manifest-path tools/preferred-cli-guard/Cargo.toml --all-targets --
  -D warnings` clean.
- `node --test pi/extensions/preferred-cli-guard.test.ts` — wiring tests pass.
- Manual smoke test in a live Pi session: run `find . -name '*.rs'` as a bash tool
  call, confirm it blocks with a reason naming the `fd -e rs` idiom (not a mechanical
  flag translation); run `grep pattern src/`, confirm it blocks with a reason naming
  `rg`; run `git grep pattern`, confirm it is allowed.
