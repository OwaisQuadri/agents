# agents

The versioned home of my agentic workflows: the skills, config, and installer that drive
my coding agents (Claude Code, Codex), kept in one repo to version, share, and reuse.

## layout

- `skills/` — one directory per skill, each a `SKILL.md` the agent loads on trigger:
  - `agent-config-reset` — audit a sprawling agent setup and, gated on approval, reset it
  - `ai-author` — decides whether a skill/agent/workflow should exist at all; owns the
    eval + logging contract and the GEPA (Genetic-Pareto prompt evolution) tuning loop
  - `skill-author` / `agent-author` / `workflow-author` — deep authoring craft per type
  - `mouthpiece` — voice rules for end-user-facing replies, scored by `eval/check.py`
    (the personal voice examples live in a gitignored file; the skill asks for them
    if missing)
  - `vocabulary` — precise design and UI terminology
- `config/` — tracked copies of `settings.json` / `settings.local.json`; the live files
  are never symlinked and never written by the installer
- `install.sh` — symlink-only installer; `--dry-run` prints every mutation through the
  same code path as the real run
- `docs/` — the executed reset spec; audit records stay local (gitignored)
- `CLAUDE.md` — global guidance loaded every session

## how it works

One canonical skills root at `~/.agents/skills` holds a symlink per skill into this
repo. Each tool root (`~/.claude/skills`, `~/.codex/skills`) is a single directory
symlink into it, so a new tool costs one line.

Skills log their own usage to `skills/<name>/logs/` (local, gitignored) and grow their
eval cases from real use; blind judge votes land the same way.

## install

```sh
./install.sh --dry-run   # print the plan
./install.sh             # symlink for real; pre-write backups land outside the skills root
```
