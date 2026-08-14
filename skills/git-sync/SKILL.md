---
name: git-sync
description: Use when pending work has to land on main and reach the remote — commit what is dirty, rebase onto whatever another machine already pushed, fast-forward main, push, then prune the branches git proves are merged. Covers "commit, merge to main, and push", "get it all on main", "clean up the stale branches", and "make sure local and origin are synced". Skip when the ask ends in a PR(Pull Request), which create-pr owns.
metadata:
  short-description: Land pending work on main, sync with the remote, prune merged branches
---

# git-sync

JOB: land every pending change on main and push it, deleting only the branches git proves are merged
IN:  a git repo; tree possibly dirty; possibly on a work branch; possibly behind a remote another machine pushed to
OUT: the report in step 8 — one SHA that main and origin/main both hold, the commits created, a verdict per branch, and every refusal named

## hard rules

- **A branch is deletable only when `git branch --merged main` lists it.** Never infer "stale" from a name, an age, or a phrase like "clean up the old branches". A branch holding commits main does not have is work, whatever it is called.
- No AI(Artificial Intelligence) attribution: no `Co-authored-by: Claude` trailer, no "Generated with Claude Code" footer. `attribution.commit` in settings.json suppresses the harness default and a PreToolUse hook blocks the rest; when either is missing on this machine, strip the trailer by hand.
- Never force-push. A rejected push means the remote moved: go back to step 3 and rebase.
- Never `rebase` a branch that is already pushed and shared. Rebase the local work onto the remote, never the reverse.
- Explicit instructions in the user's request beat every default here.

## steps

1. **Establish facts.** Run `git fetch --prune origin`, then `git status -sb`, `git branch -avv`, and `git log --oneline @{u}..HEAD` plus `HEAD..@{u}` where an upstream exists. Done when the current branch, the dirty files, the ahead and behind counts, and every local and remote branch are known.

2. **Commit the tree.** Read every change with `git diff` and inspect untracked files before staging. Split by concern, never by author: one commit per coherent body of work. Route each message through `/byline` and ship only on `ste-check --register byline` exit 0. Done when `git status --short` is clean, or every file left out is deliberate and queued for the report.

3. **Rebase onto the remote.** This is the step that keeps two machines in sync. When `HEAD..@{u}` is non-empty, another clone pushed first: run `git rebase origin/<branch>`. On a conflict, stop and report it — never resolve a conflict on the user's behalf and never `--skip`. Done when the branch sits on top of the remote tip, or the user holds a named conflict.

4. **Land on main.** `git checkout main`, `git merge --ff-only <branch>`. A fast-forward refusal means main holds commits the branch does not: stop, report both SHAs, and ask. Never paper over it with a merge commit. Done when main's tip equals the work branch's tip.

5. **Push.** `git push origin main`. On rejection, return to step 3 — never `--force`. Done when the push prints the `<old>..<new>` range.

6. **Triage branches.** Run both halves and let them decide:
   ```sh
   git branch --merged main | grep -v '^\*'   # deletable
   git branch -a --no-merged main             # refuse, every one
   ```
   Delete the first list with `git branch -d` (never `-D`, which defeats the check). For every branch in the second list, report its commit count and diffstat instead of deleting it. Done when each branch has a verdict and no unmerged branch was touched.

7. **Verify the sync.** `git rev-parse HEAD origin/main` and compare the two SHAs literally. A clean `git status` is not proof. Done when both SHAs are identical, or the difference is reported.

8. **Report.** Emit exactly this shape:
   ```
   synced:    <sha>  (main == origin/main)
   commits:   <sha> <subject>            (one line each, newest first)
   branches:  <name>  deleted | kept: <reason with commit count>
   refused:   <what was not done and why>          (omit when empty)
   left out:  <files not committed and why>        (omit when empty)
   ```
   Done when the report carries the SHA, every commit, a verdict for every branch, and every refusal from steps 3, 4, and 6.

## evals

`evals/run.sh` grades every non-holdout case in `evals/cases.jsonl` against this file, or a candidate via `./run.sh candidate.md`, using `evals/rubric.md`; `--holdout` runs the held-out slice. One JSON(JavaScript Object Notation) line per case to stdout, mean to stderr.

## logging

At the end of a use, append ONE JSON line to this artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"git-sync","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `ts` is the machine's current local timezone with offset (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user analyzes these against their own day.
- The excerpt is the relevant transcript parts only — the trigger, the key outputs, any human correction. Never the full transcript; cap ~2KB per line.
