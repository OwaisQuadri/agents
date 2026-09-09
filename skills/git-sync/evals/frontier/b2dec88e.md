---
name: git-sync
description: Use when pending work must reach main and the remote, or when branches need cleanup. Put all work through a PR(Pull Request). Prune branches that main contains, or prepare an exact manual deletion override. Skip when the ask is only to open a PR, which create-pr owns.
metadata:
  short-description: Put pending work through a PR and prune branches
---

# git-sync

JOB: Put every pending change into a PR and prune branches through proof or an exact manual override.
IN: A Git repository with possible changes, branches, remote changes, or a visual-evidence manifest.
OUT: The step 8 report names publication, branch verdicts, override plans, recovery references, evidence, refusals, and omitted files.

## hard rules

- Never commit, merge, squash, or push directly on main. Every change reaches main through a PR.
- This skill prepares a final PR, not an evidence-pending initial draft. Only a direct create-pr run has that exception.
- Normal cleanup deletes a branch only after one of two proofs. `git branch --merged origin/main` lists it, or `git diff origin/main <branch>` is empty.
- A completed manual override can delete an unproved branch. Step 7 defines the complete override protocol.
- Never infer stale status from a branch name or age.
- Never delete local main, a remote main, a remote default branch, or a symbolic remote HEAD reference.
- Never delete the current PR branch during normal cleanup.
- Use `git branch -D` only after an empty-diff proof or a completed manual override.
- Never force-push branch content. The override permits an expected-value lease only for an exact remote deletion.
- Stop after a rejected push. Ask the user because the remote diverged.
- Never rebase a branch after another person can use its pushed commits.
- No AI(Artificial Intelligence) attribution can enter a commit or PR body. Inspect both outputs before success.
- Never commit temporary screenshots, recordings, or a visual-evidence manifest. Commit media only for product documentation.
- Explicit instructions beat every default except these hard rules.

## steps

1. **Establish facts.** Run `git fetch --prune origin`, `git status -sb`, and `git branch -avv`. Compare the current branch with its upstream. Run `gh pr list --head <branch>`. Done when the output shows the branch, changes, divergence, and an existing PR.

2. **Route direct PR requests.** Invoke `/create-pr` when the request asks only for a PR. Stop after it returns its report. Do not clean branches unless the user asks. Done when `/create-pr` returns its report.

3. **Protect main.** Create `git-sync/$(date +%Y%m%d-%H%M%S)` when main has pending work. Stop if Git cannot create it. Done when pending work is on a non-main branch.

4. **Commit pending work.** Review the diff and every untracked file. Decide whether the work changes visible or interactive behavior. Interface-file contact alone does not qualify. Require a visual-evidence manifest for qualifying work. Treat a supplied manifest for other work as relevant evidence. Stop before staging when required evidence is absent.

   Use step 1 to select `gh pr create --help` or `gh pr edit --help` when evidence applies. Require the output to contain `--attach`. Otherwise, require GitHub Command Line Interface 2.99.0 or newer.

   Exclude each temporary manifest path. Split unrelated concerns into separate commits. Route each message through `/byline`. Run `ste-check --register byline`. Reject an attributed draft. Done when the tree is clean or each omitted file has a reason.

5. **Handle remote divergence.** Rebase unpublished local commits when their upstream is ahead. Stop and name each conflict. Never resolve a conflict or use `git rebase --skip`. Do not rewrite shared commits. Done when the branch can push without force, or the report names the blocker.

6. **Create the PR.** Invoke `/create-pr` for the current branch and target main. Forward each closing ticket identifier. Forward the visual-evidence manifest unchanged. Never upload or commit temporary evidence here. Never reproduce the push procedure. Done when `/create-pr` returns a verified URL and evidence result, or its failure becomes the refusal.

