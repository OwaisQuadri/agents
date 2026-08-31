# agents

Skills, subagents, workflows, and config for my coding agents (Claude Code, Codex),
versioned in one repo with a symlink installer.

## install

```sh
cargo build --release --manifest-path tools/tool-sync/Cargo.toml
./install.sh --dry-run   # print the plan
./install.sh             # symlink for real; pre-write backups land outside the skills root
```

A fresh checkout needs the one-time build before its first dry run. The dry run exits with the required build command when the binary is absent.

`install.sh` never escalates. It runs from the `post-merge` hook, where a password prompt has no terminal to appear on. It reports policy drift and writes nothing under `/Library`.

### policy file

One separate step pins `env`, `statusLine`, and `hooks` in Claude Code's policy file, which outranks every writer of `~/.claude/settings.json`. Run it by hand after `install.sh`:

```sh
./install-policy.sh --dry-run   # print the exact file, escalates for nothing
./install-policy.sh             # prompts for your password before the write
```

Run it **as yourself, without `sudo`**. It escalates on its own, and prompts once, after it prints the diff and before it writes. `sudo` resets `HOME`. Running it with `sudo` would render every path against `/var/root`, into a root-owned file that needs `sudo` to correct. The script refuses to run as root.

Neither `--dry-run` nor an already-current policy file escalates at all, so neither prompts.

The policy file is the highest-precedence config on the machine. To remove it:

```sh
sudo rm '/Library/Application Support/ClaudeCode/managed-settings.json'
```

## layout

| path | holds |
| --- | --- |
| `skills/` | one `SKILL.md` per skill, loaded on trigger |
| `agents/` | subagent definitions, each with its own tools and model |
| `workflows/` | multi-agent graph specs |
| `config/` | `tools.toml`, the executable-tool manifest; `mcp-servers.toml`, the tracked MCP (Model Context Protocol) server manifest; `mcp-sync-state.toml`, the machine-written, untracked sync state; reference copies of `settings.json` and `settings.local.json` |
| `docs/` | prose style (the ASD-STE100 rules every register runs on), code style, comment style, docstring style (the standard generator per language), the executed reset spec, fleet research |
| `rules/` | Claude Code rules that load only for matching file paths |
| `tools/` | `tool-sync`, which installs executable tools; `ste-check`, which grades prose; `mcp-sync`, which renders the MCP server manifest; `tool-wizard`, which writes and updates `tools.toml` entries; `pr-review-filter`, which lists the PRs that start a review pass; `transcript-directed-video-processor`, which segments a YouTube or local video's transcript into candidate moments and runs a configured vision model over selected frames |
| `hooks/` | both git hooks and Claude Code hooks. `post-checkout` carries the live checkout's uncommitted work into worktrees and branches cut at main's tip, `test.sh` is its regression suite; `rag-recall` is the UserPromptSubmit hook that searches the personal RAG store on every prompt, registered for both Claude Code and Codex |
| `.conductor/` | repo settings for Conductor; its setup script runs `hooks/post-checkout` in every new workspace |
| `install.sh` | the top-level installer; it builds the local Rust tools, runs `tool-sync`, and runs `mcp-sync` when its live inputs exist |
| `CLAUDE.md` | global guidance loaded every session; the single instructions source for both tools. `install.sh` links `~/.codex/AGENTS.md` to it, so Codex reads the same file |

### skills

| skill | job |
| --- | --- |
| `ai-author` | decides whether a skill/agent/workflow should exist at all; owns the eval + logging contract and GEPA (Genetic-Pareto prompt evolution) tuning |
| `skill-author` / `agent-author` / `workflow-author` | deep authoring craft per artifact type |
| `agent-config-reset` | audit a sprawling agent setup and, gated on approval, reset it |
| `create-pr` | commit, push, and open the pull request |
| `engineer` | research → plan → implement → test → signoff → close for one picked task, gated at plan review and signoff, landed through `git-sync` |
| `pick-task` | interactively grills you and lands on one task to work on, from an existing backend or a hand-off to `ideate` |
| `ideate` | brainstorm and file new work: research, lateral reframing, grilling, then a human-gated filing step |
| `hq` | front door over every project: gates-first digest, worktree-isolated dispatch, drill-down into any project agent; two-stage launchd heartbeat keeps quiet cycles at zero tokens |
| `bro` | re-explains the last reply in plain words when it lost you; jargon goes, facts stay verbatim, no length cap |
| `byline` | de-slops prose that ships under your name: commits, PR bodies, tickets, READMEs; facts stay verbatim |
| `mouthpiece` | voice rules for end-user-facing replies, scored by `ste-check --register mouthpiece` |
| `rust-style` | applies the shared Rust baseline in Pi and Codex while Claude Code uses its matching path rule |
| `task-graph` | work items + deps → statused DAG or ABCD-NNNN tickets, rendered in mermaid |
| `vocabulary` | precise design and UI terms: exact lookup, near-synonym boundaries, reverse lookup from a vague ramble |
| `volley` | short-turn mode: every turn ends inside 30 seconds; Volley dispatches longer work and reports it on the next turn |

