---
name: hq
description: Use when the user invokes /hq, asks what is happening across his agents, Conductor workspaces, or Mac automations, wants work dispatched into another project, or wants to drill into a named project agent. Presents pending human gates first, then activity since last talk, then acts. Skip for work inside the current repo naming no other project, and for authoring artifacts (ai-author).
metadata:
  minimum-tier: T3
  short-description: "Front door over every project: gates-first digest, isolated dispatch, drill-down"
---

# hq

JOB: front-door digest and dispatch: surface pending human gates and cross-project activity since last talk, then run the user's request, never signing any gate on his behalf
IN:  a /hq invoke or a cross-project ask; state at ~/.claude/hq/ (gates/, activity.jsonl, digest.md, registry.json, heartbeat.log); live rosters at ~/.claude/sessions/ and ~/.claude/jobs/
OUT: a mouthpiece-voice digest (unresolved gates first, each with evidence paths), then exactly one of: a dispatched background worker reported with its target cwd(current working directory), an answer sourced from monitored state with absolute paths cited, or a drill-down (registry record + transcript tail + the direct handle); state advanced

## not the dismissed router

The advisory pipeline struck in docs/reset-spec.md (router / model-selector / prompt-engineer, dismissed 2026-07-30) was hook-forced middleware inserted into every dispatch, once costing 66.4% of output tokens. hq is exempt by the user's explicit gate (2026-08-04): it is deliberately invoked, read-only over other agents' surfaces, never touches another agent's dispatch, prompt, or model choice, and everything it produces terminates at the human.

## hard rules

- gates are never signed on the user's behalf: no plan approval, permission grant, merge, sign-off, or destructive action without his explicit words naming the gate; even then the action runs as its own visible step, and `urgency:"notify_now"` gates escalate via terminal-notifier while everything else waits for the next talk
- every dispatched worker works in a git worktree, never on a target's main or live checkout: in-session workers via the Agent tool's `isolation: "worktree"`; cross-repo dispatch via `git worktree add <repo>/.claude/worktrees/hq-<slug> -b hq/<slug>` (the harness pre-excludes `**/.claude/worktrees/` in every repo, so nothing lands in git status). Dispatching into an existing Conductor workspace satisfies isolation natively
- a target directory without a repo gets `git init` and an initial commit first, local only, with `.claude/` appended to `.git/info/exclude` (a fresh init has no harness exclude block yet, so the worktree dir would land in git status)
- finished work reaches main only through an approved `kind:"merge"` gate carrying the worktree path, branch, and a diff summary; the worktree is cleaned up only after the merge commit is verified on main, never before
- no `git push` and no `gh repo create` unless the user explicitly says so; an explicitly requested repo is created private; public needs its own explicit words. The headless triage never touches remotes at all
- honesty: there is no transport into a running interactive session. hq can read any agent's state and transcript, spawn fresh workers in any repo, and queue context for the user — it cannot speak into a live session. "tell <agent> to X" gets one of: that agent's state read back, a fresh worker offered in its workspace, or the direct handle. Never pretend otherwise
- the user's ask passes to workers verbatim; hq never rewrites prompts and never selects models

## steps

1. first run only (`~/.claude/hq` missing): run `bash ~/.claude/skills/hq/scripts/scan.sh` to create the state tree, seed `watched-jobs.txt`, and write the baseline snapshot. Then present the heartbeat install as a HUMAN GATE — never run it unasked:

   ```sh
   cp /Users/owaisquadri/Documents/agents/skills/hq/launchd/com.owaisquadri.hq.plist ~/Library/LaunchAgents/
   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.owaisquadri.hq.plist
   launchctl kickstart gui/$(id -u)/com.owaisquadri.hq
   ```

   Uninstall is `launchctl bootout gui/$(id -u)/com.owaisquadri.hq`, and before the plist is ever deleted its XML(Extensible Markup Language) is preserved in `docs/audits/<date>-hq.md`. Done when the state dir exists, a baseline snapshot is written, and the gate has been put to the user.
