---
name: create-pr
description: Use when the user asks for a PR(Pull Request) from the current branch or a Conductor PR-instructions attachment arrives. commit pending changes, push, open the PR with gh. Skip when the ask is only a commit or a push with no PR, and for reviewing an existing PR.
metadata:
  short-description: Commit, push, and open the PR with gh
---

# create-pr

JOB: turn the branch's pending work into an open PR: commit, push, gh pr create
IN:  a PR request, usually a Conductor PR-instructions attachment naming a target branch; a git repo on a work branch, tree possibly dirty
OUT: an open PR URL reported with commit SHAs plus every surprise (branch-name mismatch, files left uncommitted); zero attribution lines

## hard rules

- no AI(Artificial Intelligence) attribution lines: no `Co-authored-by: Claude ...` trailer in commit messages, no "Generated with ..." footer in PR bodies. Harness defaults inject both, so the verify sub-steps below have the final say; only an explicit user ask naming an attribution line overrides this.
- never rename the branch. `git status` is the truth about the branch name; when an instructions attachment disagrees, proceed on the real branch and name the mismatch in the report
- never force-push. A rejected push means the remote diverged: stop and ask
- explicit instructions in the user's request beat every default below

## steps

1. establish facts: `git status`, `git branch -vv`, `git log <target>..HEAD --oneline`, `gh pr list --head <branch>`. The target branch comes from the request, default origin/main; confirm it exists on the remote before it becomes `--base`. Done when branch, target, ahead count, dirty files, and any existing open PR are known.
2. dirty tree: review every change with `git diff` plus untracked files, then commit the reviewed files with a message describing the change. Verify `git log -1 --format=%B` shows no attribution trailer, else `git commit --amend`. Done when `git status` is clean (or every file left out is deliberate and queued for the report) and the verify passed.
3. clean tree and zero commits ahead: stop and ask, there is no diff to open. Done when work exists or the user has been asked.
4. push: `git push -u origin HEAD`, or to the existing upstream when it tracks a different name or remote. Done when the push succeeds.
5. review the full PR diff: `mcp__conductor__GetWorkspaceDiff` inside Conductor, else `git diff <target>...HEAD`. Done when every changed file in the diff is accounted for in the description draft, not just this session's edits.
6. create: `gh pr create --base <target>`, title under 80 characters, body under five sentences saying what changed and why, unless instructed otherwise. When step 1 found an open PR, update it with `gh pr edit` and say so instead. Verify `gh pr view --json body` shows no attribution footer, else fix via `gh pr edit --body`. Done when gh prints the PR URL and the verify passed.
7. report: PR URL, commit SHAs, both verify results, and the surprises queued in steps 1-3. A failed step ships its output and a question for the user, never a claimed success. Done when the report carries all four.

## evals

`evals/run.sh` grades every non-holdout case in `evals/cases.jsonl` against this file, or a candidate via `./run.sh candidate.md`, using `evals/rubric.md`; `--holdout` runs the held-out slice. One JSON(JavaScript Object Notation) line per case to stdout, mean to stderr.

## logging

At the end of a use, append ONE JSON line to this artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"create-pr","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
- `ts` is the machine's current local timezone with offset (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user analyzes these against their own day.
- The excerpt is the relevant transcript parts only: the trigger, the key outputs, any human correction. Never the full transcript; cap ~2KB per line.