### agents

| agent | job |
| --- | --- |
| `anchor-verifier` | grade one worker's finished product against a dispatch rubric, on executed-command and file:line anchors only |
| `code-reviewer` | fresh-context diff review; ranked findings anchored to file:line |
| `debugger` | root-cause a failure with a named repro, apply the minimal fix |
| `maestro-tester` | one flow objective → Maestro YAML run → junit-anchored verdict |
| `spec-tester` | one natural-language test case or attack angle → harness-driven anchored verdict |
| `web-research-summarizer` | fan out over web sources, return a cited findings block |

### workflows

| workflow | job |
| --- | --- |
| `research-sweep` | answer one research question: fan out web (academic/news/design/UX) and codebase researchers over distinct angles, gap-check with an independent critic, fill what's missing |

## Herdr state

The `herdr-state` extension gives Pi a read-only view into the running Herdr session.

`/herdr-state` lists every open Herdr workspace and marks Pi's own workspace, tab, and pane. `/herdr-state workspace <workspace-id>` scopes the result to that one workspace's tabs and panes. `/herdr-state pane <pane-id> [line-limit]` reads that pane's recent output, bounded to its last 200 lines by default and to the requested `line-limit` when given. An explicit `line-limit` must be an integer from 1 through 10,000. The command only ever reads Herdr state; it never writes to it.

## Pi stack

The managed upstream stack pins these immutable revisions:

- `tintinweb/pi-subagents` at `3f9d35cd078d18a141eb5a6d8f4fc5010d756280`.

`pi-subagents` provides a live subagent viewer and inline steering.
- `backnotprop/plannotator` at `e1ce7dabe10474b3a653bef9ed5134b73e0b5336`.
- `humanlayer/skills` at `3c2629142c5d437428269b1b722b08c0b87f574d`.
- `mattpocock/skills` at `068b6e0c62393147daf03530149cdce209c93da8`.

`tool-sync` manages Pi packages and telemetry under `~/.pi/agent/extensions`. It links selected upstream skills under `~/.agents/skills` and caches Git checkouts under `~/.cache/tool-sync`. It derives Pi project agents under `~/.pi/agent/agents` from `agents/*/*.md`. The adapter maps only supported tools and models, and it preserves the prompt bytes.

Package extensions execute with full user permissions. Review every pinned update before installation.

### Managed Pi configuration

Pi configuration sources live in this repository. `tool-sync` links the extensions into `~/.pi/agent/extensions`; do not edit those links or any managed skill, agent, or instruction destination directly. Edit the source here, then run:

```sh
cargo build --release --manifest-path tools/tool-sync/Cargo.toml
REPO_TARGET="$PWD" ./install.sh
```

The `config-write-guard` extension blocks Pi `edit` and `write` calls that target managed destinations. It also blocks shell commands that name a managed destination, because a shell command cannot prove that it only reads the path. The guarded paths are `~/.agents/skills`, the managed `~/.claude` and `~/.codex` files, plus Pi's managed agents, extensions, and settings.

The source extensions provide `ask_user_question`, the `owais` theme, a custom header, and prompt snippets. Press `Alt+S` or run `/snippets` to choose snippets for the next message.

### Private telemetry

Telemetry uses the private local JavaScript Object Notation (JSON) Lines store at `${PI_CODING_AGENT_DIR:-~/.pi/agent}/telemetry.jsonl`. No prompts, outputs, tool arguments, tool results, file paths, or free-text feedback enter the closed schema.

A run has exactly these fields: `recordType`, `runId`, `parentRunId`, `packageName`, `packageVersion`, `agentName`, `startedAt`, `settledAt`, `durationMs`, `status`, `tokens`, and `costUsd`. The nested `tokens` value has exactly `input`, `output`, `cacheRead`, and `cacheWrite`. The run status is `succeeded`, `failed`, or `cancelled`.

A feedback record has exactly `recordType`, `runId`, `value`, and `createdAt`. The accepted feedback categories are `accepted`, `corrected`, and `rejected`.

