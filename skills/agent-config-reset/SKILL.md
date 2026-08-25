---
name: agent-config-reset
description: Use when the user asks to audit or reset their agent config, when a finished reset needs closing out (learnings fold-back, archive retirement), or on sprawl symptoms. skill roots disagreeing on counts, skill links resolving into unrelated repos, hooks accumulating, forced dispatches on trivial turns, orchestration eating a large share of output tokens, unversioned config edits piling up, a committed .claude/ dir growing in any repo. Skip when adding a single skill or making a one-off config edit.
metadata:
  minimum-tier: T4
---

# agent-config-reset

Two modes, each written as a graph spec below.

- audit (default): phases 1-3. Read-only, cheap. Run on any sprawl symptom or every few months. Stops at the human verdict.
- reset: phases 4-8, gated. Only on the user's explicit ask, or after an audit verdict the user approves.

## audit: phases 1-3

Inventory and measure every agent-config surface.
FAN OUT: one agent per surface, all parallel:
- ~/.claude: CLAUDE.md, settings.json, settings.local.json, hooks/, skills/, agents/
- ~/.codex
- ~/.agents/skills (the canonical skills root)
- the versioned agents repo (default: ~/Documents/agents)
- repo-committed .claude/ dirs across the user's projects
- launchd jobs that drive agent scripts
- MCP config (mcpServers in ~/.claude.json, project .mcp.json files, ~/.codex/config.toml): entry count, schema validity, dead or duplicate server definitions

Each agent reports entry counts, where every symlink resolves, links into other
projects' repos, and dead links.
VERIFY: an independent checker with fresh context re-derives each surface's counts; mismatches become findings, not errors.
Aggregate drift metrics into one report:
- skill-count drift across roots vs the canonical root (the old setup drifted to 128/161/110 across ~/.claude, ~/.codex, ~/.agents)
- hook count and what each one enforces
- orchestration token overhead, if transcripts are available (forced-dispatch hooks once cost 66.4% of output tokens; that is the benchmark for way-too-much)
- advisory-pipeline artifacts (the old router / model-selector / prompt-engineer defs) and hook-only scripts (the old skill-usage-log.sh, playbook-gate.sh, skill-usage-sweep.sh): flag if anything exists only to serve a hook; it does not survive a reset
- unversioned config files
- vendored third-party skill repos
- files tracked under any .claude/ dir (one repo hit 1244; that is the anti-pattern)
- the one-sentence test on every component: if it cannot justify its existence in one sentence, flag it

CAP: 8 agents per pass.
ON FAIL: a surface whose agent does not return is flagged in the report, never skipped silently.
SAVE: docs/audits/YYYY-MM-DD.md, findings ranked, verdict one of healthy / drifting / reset warranted.
HUMAN GATE: show the user the verdict. Audit mode stops here.

## reset: phases 4-8

Phases 4-8 execute sequentially. Human approval gates phase 4 (archive), phase 5
(spec), phase 7 (cutover), and phase 8 (delete). Fan out only in phase 6 (rebuild
parallel jobs).

### phase 4: archive

tar every location the reset will touch to ~/archive-YYYYMMDD.tar.gz.
RULE: verify completeness as a separate step: file counts source vs archive must match, then `git ls-files -s` (filter .DS_Store) to confirm symlinks (mode 120000) and the exec bit survived; plain shasum and git hash-object both follow symlinks and both miss the exec bit. Never proceed without a verified archive.
ON MISMATCH: diagnose with a second fresh pass before halting. Compare matching metrics on both sides — tar listings count directory entries and name symlinked dirs without a trailing slash, and files written after tar started (usage logs) are legitimately a minute stale. Only a gap that survives diagnosis halts the run; the first run's 3 mismatches were all counting artifacts.
HUMAN GATE: show the user the verified counts; proceed to phase 5 only on approval.
ON FAIL: if tar fails or a mismatch survives diagnosis, halt and report which surfaces failed to archive; do not proceed to phase 5.

### phase 5: spec

Build a half-page spec at docs/reset-spec.md listing what survives conceptually (each item: a one-sentence justification), the abbreviation regulation included (expand every abbreviation, shortform, acronym, or pseudonym at first use; never introduce one without its inline expansion; never guess an unresolved one). Standing decisions carry forward unless the user re-litigates them.
HUMAN GATE: the user approves the spec before any build. Commit the spec at this gate, and commit each later gate's answers into it as they land — the spec is the record every following phase, and the next audit, diffs against.

### phase 6: rebuild

PARALLEL JOBS: one per surviving artifact, run at once. The approved spec owns the job
list; this skill does not. Durable constraints on two recurring artifacts:
- install.sh: symlinks plus any compiled tool the config needs, a --dry-run mode where every mutation goes through a run() wrapper and every announcement through a plan() wrapper so dry-run and the real run cannot diverge, a pre-write backup phase, and backups NEVER land inside a live skills root — a backup inside one surfaces in the tool catalog as a phantom skill.
- tracked copies at config/settings.json and config/settings.local.json (the installer never touches the live ones).

