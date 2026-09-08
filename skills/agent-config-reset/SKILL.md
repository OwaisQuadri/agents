---
name: agent-config-reset
description: Use when the user asks to audit or reset their agent config, when a finished reset needs closing out (learnings fold-back, archive retirement), or on sprawl symptoms. skill roots disagreeing on counts, skill links resolving into unrelated repos, hooks accumulating, forced dispatches on trivial turns, orchestration eating a large share of output tokens, unversioned config edits piling up, or unmanaged Pi configuration growing in any repository. Skip when adding a single skill or making a one-off config edit.
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
- `~/.pi/agent`: settings, agents, extensions, and telemetry
- `~/.agents/skills` (the canonical skills root)
- the versioned agents repository (default: `~/Documents/agents`)
- launchd jobs that drive Pi scripts
- Pi model and extension configuration: entry count, schema validity, dead or duplicate definitions

Each agent reports entry counts, where every symlink resolves, links into other
projects' repos, and dead links.
VERIFY: an independent checker with fresh context re-derives each surface's counts; mismatches become findings, not errors.
Aggregate drift metrics into one report:
- skill-count drift from the canonical root
- hook count and what each one enforces
- orchestration token overhead, if transcripts are available (forced-dispatch hooks once cost 66.4% of output tokens; that is the benchmark for way-too-much)
- artifacts that exist only to serve a hook: flag them for removal
- unversioned config files
- vendored third-party skill repos
- files that bypass the managed Pi configuration roots
- the one-sentence test on every component: if it cannot justify its existence in one sentence, flag it

CAP: 8 agents per pass.
ON FAIL: a surface whose agent does not return is flagged in the report, never skipped silently.
SAVE: docs/audits/YYYY-MM-DD.md, findings ranked, verdict one of healthy / drifting / reset warranted.
HUMAN GATE: show the user the verdict. Audit mode stops here.

## reset: phases 4-8

Phases 4-8 execute sequentially. Human approval gates phase 4 (archive), phase 5
(component list), phase 7 (cutover), and phase 8 (delete). Fan out only in phase 6
(rebuild parallel jobs).

### phase 4: archive

tar every location the reset will touch to ~/archive-YYYYMMDD.tar.gz.
RULE: verify completeness as a separate step: file counts source vs archive must match, then `git ls-files -s` (filter .DS_Store) to confirm symlinks (mode 120000) and the exec bit survived; plain shasum and git hash-object both follow symlinks and both miss the exec bit. Never proceed without a verified archive.
ON MISMATCH: diagnose with a second fresh pass before halting. Compare matching metrics on both sides — tar listings count directory entries and name symlinked dirs without a trailing slash, and files written after tar started (usage logs) are legitimately a minute stale. Only a gap that survives diagnosis halts the run; the first run's 3 mismatches were all counting artifacts.
HUMAN GATE: show the user the verified counts; proceed to phase 5 only on approval.
ON FAIL: if tar fails or a mismatch survives diagnosis, halt and report which surfaces failed to archive; do not proceed to phase 5.

### phase 5: spec

List each surviving component and its one-sentence justification in the human-gate record. Do not create a separate reset specification.
HUMAN GATE: the user approves the component list before any build.

### phase 6: rebuild

PARALLEL JOBS: one per surviving artifact, run at once. The approved component list owns
the job list. This skill does not.
- install.sh: symlinks plus any compiled tool the config needs, a --dry-run mode where every mutation goes through a run() wrapper and every announcement through a plan() wrapper so dry-run and the real run cannot diverge, a pre-write backup phase, and backups NEVER land inside a live skills root — a backup inside one surfaces in the tool catalog as a phantom skill.

RULE: the user hand-edits artifacts between gates; before any fix pass, `git diff` for manual edits and never revert them. Gate feedback arrives as a punch list plus hand edits — both are canon.
RULE: grep every rebuilt artifact for live references to removed components. Check registrations, wiring, and paths that must resolve. Zero live hits before cutover.
VERIFY: an independent checker compares every artifact to the approved spec and to
the standing invariants checklist below, and flags anything off. Verify anything
moved from the archive byte-for-byte with `git ls-files -s` per the phase 4 rule —
the mode field is what distinguishes a link (120000, link text as its blob) from a
copy and carries the exec bit.
RULE: the checker verifies document structure. Recount headings. Do not count grep hits. A prose reference is not a duplicate section.
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
- Fold durable process learnings into this skill. Commit the skill update with the close-out when one is needed.
- The archives are during-reset safeguards, not fixtures: once the live state passes a fresh independent verification, offer deletion of ~/archive-*.tar.gz and every pre-reset backup, and record the deletion in the phase 8 doc. After that, git history in the versioned repo is the only rollback.

## standing invariants

Enforce this checklist on every run, both modes. Phase 6 VERIFY checks every
artifact against it; phase 8 re-checks it before any deletion.

- [ ] audit mode is read-only and modifies nothing
- [ ] no unmanaged Pi extensions
- [ ] Pi settings and extensions resolve from their managed destinations
- [ ] installer backups never land inside a live skills root
- [ ] one canonical skills root (`~/.agents/skills`); each skill resolves from it
- [ ] never point a skill link into another project's repo
- [ ] Pi configuration lives in one versioned place, never scattered across project repositories
- [ ] never vendor a third-party skill repo; re-clone on demand
- [ ] skills re-added on demand, one at a time, each from a single canonical source
- [ ] verify moves with `git ls-files -s`; filter .DS_Store
- [ ] never rm before a verified move; verification is a separate step
- [ ] ask before touching launchd-driven scripts

## evals

evals/ follows skills/ai-author/templates/eval-harness.md: cases.jsonl, rubric.md, run.sh (`./run.sh [candidate]` grades non-holdout cases, `--holdout` the held-out slice). Unseeded by the user's call (2026-07-31); cases grow from real transcript-hit failures (found via gepa-due's own scan) and judge votes, or bootstrap on "seed the evals".
