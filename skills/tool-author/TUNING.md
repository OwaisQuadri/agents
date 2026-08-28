# tool-author tuning

The GEPA(Genetic-Pareto prompt evolution) record: mutations, deferred verdicts, the open
list. History lives here and never in SKILL.md, which every run loads.

## history

- **2026-08-28 — authored.** Gap found while building `tools/logpath-check` and the
  associated runtime guard: `ai-author`'s own "question one" routes decidable work to "a
  checker" or "a pi extension" but had no sibling craft skill for either, unlike
  skill/agent/workflow. Scoped to four branches after a follow-up audit found CLI hooks
  (`config/*.json`'s `"hooks"` key — proven load-bearing by `tools/no-ai-attribution`
  already serving as the `PreToolUse` hook) and git hooks (`hooks/`, symlinked via
  `install.sh`) are real, distinct "runtime fires it" backends this repo already uses,
  with git hooks explicitly named in AGENTS.md's own Rust-language carve-out. Folded into
  one skill rather than split per branch: all four share the same craft shell (minimal
  deps, tests over a GEPA harness, no README) and splitting would have duplicated section
  4 four ways for a distinction that is really just "which event source."

## open list

- No usage lines yet — this skill has not been dispatched against a real authoring task.
  The eval cases are seed-only (5 seed, 1 log-adjacent, 1 holdout); grow from real usage
  once `tools/logpath-check`'s own runtime guard gets built through this skill, which
  will be the first real dispatch.