RULE: the user hand-edits artifacts between gates; before any fix pass, `git diff` for manual edits and never revert them. Gate feedback arrives as a punch list plus hand edits — both are canon.
RULE: grep every rebuilt artifact, the tracked settings copies above all, for LIVE references to anything on the spec's dies list — registrations, wiring, paths that must resolve; historical mentions in records and in this skill's own text are exempt. The first run shipped stale hook and swarm references in settings.json and needed a rework pass. Zero live hits before cutover.
VERIFY: an independent checker compares every artifact to the approved spec and to
the standing invariants checklist below, and flags anything off. Verify anything
moved from the archive byte-for-byte with `git ls-files -s` per the phase 4 rule —
the mode field is what distinguishes a link (120000, link text as its blob) from a
copy and carries the exec bit.
RULE: the checker verifies document structure structurally — recount headings, don't count grep hits; prose that names a section is not a duplicate section. The first run burned a fix cycle on exactly this false alarm.
ON FAIL: rebuild or drop a flagged artifact; do not proceed to cutover while any artifact fails verification.

### phase 7: cutover

Run install.sh --dry-run.
HUMAN GATE: show the user the dry-run output and wait for approval. Then, only on approval, run for real.
Then verify installation complete FROM A FRESH SESSION: every link resolves AND destination file counts match expected values; this is the separate verification step phase 8 requires before any delete. The running session's skill catalog is stale until it reloads — tell the user to open a fresh session after cutover.
ON FAIL: if the dry-run shows wrong paths or the real run fails, halt before phase 8; do not delete until the real run completes and link + count verification passes.

### phase 8: delete + close-out

Only after phase 7 is verified.
RULE: never rm before a verified move; verify destination counts as a separate step before any delete (an rm chained after a silently-failed mv once cost 40 files).
RULE: ask before touching anything driven by launchd. Preserve a deleted launchd plist's XML in the phase 8 record for reconstruction.
RULE: keep ledgers unless the user explicitly puts them on the delete list — a keep decision is the user's to reverse at this gate, as ~/.gstack's was.
Re-check the standing invariants checklist against everything still in place.
HUMAN GATE: list exactly what will be deleted. The user may WIDEN the list here (backups, ledgers, previously spared dirs); restate the widened list verbatim and act only on the final approved list.
ON FAIL: if any destination count does not match, delete nothing and halt.

Close-out, after the deletes:
- SAVE: docs/audits/YYYY-MM-DD-phase8.md — what was deleted, the verification evidence, any preserved plists.
- Fold the run's learnings into THIS skill and mark docs/reset-spec.md executed with what actually shipped and what widened at the gates. Not done until this skill's own diff is in the closing commit — the first run recorded learnings in the audit doc and left the skill untouched, which is exactly the miss.
- The archives are during-reset safeguards, not fixtures: once the live state passes a fresh independent verification, offer deletion of ~/archive-*.tar.gz and every pre-reset backup, and record the deletion in the phase 8 doc. After that, git history in the versioned repo is the only rollback.

## standing invariants

Enforce this checklist on every run, both modes. Phase 6 VERIFY checks every
artifact against it; phase 8 re-checks it before any deletion.

- [ ] audit mode is read-only and modifies nothing
- [ ] no hooks, of any kind
- [ ] never symlink settings.json or settings.local.json: Claude Code writes them via write-temp-then-rename, which replaces the symlink with a regular file and silently orphans the repo copy; keep a tracked copy for drift visibility
- [ ] the installer only symlinks; it never writes settings.json
- [ ] installer backups never land inside a live skills root
- [ ] one canonical skills root (~/.agents/skills); every other tool gets a thin link into it, meaning a single directory symlink (e.g. `ln -s ~/.agents/skills ~/.claude/skills`), never per-file links; a new tool costs one line
- [ ] never point a skill link into another project's repo
- [ ] never commit a .claude/ directory to a project repo; agent config lives in one versioned place, never scattered across project repos
- [ ] never vendor a third-party skill repo; re-clone on demand
- [ ] skills re-added on demand, one at a time, each from a single canonical source
- [ ] verify moves with `git ls-files -s`; filter .DS_Store
- [ ] never rm before a verified move; verification is a separate step
- [ ] ask before touching launchd-driven scripts

## evals

evals/ follows skills/ai-author/templates/eval-harness.md: cases.jsonl, rubric.md, run.sh (`./run.sh [candidate]` grades non-holdout cases, `--holdout` the held-out slice). Unseeded by the user's call (2026-07-31); cases grow from logs/usage.jsonl and judge votes, or bootstrap on "seed the evals".

## logging

At the end of a use, append ONE bounded JSON (JavaScript Object Notation) line to this
skill's `logs/usage.jsonl`, in exactly this shape and with exactly these keys:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"agent-config-reset","prompt_version":"<short sha>","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md' ':(exclude)**/logs/**' ':(exclude)**/votes/**'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
- `ts` is the machine's CURRENT LOCAL TIMEZONE with offset
  (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC (Coordinated Universal Time): the user
  analyzes these against their own day.
- The excerpt is the relevant transcript parts only; cap ~2KB per line.
