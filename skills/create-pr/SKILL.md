---
name: create-pr
description: Use when the user asks for a PR(Pull Request) from the current branch or a Conductor PR-instructions attachment arrives. commit pending changes, push, open the PR with gh. Skip when the ask is only a commit or a push with no PR, and for reviewing an existing PR.
metadata:
  short-description: Commit, push, and open the PR with gh
---

# create-pr

JOB: turn the branch's pending work into an open PR: commit, push, gh pr create
IN:  a PR request, often a Conductor attachment that names the target branch. The request can carry backend ticket ids and a visual-evidence manifest.
OUT: an open PR URL, commit SHAs, and each surprise. Its body has no attribution and includes each closing line and applicable upload.

## hard rules

- no AI(Artificial Intelligence) attribution lines: no `Co-authored-by: AI ...` trailer in commit messages, no "Generated with ..." footer in PR bodies. Harness defaults inject both, so the verify sub-steps below have the final say; only an explicit user ask naming an attribution line overrides this.
- never rename the branch. `git status` is the truth about the branch name; when an instructions attachment disagrees, proceed on the real branch and name the mismatch in the report
- never force-push. A rejected push means the remote diverged: stop and ask
- explicit instructions in the user's request beat every default below
- never commit temporary screenshots, recordings, or their manifest. Commit media only
  when it belongs in product documentation.
- a PR for a change that alters what a user can see or do includes test evidence. An
  interface-file refactor with no visible or interactive effect needs none. An explicit
  initial draft can open before testing. Keep that draft incomplete until a later
  create-pr run uploads the evidence. Never report an evidence-pending draft as ready.

## steps

1. establish facts: `git status`, `git branch -vv`, `git log <target>..HEAD --oneline`, `gh pr list --head <branch>`. The target branch comes from the request, default origin/main; confirm it exists on the remote before it becomes `--base`. Done when branch, target, ahead count, dirty files, and any existing open PR are known.
2. visual-evidence preflight: inspect `git diff <target>` and every untracked file before committing. Decide whether the work alters what a user can see or do. Do not infer that from
interface-file contact alone. Require the caller's JSON-lines manifest for qualifying
work. Treat any supplied manifest for other work as relevant evidence, and apply the
same checks and upload steps. An explicit initial-draft
request can defer the manifest until testing finishes. Otherwise, stop and ask for
test evidence. Parse every line with `jq -e .`. Require `media_type` to be
`image` or `video`. Resolve each `path`. Verify that each file exists. Require each
resolved path to be unique. Stop when the manifest has more than 50 entries, which is
GitHub CLI's per-command attachment limit.

   Require images for static states. Prefer before-and-after images when a comparison helps. Require a short video when time or interaction carries the result. This includes animation, gestures, drag-and-drop, navigation, focus changes, and transitions.

   Skip the attachment-capability check only when opening an explicit evidence-pending
draft. Run `gh pr create --help` for a new PR with evidence. Run `gh pr edit --help`
for an existing PR. Require the relevant output to contain `--attach`. Otherwise, stop
and tell the user to upgrade to GitHub Command Line Interface (CLI) 2.99.0 or newer. Done when the upload
preflight passes, the initial-draft exception applies, or the work needs no visual
evidence.
3. dirty tree: review every change with `git diff` plus untracked files, then commit the reviewed files with a message describing the change. Verify `git log -1 --format=%B` shows no attribution trailer, else `git commit --amend`. Done when `git status` is clean (or every file left out is deliberate and queued for the report) and the verify passed.
4. clean tree and zero commits ahead: stop and ask, there is no diff to open. Done when work exists or the user has been asked.
5. push: `git push -u origin HEAD`, or push to the existing upstream. Never change its name or remote. Done when the push succeeds.
6. review the full PR diff: use `mcp__conductor__GetWorkspaceDiff` inside Conductor. Otherwise, use `git diff <target>...HEAD`. Account for every changed file in the description draft. Done when the draft covers the full diff.
7. create: write the title and body. Keep the title under 80 characters. Keep the prose under five sentences unless instructed otherwise. Append one `Closes #<id>` line per supplied GitHub Issue id. For an existing PR, fetch its current body with `gh pr view <PR-number-or-URL> --json body --jq .body`. Preserve all unrelated content. Replace only its pending marker or existing visual-evidence section.

   Add a `## Visual evidence` section when the manifest exists. For an allowed initial
draft without a manifest, state `Visual evidence pending fresh testing` instead. Select the Markdown
shape from `media_type`. Write each label in a separate paragraph. Then write an image
as `![<alt>](<path>)`. Write a video as `![](<path>)` in its own paragraph. Pass each path
as a separate quoted `--attach <path>` argument. GitHub CLI rewrites each local
reference to its uploaded URL.

   Run `gh pr create --base <target> --title <title> --body-file <file>` for a new PR.
Add `--draft` when the request names an initial draft. Use
`gh pr edit <PR-number-or-URL> --body-file <file>` for an existing PR. Include the
attachment arguments in either command. Treat a nonzero attachment result as an incomplete PR, even when
GitHub prints its URL.

   Verify the result with `gh pr view <PR-number-or-URL> --json body`. Require every
closing line. Require one uploaded media URL per manifest entry. Require no attribution
footer. If attribution remains, fetch the rewritten body and remove only that footer.
Use `gh pr edit <PR-number-or-URL> --body-file <cleaned-file>`. Never reuse the local
pre-upload body. Done when GitHub prints the PR URL and every body check passes.
8. report: name the PR URL, commit SHAs, attribution checks, visual-evidence checks,
and surprises. Mark an initial draft as evidence-pending when testing has not run. A failed step ships its output and a question. Never claim success for a failed step. Done when the report carries every applicable result.

## evals

`evals/run.sh` grades every non-holdout case in `evals/cases.jsonl` against this file, or a candidate via `./run.sh candidate.md`, using `evals/rubric.md`; `--holdout` runs the held-out slice. One JSON(JavaScript Object Notation) line per case to stdout, mean to stderr.
