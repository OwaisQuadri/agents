---
name: git-sync
description: Use when pending work must reach main and the remote, or when merged branches need cleanup. Put all work through a PR(Pull Request), then prune only branches that main contains. Covers "commit, merge to main, and push", "get it all on main", and "clean up stale branches". Skip when the ask is only to open a PR, which create-pr owns.
metadata:
  minimum-tier: T3
  short-description: Put pending work through a PR and prune merged branches
---

# git-sync

JOB: Put every pending change into a PR and prune only branches that main contains.
IN:  A Git repository with possible local changes, work branches, remote changes, merged branches, or a visual-evidence manifest.
OUT: The step 8 report. It names the PR URL, published hash, commits, branch verdicts, evidence, refusals, and omitted files.

## hard rules

- Never commit, merge, squash, or push directly on main. Every change reaches main through a PR.
- This skill prepares a final PR, not an evidence-pending initial draft. The initial-draft
  exception belongs only to a direct create-pr run.
- A branch is deletable only on one of two proofs: `git branch --merged origin/main` lists it, OR `git diff origin/main <branch>` is run and shown empty. Never infer stale status from its name or age. The second proof exists because a squash-merged PR never makes its branch an ancestor of main, so `--merged` alone would keep every squash-landed branch forever.
- Never delete the current PR branch during the run. A squash merge does not make that branch an ancestor of main.
- `git branch -D` is legal only directly after the empty-diff proof, because `-d` refuses a squash-merged branch however completely main holds it. Without that shown proof, keep the branch.
- Never force-push. A rejected push means that the remote diverged. Stop and ask the user.
- Never rebase a branch after another person can use its pushed commits.
- No AI(Artificial Intelligence) attribution can enter a commit or PR body. Inspect both outputs before reporting success.
- Never commit temporary screenshots, recordings, or a visual-evidence manifest. Commit
  media only when it belongs in product documentation.
- Explicit instructions in the user's request beat every default except the safety rules above.

## steps

1. **Establish facts.** Run `git fetch --prune origin`, `git status -sb`, and `git branch -avv`. Compare the current branch with its upstream when one exists. Run `gh pr list --head <branch>`. Done when the output shows the branch, changes, divergence, and an existing PR.

2. **Route direct PR requests.** If the request asks only for a PR, invoke `/create-pr` and stop. Do not run branch cleanup unless the user asks for it. Done when `/create-pr` returns its report.

3. **Protect main.** When main has pending work, create `git-sync/$(date +%Y%m%d-%H%M%S)` before staging anything. Stop if work exists and Git cannot create the branch. Done when pending work is on a non-main branch.

4. **Commit pending work.** Review the diff and every untracked file. Decide whether the work alters what a user can see or do. Interface-file contact alone
does not qualify. Require a visual-evidence manifest for qualifying work. Forward any
supplied manifest for other work as relevant evidence. If a required manifest is
missing, stop before staging and ask for test evidence. When a manifest is required or
supplied, use step 1 to select `gh pr create --help` or `gh pr edit --help`. Require the
relevant output to contain `--attach`. Otherwise, stop before staging and report the requirement for GitHub Command Line Interface 2.99.0 or newer.

   Exclude each temporary path in the manifest. Split unrelated concerns into separate commits. Route each message through `/byline`, then run `ste-check --register byline`. Reject an attributed draft before the commit. Verify each final message contains no attribution. Done when the tree is clean or every omitted file has a reason.

5. **Handle remote divergence.** Rebase unpublished local commits onto their current upstream when the upstream is ahead. Stop and name each conflict. Never resolve a conflict or use `git rebase --skip`. Do not rewrite commits that another person can use. Done when the branch can push without force, or the report names the blocker.

6. **Create the PR.** Invoke `/create-pr` for the current branch and target main. Forward each closing ticket id from the caller. Forward the visual-evidence manifest unchanged when the caller supplies one. Never upload or commit temporary evidence in this skill. The clean branch allows `/create-pr` to push and open or update the PR. Never reproduce its push procedure here. Done when `/create-pr` returns a verified PR URL and its applicable evidence checks, or its failure becomes the named refusal.

7. **Triage merged branches.** Run both commands after the PR step:
   ```sh
   git branch --merged origin/main | grep -v '^\*'
   git branch --no-merged origin/main
   ```
   Copy deletion candidates from the first command, and delete those with `git branch -d`. For each unmerged branch, run `git diff origin/main <branch>`: an empty diff proves main holds the content and `git branch -D` with that shown proof deletes it; a non-empty diff means work, and the branch stays with its `git rev-list --count origin/main..<branch>` recorded. Keep the current PR branch even when a proof lists it. Done when every local branch has a recorded verdict.

8. **Verify and report.** Run `git rev-parse HEAD` and `git rev-parse @{u}`. Compare the hashes literally. Verify the PR base is main and its head is the current branch. Emit exactly this shape:
   ```text
   PR:        <URL>
   published: <matching local and upstream hash>
   commits:   <hash and subject>
   branches:  <name and verdict>
   evidence:  <uploaded visual evidence, when applicable>
   refused:   <reason>
   left out:  <files and reason>
   ```
   Write commits newest first. Name each kept branch's commit count. Omit `refused` and `left out` when empty. Done when the hashes match and the report contains every result from steps 4 through 8.

## evals

`evals/run.sh` grades each development case in `evals/cases.jsonl` against this file or a candidate. It uses `evals/rubric.md`. `--holdout` runs the held-out slice. It prints one JSON(JavaScript Object Notation) line per case and the mean score.
