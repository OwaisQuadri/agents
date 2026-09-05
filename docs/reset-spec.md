# reset spec — 2026-07-31 (executed; final record)

Locked at the phase 5 gate, amended at every later gate, executed the same night.
Anything not on this page is not in the new setup; the next audit diffs against this page.

## shipped

- skills/, exactly seven: agent-config-reset, ai-author, skill-author, agent-author,
  workflow-author, mouthpiece, vocabulary. mouthpiece is the end-user-voice skill, body
  capped at 500 characters excluding exact information (code, command output, data) and
  requested formatting; the old check.py lives on as its eval.
- agents/: empty at cutover (a .gitkeep holds the surface); the fleet rewrite is
  ai-author's first project, research first.
- lean CLAUDE.md: working style, the abbreviation regulation, agent-communication
  guidance, the /mouthpiece pointer for user-facing replies, never-rm-before-a-verified-move,
  time-estimation guidance. Hand-edited by owais at the phase 6 gate; those edits are canon.
- install.sh: symlinks only, run()/plan() --dry-run wrappers, pre-write backups landing
  outside any live skills root, never touches settings.json.
- one canonical skills root ~/.agents/skills of per-skill links into ~/Documents/agents;
  ~/.claude/skills and ~/.codex/skills are each ONE directory symlink into it.
- tracked copies of settings.json and settings.local.json, hook registrations scrubbed;
  the installer never touches the live ones.
- rule: no component may wire in a hook or resolve a path into ~/swarm or ~/swarm-skills;
  naming them as history or as audit targets is exempt.

## died

- the 5 hooks and their settings.json registrations — the forced-dispatch regime that
  once cost 66.4% of output tokens
- the ~13-agent fleet in "~/agent loop", including the dismissed
  router / model-selector / prompt-engineer advisory-pipeline definitions
- the vendored gstack checkout, every drifted skill symlink across the three roots, and
  the hook-only ~/swarm scripts
- skills/skillify: built at phase 6, deleted at the gate rework; ai-author plus the three
  author skills supersede it, its planned GEPA(Genetic-Pareto prompt evolution) round cancelled
- GLOSSARY.md (including this repo's): replaced by the CLAUDE.md abbreviation regulation
- teacher-observer: launchd job booted out, plist preserved in docs/audits/2026-07-31-phase8.md
- widened at the phase 8 gate by owais: ~/.gstack (its keep decision reversed),
  ~/swarm-skills, and every .pre-reset-20260731 backup — "fresh-fresh"
- retired after the reset, 2026-07-31: ~/archive-20260730.tar.gz and
  ~/archive-20260731.tar.gz. During-reset safeguards, deleted at owais's order once an
  independent fresh-context verification passed the live state clean. Rollback from here
  is git history at ~/Documents/agents.

## decisions locked (owais, 2026-07-31)

1. durable checkout for symlink targets: ~/Documents/agents, local merges only, no push
2. mouthpiece ports as the capped skill above; all agents rewritten fresh later, research first
3. teacher-observer: kill it
4. phase 8 widened at the gate: gstack, swarm-skills, and all backups deleted
5. evals stay unseeded on purpose; they grow from natural usage logs and judge votes, or
   bootstrap on "seed the evals"
