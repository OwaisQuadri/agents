# git-sync tuning

The GEPA(Genetic-Pareto prompt evolution) record: mutations, deferred verdicts, the open
list. History lives here and never in SKILL.md, which every run loads.

## history

- **2026-08-14 — authored.** Routed through ai-author. `create-pr` owns commit-push-PR and
  its own skip clause excludes "only a commit or a push with no PR", so this is a sibling
  job rather than an update to it. Recurrence verified from transcripts, not asserted:
  three occurrences in under 24 hours across the waybar, hypr, and rag repos.

## why the hard rules exist

Each one answers an observed failure, not an imagined one. Narrowing a rule needs a
logged false positive; widening needs only that no rule paid for itself.

- **`git branch --merged` as the only delete gate.** On 2026-08-14 in the rag repo, "cleanup
  the stale branches" would have deleted `origin/map/RAG-0001` — 43 commits, 12,677
  insertions, a Rust rewrite last touched the previous day. The gate is what caught it.
- **No attribution trailer.** On the same pass, `Co-Authored-By: Claude Opus 5` reached
  `dad21ec` and `f1cea61` and was pushed, against create-pr's standing hard rule. The
  deterministic fix is `attribution.commit: ""` in settings.json plus the
  `block-commit-attribution.sh` PreToolUse hook; the rule here is the third layer.
- **Rebase before landing.** The user runs clones on an Arch desktop and a Mac. Either can
  push first, so a land that skips the rebase either fails on push or invites a force.
- **Push main only on an explicit ask.** Two sessions twelve hours apart reached opposite
  answers on this: the hypr pass refused to push main on principle, the rag pass pushed it
  on request. Nothing pinned the rule, so it was re-decided each time. Step 5 pins it.

## open list

Standing input to the next GEPA pass. Read this before proposing any mutation.

- The report shape in step 8 is untested against a repo with more than one remote. No case
  covers `upstream` plus `origin`.
- No case covers a repo whose default branch is not named `main`. The steps hardcode the
  name. Widen only if a log line shows a `master` or `trunk` repo reaching this skill.
- `evals/run.sh` grades a plan rather than a live run. A plan that cites the right command
  and a run that executes it are not the same evidence. A sandboxed fixture repo would be
  stronger, and costs a temp clone per case.
