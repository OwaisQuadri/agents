# agents

Skills, subagents, workflows, and config for my coding agents (Claude Code, Codex),
versioned in one repo with a symlink installer.

## install

```sh
./install.sh --dry-run   # print the plan
./install.sh             # symlink for real; pre-write backups land outside the skills root
```

## layout

| path | holds |
| --- | --- |
| `skills/` | one `SKILL.md` per skill, loaded on trigger |
| `agents/` | subagent definitions, each with its own tools and model |
| `workflows/` | multi-agent graph specs |
| `config/` | `mcp-servers.toml`, the tracked MCP (Model Context Protocol) server manifest and the single edit surface for servers; `mcp-sync-state.toml`, the machine-written, untracked sync state; tracked copies of `settings.json` / `settings.local.json`, which stay reference-only: never installed, never symlinked, never written by the installer |
| `docs/` | prose style (the ASD-STE100 rules every register runs on), code style, comment style, docstring style (the standard generator per language), the executed reset spec, fleet research |
| `tools/` | `ste-check`, the zero-dependency Rust binary that grades prose against the STE rules plus whatever the register adds; `mcp-sync`, the Rust binary that renders the MCP server manifest into each tool's live config |
| `hooks/` | both git hooks and Claude Code hooks. `post-checkout` carries the live checkout's uncommitted work into worktrees and branches cut at main's tip, `test.sh` is its regression suite; `rag-recall` is the UserPromptSubmit hook that searches the personal RAG store on every prompt, registered for both Claude Code and Codex |
| `.conductor/` | repo settings for Conductor; its setup script runs `hooks/post-checkout` in every new workspace |
| `install.sh` | symlink installer; it builds `ste-check` and `mcp-sync` with cargo, then runs `mcp-sync` to render the manifest into the live configs; `--dry-run` prints every mutation through the real code path |
| `CLAUDE.md` | global guidance loaded every session; the single instructions source for both tools. `install.sh` links `~/.codex/AGENTS.md` to it, so Codex reads the same file |

### skills

| skill | job |
| --- | --- |
| `ai-author` | decides whether a skill/agent/workflow should exist at all; owns the eval + logging contract and GEPA (Genetic-Pareto prompt evolution) tuning |
| `skill-author` / `agent-author` / `workflow-author` | deep authoring craft per artifact type |
| `agent-config-reset` | audit a sprawling agent setup and, gated on approval, reset it |
| `create-pr` | commit, push, and open the pull request |
| `engineer` | the exact 23-phase map for agent coding work: ticket to PR, every loop through one walk-back rule |
| `hq` | front door over every project: gates-first digest, worktree-isolated dispatch, drill-down into any project agent; two-stage launchd heartbeat keeps quiet cycles at zero tokens |
| `bro` | re-explains the last reply in plain words when it lost you; jargon goes, facts stay verbatim, no length cap |
| `byline` | de-slops prose that ships under your name: commits, PR bodies, tickets, READMEs; facts stay verbatim |
| `mouthpiece` | voice rules for end-user-facing replies, scored by `ste-check --register mouthpiece` |
| `task-graph` | work items + deps → statused DAG or ABCD-NNNN tickets, rendered in mermaid |
| `vocabulary` | precise design and UI terms: exact lookup, near-synonym boundaries, reverse lookup from a vague ramble |
| `volley` | short-turn mode: every turn ends inside 30 seconds, anything longer is dispatched to the background and reported on the next turn |

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
| `research-sweep` | answer one research question: fan out researchers over distinct angles, gap-check with an independent critic, fill what's missing |

## how it works

- `~/.agents/skills` is the canonical root: one symlink per skill into this repo.
- Each tool root (`~/.claude/skills`, `~/.codex/skills`) is a single directory symlink
  into it, so a new tool costs one line. `tools/mcp-sync` renders `config/mcp-servers.toml`
  into `~/.claude.json` and `~/.codex/config.toml` on every install and pull.
  `mcp-sync adopt` folds the servers that `claude mcp add` or `codex mcp add` created
  back into the manifest.
- Skills log usage to `skills/<name>/logs/` (local, gitignored) and grow their eval
  cases from real use; blind judge votes land the same way.
- A worktree or branch checked out clean at main's tip inherits the live checkout's
  uncommitted work via the `post-checkout` hook: tracked changes (staged, unstaged,
  deletions) arrive as a 3-way apply so tip drift surfaces as conflict markers, untracked
  non-ignored files arrive by copy, and a dirty destination or in-flight rebase is never
  touched — nor is the live tree.