7. **Triage branches.** Use normal cleanup unless the user completes the manual override branch below.

   **Normal cleanup.** Run:
   ```sh
   git branch --merged origin/main | grep -v '^\*'
   git branch --no-merged origin/main
   ```
   Delete merged candidates with `git branch -d`. Run `git diff origin/main <branch>` for each unmerged branch. Delete it with `git branch -D` only when that diff is empty. Keep each other branch.

   Record its `git rev-list --count origin/main..<branch>` result. Keep the current PR branch. Done when every local branch has a verdict.

   **Manual override, stage one.** Treat any delete request as permission to prepare an inventory only. This includes requests that say `all`, `force`, or use a prefix. Expand the scope into exact full local and remote reference names. Never pass a wildcard to Git.

   Set the common directory with `git rev-parse --path-format=absolute --git-common-dir`. Write the canonical plan under `<common-dir>/git-sync/override/<run-id>.json`. Use `jq -S -c` for canonical JSON(JavaScript Object Notation). Compute its plan identifier with `shasum -a 256`.

   The plan contains these fields:
   ```text
   run_id
   local_main: {ref, oid}
   remotes: [{name, default_ref, default_oid, head_ref}]
   targets: [{scope, ref, oid, recovery_ref, restore_command}]
   worktrees: [{path, branch_ref, oid, locked, status}]
   pull_requests: [{number, state, head_ref, base_ref}]
   ```
   Include each local and remote target. Include every worktree from `git worktree list --porcelain`. Record each worktree status with `git status --porcelain=v2`. Include every open PR that uses a target as its head or base.

   Reject local main, every remote main, every remote default branch, and every symbolic remote HEAD reference from the target set. This protection has no override. Write one recovery reference and one restore command for each target. Show the complete plan and its plan identifier.

   Require this exact user message in a later turn:
   ```text
   DELETE OVERRIDE <plan-id>: delete every non-main local and remote branch listed in this inventory even if main lacks its content. Close these open Pull Requests: <numbers or none>. I accept the possible loss of unmerged branch content.
   ```
   Stop after the inventory. Done when the user can compare each exact target, PR, recovery reference, and restore command.

   **Manual override, stage two.** Read the saved plan after the exact confirmation arrives. Rebuild the complete inventory from live Git and GitHub state. Canonicalize and hash it again. Stop unless its bytes and plan identifier match the saved plan. Reject a near-match confirmation.

   Stop when any branch tip, target, worktree, status, lock, or PR set changed. Prepare a new plan instead. Stop when the confirmed PR numbers differ from the plan. Use `none` only when the plan has no affected open PR.

   Create every recovery reference with `git update-ref <recovery-ref> <oid>`. Verify each one with `git rev-parse <recovery-ref>`. Stop before deletion when any value differs. Keep all recovery references after success or failure.

   Refuse a target that has a dirty or changed attached worktree. Preserve its files and branch. For each clean attached target, run `git -C <path> switch --detach <oid>`. Verify that its worktree status stays unchanged.

   Close only the confirmed open PR numbers with `gh pr close <number>`. Verify each closed state. Group remote targets by remote. Delete each group atomically with exact deletion refspecs and one `--force-with-lease=<ref>:<oid>` argument per target. Never update branch content with that option.

   Delete each exact local target with `git branch -D -- <name>` after its recovery reference passes. Never pass a wildcard. Stop and report any partial failure.

   Verify that every target reference is absent. Verify that every recovery reference still resolves to its recorded object identifier. Verify that protected references retain their recorded identifiers. Verify that every retained worktree keeps its recorded pending content. For an `all` scope, verify that only protected branches remain. Done when every target has a verified result or a named refusal.

8. **Verify and report.** Run `git rev-parse HEAD` and `git rev-parse @{u}`. Compare the hashes. Verify the PR base and head when a PR exists. Emit exactly this shape:
   ```text
   PR:        <URL or none>
   published: <matching local and upstream hash>
   commits:   <hash and subject>
   plan:      <override plan identifier and path, when applicable>
   recovery:  <reference, original object identifier, and restore command>
   branches:  <name and verdict>
   evidence:  <uploaded visual evidence, when applicable>
   refused:   <reason>
   left out:  <files and reason>
   ```
   Write commits newest first. Name each kept branch's commit count. Omit empty optional fields. Done when the report contains every result from steps 4 through 8.

## evals

`evals/run.sh` grades each development case against this file or a candidate. It uses `evals/rubric.md`. `--holdout` runs the held-out slice. It prints one JSON(JavaScript Object Notation) line per case and the mean score.
