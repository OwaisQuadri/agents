<!-- TODO(AGNT-0032.T51): record only evidence-backed git-sync mutations -->
# git-sync tuning

The GEPA(Genetic-Pareto prompt evolution) record: mutations, deferred verdicts, the open
list. History lives here and never in SKILL.md, which every run loads.

## history

- **2026-08-14 — squash by default, one gate, two proofs of merged.** Three mutations in one
  pass, from the owner at the RAG-0001 GATE E. Two are directives and one is a defect fix, and
  the acceptance evidence differs for each, so each is labelled.

  **Squash replaces fast-forward in step 4. OWNER DIRECTIVE, no harness claim.** His words were
  "git-sync should squash brother, as always", and the standing preference dates to 2026-08-07.
  Step 4 also gained a hand-off route: when the caller already made the commit on main, the step
  is skipped and the report says so, which is what lets `/engineer` phase 23 squash for itself
  and hand the sync and the triage here.

  **The happy path lost its gate, and a fresh reviewer took its place. OWNER DIRECTIVE.** "only
  when theres a conflcit that requires a decision do i want a HUMAN gate. otherwise you can
  deploy a fresh reviewer to see if its implemented and integrated smoothly with the origin main
  branch." A conflict in the rebase or the squash still stops the run. Nothing else does. The
  reviewer runs on EVERY clean landing rather than a doubtful one, because a check that fires
  only when someone is already suspicious is not a replacement for a gate.

  **Step 8 takes two proofs of merged. DEFECT FIX, accepted on a reproduction, not on a mean.**
  Squash breaks the ancestry check that step 8 relied on. Reproduced in a scratch repo: after
  squashing a two-commit branch, `git branch --merged main` returns EMPTY, `git diff main work`
  returns 0 lines, and `git branch -d work` refuses with "not fully merged". With squash now the
  default, the old rule would have called every branch this skill just landed unmerged work,
  permanently. `/engineer` phase 23 already documents the same blindness against its own phase
  01 sweep, so this is the second consumer it has bitten. An empty tree diff is the stronger of
  the two proofs, because it reads content rather than the shape of history.

  **The harness could not measure any of this, and the reason is worth recording.** `logs/` and
  `votes/` were both EMPTY at the time of the pass, so GEPA step 1 had no reflective dataset and
  no failure histogram was built. There was nothing to reflect on. A fenced case author was
  dispatched in the same pass and wrote 7 cases from the requirements and the reproduction,
  never seeing the mutated text. It returned two harness conflicts the mutation created:
  `rubric.md` scored the corrected deletion behaviour as CATASTROPHIC, and `g1` and `g5` still
  encoded the fast-forward expectation, so a correct skill would have failed them by
  construction. Both were repaired by that same fenced author rather than by the proposer.

- **2026-08-14 — authored.** Routed through ai-author. `create-pr` owns commit-push-PR and
  its own skip clause excludes "only a commit or a push with no PR", so this is a sibling
  job rather than an update to it. Recurrence verified from transcripts, not asserted:
  three occurrences in under 24 hours across the waybar, hypr, and rag repos.

## why the hard rules exist

Each one answers an observed failure, not an imagined one. Narrowing a rule needs a
logged false positive; widening needs only that no rule paid for itself.

- **A proof of merged before any delete.** On 2026-08-14 in the rag repo, "cleanup the stale
  branches" would have deleted `origin/map/RAG-0001` — 43 commits, 12,677 insertions, a Rust
  rewrite last touched the previous day. The gate is what caught it. It was `--merged` alone
  until later the same day, when squash became the default and the reproduction above showed
  `--merged` cannot see a squashed branch. The rule now takes an empty `git diff main <branch>`
  as the second proof. That WIDENS what counts as merged and narrows nothing, so it does not
  weaken the case above: `origin/map/RAG-0001` fails both proofs, not just one.
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

- The report shape in step 10 is untested against a repo with more than one remote. No case
  covers `upstream` plus `origin`.
- **Nothing here has ever been measured.** `logs/usage.jsonl` and `votes/votes.jsonl` are both
  empty, so no mutation to this skill has an eval number behind it and no blind judge has ever
  graded a use. The 2026-08-14 mutations rest on an owner directive and a reproduction. The
  first real use should log, and the first log should be judged, before any mutation claims a
  win on a mean.
- The squash hand-off route in step 4 has no live exercise. `/engineer` phase 23 is the only
  caller that would use it and it has not run against this skill yet, so the "skip the commit
  and say so" path is written and unproven.
- Step 7 dispatches a reviewer on every landing, and nothing bounds what that costs on a large
  diff. No log line yet shows the reviewer catching anything a landing would otherwise have
  shipped, which is the number that would justify it.
- **The reviewer runs AFTER the push, so it cannot stop a bad landing — only describe one.**
  Step 6 pushes and step 7 reviews. That order follows the owner's wording, "integrated smoothly
  with the origin main branch", which reads as a review of what reached the remote. It has a
  cost he should decide on rather than inherit: force-push is banned skill-wide, so anything the
  reviewer finds is fixed forward with another commit and never retracted. Reviewing local main
  against `origin/main` BEFORE the push would answer the same question and could still stop the
  push, without adding a gate, since a Critical would go to the user exactly as it does now. The
  open question is whether that counts as the gate he asked not to have. Not changed unilaterally
  because it alters the step order rather than the harness.
- No case covers a repo whose default branch is not named `main`. The steps hardcode the
  name. Widen only if a log line shows a `master` or `trunk` repo reaching this skill.
- `evals/run.sh` grades a plan rather than a live run. A plan that cites the right command
  and a run that executes it are not the same evidence. A sandboxed fixture repo would be
  stronger, and costs a temp clone per case.

## recovered from the live clone, 2026-08-18

Two observations from the pre-rewrite working copy, salvaged when the PR flow landed:

- **`git branch --merged` as the only delete gate.** On 2026-08-14 in the rag repo, "cleanup
  the stale branches" would have deleted `origin/map/RAG-0001` — 43 commits, 12,677
  insertions, a Rust rewrite last touched the previous day. The gate is what caught it.
- The report shape is untested against a repo with more than one remote. No case covers
  `upstream` plus `origin`.
