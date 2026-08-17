---
name: git-sync
description: Use when pending work must reach main and the remote. Commit the tree, rebase onto remote work, squash onto main, push, and prune branches that main contains. Covers "commit, merge to main, and push", "get it all on main", "clean up the stale branches", and "make sure local and origin are synced". Skip when the ask ends in a PR(Pull Request), which create-pr owns.
metadata:
  short-description: Squash pending work onto main, sync with the remote, prune merged branches
---

# git-sync

JOB: Land every pending change on main and push it. Delete only branches that main contains.
IN:  A git repository with possible local changes, work branches, or remote changes.
OUT: The step 8 report. It names the shared commit hash, created commits, branch verdicts, and refusals.

## hard rules

- **A branch is deletable only when `git branch --merged main` lists it.** Never infer "stale" from a name, age, or cleanup request. A branch with commits absent from main is work, whatever its name. A squash does not make its source branch an ancestor of main. Never delete that source branch during the squash run.
- No AI(Artificial Intelligence) attribution: no `Co-authored-by: Claude` trailer and no "Generated with Claude Code" footer. `attribution.commit` in `settings.json` suppresses the harness default. A `PreToolUse` hook blocks other attribution. Either control is sufficient. Never reuse a draft that contains attribution. Write `/byline` output to a new clean message file. Use exactly `git commit -F <file>` with no extra commit flags. Inspect the final commit message. A missing setting alone never stops the run. Stop only when `git log` shows actual attribution.
- Never force-push. A rejected push means the remote moved: go back to step 3 and rebase.
- Never `rebase` a branch that is already pushed and shared. Rebase the local work onto the remote, never the reverse.
- Explicit instructions in the user's request beat every default here.

## steps

1. **Establish facts.** Run `git fetch --prune origin`. Run `git status -sb` and `git branch -avv`. Where an upstream exists, run `git log --oneline @{u}..HEAD` and `git log --oneline HEAD..@{u}`. When clean main matches origin/main, skip steps 2 through 5 and continue with branch triage. Done when the output shows the current branch, local changes, ahead and behind counts, and all branches.

2. **Commit the tree.** Read every change with `git diff` and inspect untracked files before staging. When main is dirty, run `git switch -c "git-sync/$(date +%Y%m%d-%H%M%S)"` once. Immediately record it with `branch=$(git branch --show-current)`. Step 4 can then squash the work. Split work commits by concern, never by author. Route each message through `/byline`. Commit only after `ste-check --register byline` exits 0. These are work commits; step 4 folds the branch into one main commit. Done when `git status --short` is clean or the report explains every omitted file.

3. **Rebase onto the remote.** This step keeps two machines in sync. Skip it when the work branch has no upstream. When `HEAD..@{u}` is non-empty, run `git rebase origin/<branch>`. On a conflict, stop and report it. Never resolve a conflict for the user. Never use `--skip`. Done when the branch sits on the remote tip or the user holds a named conflict.

4. **Land on main, squashed.** Record the work branch. Run `git checkout main`. Fast-forward main with `git merge --ff-only origin/main`. Run `git merge --squash <branch>`. Inspect the staged diff before committing it. Route the one squash message through `/byline`. One branch becomes one commit on main. If pending commits already sit directly on main, stop and ask before rewriting them. A conflict or fast-forward refusal is a human decision. Stop and name the paths or commit hashes. Ask the user to choose. Never create a merge commit. Done when the staged diff contains every work-branch change and the squash commit exists.

5. **Push.** `git push origin main`. On rejection, return to step 3 — never `--force`. Done when the push prints the `<old>..<new>` range.

6. **Triage branches.** Run both commands:
   ```sh
   git branch --merged main | grep -v '^\*'   # deletable
   git branch -a --no-merged main             # refuse, every one
   ```
   Capture both outputs before any deletion. Never write a deletion command from an expected result. Delete only names copied from the first output. For each name in the second output, run `git rev-list --count main..<branch>` and `git diff --shortstat main...<branch>`. A squashed source branch usually remains in the second output. Keep it. Never use `-D`; it defeats the check. Report only deletions that `git branch -d` completed. Done when the report gives each branch a verdict and no command touched an unmerged branch.

7. **Verify the sync.** Run `git rev-parse HEAD origin/main`. Compare the two commit hashes literally. A clean `git status` is not proof. Done when both hashes match or the report names the difference.

8. **Report.** Emit exactly this shape:
   ```text
   synced:   <shared commit hash>
   commits:  <hash and subject>
   branches: <name and verdict>
   refused:  <reason>
   left out: <files and reason>
   ```
   Write commits newest first. Name each kept branch's commit count. Omit `refused` and `left out` when empty. Done when the report contains every result from steps 3, 4, and 6.

## evals

`evals/run.sh` grades each development case in `evals/cases.jsonl` against this file or a candidate. It uses `evals/rubric.md`. `--holdout` runs the held-out slice. It prints one JSON(JavaScript Object Notation) line per case and the mean score.

## logging

At the end of a use, append ONE JSON line to this artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"git-sync","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `ts` uses the machine's current local timezone with offset. Get it with `date +%Y-%m-%dT%H:%M:%S%z`. Never use UTC(Coordinated Universal Time).
- The excerpt contains only the trigger, key outputs, and human corrections. Never include the full transcript. Keep each line under 2KB.