Routine status shows only active and failed counts. Use these Pi slash commands:

```text
/telemetry-status
/telemetry-runs {"packageName":"pi-subagents","packageVersion":"0.50.0","agentName":"code-reviewer","status":"failed","minimumDurationMs":1000,"maximumCostUsd":0.25,"feedback":"corrected"}
/telemetry-feedback <runId> <accepted|corrected|rejected>
```

The `/telemetry-runs` command accepts one JSON object. Its approved filters are `packageName`, `packageVersion`, `agentName`, `status`, `minimumDurationMs`, `maximumCostUsd`, and `feedback`.

### Updates and verification

For an upstream update, change its immutable revision and any reviewed adapter paths or installer. If the `pi-subagents` package version changes, also update `PinnedSubagentPackageVersion` in `pi/extensions/telemetry.ts`, its test expectations, and the telemetry filter example above. Then build `tool-sync`, preview the complete plan, and apply it:

```sh
cargo build --release --manifest-path tools/tool-sync/Cargo.toml
REPO_TARGET="$PWD" ./install.sh --dry-run
REPO_TARGET="$PWD" ./install.sh
```

Confirm clean checkouts, exact revisions, managed links, and derived agents. Then run the Cargo and telemetry tests:

```sh
set -eu

while read -r name revision; do
  test "$(git -C "$HOME/.cache/tool-sync/$name" rev-parse HEAD)" = "$revision"
  test -z "$(git -C "$HOME/.cache/tool-sync/$name" status --porcelain)"
done <<'REVISIONS'
pi-subagents 27784eed57dd62021a7add4990ac2dada6690baa
plannotator e1ce7dabe10474b3a653bef9ed5134b73e0b5336
humanlayer-skills 3c2629142c5d437428269b1b722b08c0b87f574d
mattpocock-skills 068b6e0c62393147daf03530149cdce209c93da8
REVISIONS

test "$(readlink "$HOME/.pi/agent/extensions/pi-subagents")" = "$HOME/.cache/tool-sync/pi-subagents"
test "$(readlink "$HOME/.pi/agent/extensions/pi-extension")" = "$HOME/.cache/tool-sync/plannotator/apps/pi-extension"
test "$(readlink "$HOME/.pi/agent/extensions/telemetry.ts")" = "$PWD/pi/extensions/telemetry.ts"
test "$(readlink "$HOME/.agents/skills/pi-subagents")" = "$HOME/.cache/tool-sync/pi-subagents/skills/pi-subagents"
test "$(readlink "$HOME/.agents/skills/show-me")" = "$HOME/.cache/tool-sync/humanlayer-skills/plugins/show-me/skills/show-me"
test "$(readlink "$HOME/.agents/skills/wayfinder")" = "$HOME/.cache/tool-sync/mattpocock-skills/skills/engineering/wayfinder"
test "$(readlink "$HOME/.agents/skills/grilling")" = "$HOME/.cache/tool-sync/mattpocock-skills/skills/productivity/grilling"

for source in agents/*/*.md; do
  test -f "$HOME/.pi/agent/agents/$(basename "$source")"
done
tools/tool-sync/target/release/tool-sync \
  --repository-root "$PWD" --manifest config/tools.toml --home "$HOME" --check >/dev/null

cargo test --manifest-path tools/tool-sync/Cargo.toml
node --test pi/extensions/telemetry.test.ts pi/extensions/telemetry.security.test.ts pi/extensions/telemetry.rpc.test.ts
```

Finally, use Pi Remote Procedure Call (RPC) mode to confirm that all three commands load without extension errors:

```sh
set -eu
rpc_output="$(printf '%s\n' '{"id":"commands","type":"get_commands"}' | pi --mode rpc --no-session 2>&1)"
printf '%s\n' "$rpc_output" | grep -q 'telemetry-status'
printf '%s\n' "$rpc_output" | grep -q 'telemetry-runs'
printf '%s\n' "$rpc_output" | grep -q 'telemetry-feedback'
! printf '%s\n' "$rpc_output" | grep -Eiq 'extension.*error|error.*extension'
```

Do not fork upstream until three reproduced failures share one source-level cause. Configuration, wrappers, or project-owned agents must be unable to fix that cause.

## executable tools

`config/tools.toml` declares executable tools for macOS and Linux. Each entry declares its source, installer, commands, and optional adapters.

`tool-sync` uses embedded sources in this repository without a cache. It stores Git sources under `~/.cache/tool-sync/<name>`.