2. refresh: run `bash ~/.claude/skills/hq/scripts/scan.sh`. Done when it exits 0, or its stderr is quoted verbatim in the digest.
3. digest: read unresolved `gates/*.json`, `activity.jsonl` lines newer than `state.json.lastPresentedAt`, `digest.md`, and `tail -5 heartbeat.log` for scan failures. Present gates FIRST, each with its evidence paths, then activity grouped by project; invoke the /mouthpiece skill first and author the digest under it. Advance `lastPresentedAt`. Done when every unresolved gate has been shown and the stamp advanced.
4. act — exactly one verb per request:
   - route first: hq decides whether the ask is in-project development work — a code change inside one repo, ticketable, with a definition of done. If it is, run it through the /engineer skill rather than a bare dispatch, so it walks the 23-phase spine and gets fresh-context testing instead of a builder grading itself. Cross-project status, dispatch into another project, and drill-down stay hq's own. In a repo with no remote, /engineer's closing phase is a local merge of the work branch into main instead of a pull request — still behind an approved `kind:"merge"` gate, never signed by hq.
   - dispatch: every design or shaping choice belongs to the user, greenfield or not — language, library, engine, architecture, what v1 includes and excludes, defaults he will live with, anything hq would otherwise settle by taste. Put them to him with AskUserQuestion BEFORE spawning anything and carry his answers into the prompt verbatim. The only exceptions are choices he has explicitly delegated ("your call", "pick whatever", a standing preference in CLAUDE.md) and mechanical facts with one correct answer. Shipping a guess is the expensive failure, not the question. Then spawn a background worker whose prompt names the absolute target path from `registry.json`, under the isolation rules above. Report what was spawned, where, and its checkpoint cadence so a planned hop-in is possible. A worker hitting something gate-shaped gets it written to `gates/` with `source:"hq-session"`.
   - answer: from snapshots, registry, activity, and transcripts, always citing absolute paths.
   - drill down: resolve the name against `registry.json` (session names like `machu-picchu-74`, or repo/workspace fields), show the record, `tail -40` its transcript, summarize, and end with the direct handle — the Conductor workspace for a live session, `claude --resume <sessionId>` run from that cwd for a dead one.
   - usher: when the user says he wants to hop in on running work, hq is the usher between them — report the far side in detail, not a summary: what the worker has committed and what is still uncommitted, what it is doing right now, which files it holds open, what it has decided and what it is still deciding, and where it will pause next. Then relay his words to it verbatim and its reply back. Never imply he can attach to a session that does not exist.
   Done when the verb ran and its report names its paths.
5. gate resolution: only on the user's explicit words naming the gate — set `isResolved`, `resolvedAt`, `resolution`, then mv the file into `gates/resolved/` and confirm it exists there before anything else happens. Done when the gate sits in `gates/resolved/` and the approved action (if any) ran as its own step.
6. mid-session: at the start of each user turn in an hq conversation, stat `delta.json` and `gates/`; if either is newer than `lastPresentedAt`, lead the reply with a one-line update. This is recipe behavior — never a hook.

## evals

`evals/run.sh` syntax-checks both scripts, greps this file for its required sections, and runs every non-holdout case in `evals/cases.jsonl` through `scripts/scan.sh --classify` (or a candidate via `./run.sh candidate.sh`), graded per `evals/rubric.md`; `--holdout` runs the held-out slice. One JSON(JavaScript Object Notation) line per case to stdout, mean to stderr; mechanical ceiling 6/10, `kind:"merge"` discipline and digest voice are judge-graded.

## logging

At the end of a use, append ONE JSON line to this artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"hq","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md' ':(exclude)**/logs/**' ':(exclude)**/votes/**'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
- `ts` is the machine's current local timezone with offset (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user analyzes these against his own day.
- The excerpt is the relevant transcript parts only: the trigger, the key outputs, any human correction. Never the full transcript; cap ~2KB per line.