Each Git entry pins a revision. `tool-sync` fetches a clean cached checkout and checks out that revision with a detached `HEAD`.

The sync refuses a dirty cached checkout before it fetches or changes the revision. It also refuses non-symlink command and Pi-extension destinations.

A Pi tool interface requires a Pi extension. `tool-sync` links each declared extension into `~/.pi/agent/extensions/`.

Preview and apply the complete installation from the repository root:

```sh
cargo build --release --manifest-path tools/tool-sync/Cargo.toml
REPO_TARGET="$PWD" ./install.sh --dry-run
REPO_TARGET="$PWD" ./install.sh
```

The tracked `rag` entry installs the `rag` command from its pinned Git revision. Its Pi extension registers the `search_memory` tool.

See [CONTRIBUTING.md](CONTRIBUTING.md) for every manifest field and the supported authoring path.

## pr review

`pr-review-filter` starts a review pass. It lists the open PRs in two groups: the review inbox, and unclaimed PRs with no review activity.

Run it inside a repository, or pass `--repo owner/name`. `--json` prints the machine form. `--max N` and `--all` set the group size.

`config/pr-review.toml` holds the defaults, and a `[repos."owner/name"]` table overrides them for one repository. `platform = "graphite"` swaps the review links to the Graphite dashboard. Stacked PRs sort bottom first on both platforms.

Run `pr-review-filter set platform=graphite` inside a repository to write its override table. `pr-review-filter show` prints the effective values for that repository.

## how it works

- `~/.agents/skills` is the canonical root: one symlink per skill into this repo.
- Each tool root (`~/.claude/skills`, `~/.codex/skills`) is a single directory symlink
  into it, so a new tool costs one line. `tools/mcp-sync` renders `config/mcp-servers.toml`
  into `~/.claude.json` and `~/.codex/config.toml` on every install and pull.
  `mcp-sync adopt` folds the servers that `claude mcp add` or `codex mcp add` created
  back into the manifest.
- `install.sh` selects Z shell (`zsh`) for Claude Code and Pi. Codex uses the login
  shell, and the shared global guidance requires Z shell syntax in every client.
- Claude Code and the ChatGPT app both write `~/.claude/settings.json`, so nothing in that
  file is authoritative. `install.sh` prunes the marketplace entries and plugin keys the
  ChatGPT import leaves behind, and `config/managed-settings.json` pins the settings that
  must not drift. Preferences stay out of the policy file: a pinned key can no longer be
  changed from the UI, so `model` and `effortLevel` remain the user file's to own.
- Claude Code skips the status line and every hook in a directory whose trust dialog was
  never accepted, and reports nothing when it does. `install.sh` trusts this repo's own
  worktrees under `$HOME`, and never a checkout under `/tmp`.
- Skills log usage to `skills/<name>/logs/` (local, gitignored) and grow their eval
  cases from real use; blind judge votes land the same way.
- The `post-checkout` hook copies uncommitted work into a clean worktree or branch at main's tip.
  It applies tracked changes with a three-way merge, so tip drift produces conflict markers.
  It copies untracked files that Git does not ignore.
  The hook does not touch a dirty destination, an in-progress rebase, or the live tree.

## live diffs

The `live-diff` extension shows what the agent changes as it works.

The statusline badge carries two counters, coloured by the theme. `turn` counts the current turn's changes against a snapshot taken when the turn started. `branch` counts everything since the branch point — the merge-base with the default branch — so committed and uncommitted work sit in one basket and committing does not empty the badge. When no branch point resolves, that side falls back to `all` against the HEAD tree. A clean worktree reads `diff clean`.

The badge refreshes after each write-capable tool call, when the agent settles, and when a filesystem watcher sees a change made outside the agent — an edit in another editor moves it while the agent is idle. The watcher ignores `.git` and anything git ignores, and coalesces bursts into one refresh.

The `/diff` command opens the overlay. Its keys:

```text
j k         move the selection
h l         switch columns: h is the turn, l is the branch
space       open the selected file's diff in a read-only window
enter       open the selected file in the herdr editor tab's nvim
q or esc    close
```

The read-only window shows one file's diff with line numbers:

```text
j k         scroll a line
d u         page down and up
g G         jump to the top and the bottom
h l         previous and next file, wrapping at both ends
enter       open this file in nvim
q or esc    back to the list
```

Run the tests:

```sh
node --test pi/extensions/live-diff/engine.test.ts pi/extensions/live-diff/overlay.test.ts pi/extensions/live-diff/nvim.test.ts pi/extensions/live-diff/watch.test.ts pi/extensions/live-diff.test.ts
```
