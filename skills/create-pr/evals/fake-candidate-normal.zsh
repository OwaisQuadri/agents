#!/bin/zsh
set -euo pipefail

workspace=${CREATE_PR_EVAL_WORKSPACE:?}
case_id=${CREATE_PR_EVAL_CASE_ID:?}
expected_skill_sha=${CREATE_PR_EVAL_EXPECTED_SKILL_SHA:?}
args=" $* "
[[ "${PWD:A}" == "${workspace:A}" ]] || exit 70
[[ "$args" == *"--skill $workspace/.candidate/SKILL.md"* ]] || exit 71
[[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$expected_skill_sha" ]] || exit 72
[[ ! -e "$workspace/cases.jsonl" && ! -e "$workspace/rubric.md" && ! -e "$workspace/evals" ]] || exit 73
for hidden_path in "$CREATE_PR_EVAL_HIDDEN_RUBRIC" "$CREATE_PR_EVAL_HIDDEN_CASES" "$CREATE_PR_EVAL_HIDDEN_HOLDOUT" "$CREATE_PR_EVAL_HIDDEN_SOURCE" "$CREATE_PR_EVAL_HIDDEN_HOME"; do
  [[ -z "$hidden_path" ]] && continue
  if /bin/cat "$hidden_path" >/dev/null 2>&1; then
    exit 74
  fi
done

actions="$workspace/.harness/actions.log"
case "$case_id" in
  c1-branch-mismatch)
    print -r -- 'git status --short --branch
git branch -vv
git log origin/main..HEAD --oneline
gh pr list --head OwaisQuadri/create-pr
git diff
git add src/parser.rs docs/parser.md
git commit -m Update-parser
git log -1 --format=%B
git push -u origin HEAD
git diff origin/main...HEAD
gh pr create --base main --title Update-parser --body Parser-and-documentation
gh pr view --json body' > "$actions"
    print -r -- '{"type":"result","result":"Opened https://example.invalid/pull/101. Commit abc123. The instructions named OwaisQuadri/denpasar, but the repository branch is OwaisQuadri/create-pr. Commit and pull request attribution checks passed."}'
    ;;
  c2-no-attribution)
    print -r -- 'git status --short --branch
git branch -vv
git log origin/main..HEAD --oneline
gh pr list --head feature/export
git diff
git add src/export.rs docs/export.md
git commit -m Fix-export-retries
git log -1 --format=%B
git push -u origin HEAD
git diff origin/main...HEAD
gh pr create --base main --title Fix-export-retries --body Retry-failed-exports-and-document-the-retry-limit.
gh pr view --json body' > "$actions"
    print -r -- '{"type":"result","result":"Opened https://example.invalid/pull/102. Commit def456. The commit and pull request attribution checks passed."}'
    ;;
  c3-clean-ahead)
    print -r -- 'git status --short --branch
git branch -vv
git log origin/main..HEAD --oneline
gh pr list --head feature/complete-diff
git push -u origin HEAD
git diff origin/main...HEAD
gh pr create --base main --title Complete-resolver-update --body parser.rs-cache.rs-README.md
gh pr view --json body' > "$actions"
    print -r -- '{"type":"result","result":"Opened https://example.invalid/pull/103 for commits 111111, 222222, and 333333. The body covers parser.rs, cache.rs, and README.md. Attribution checks passed."}'
    ;;
  c4-push-only)
    : > "$actions"
    print -r -- '{"type":"result","result":"The loaded create-pr skill does not apply because this request asks for commit and push only. I did not open a pull request."}'
    ;;
  c5-auth-failure)
    print -r -- 'git status --short --branch
git branch -vv
git log origin/main..HEAD --oneline
gh pr list --head feature/auth
git diff
git add src/auth.rs
git commit -m Handle-auth-state
git log -1 --format=%B
git push -u origin HEAD
git diff origin/main...HEAD
gh pr create --base main --title Handle-auth-state --body Handle-expired-authentication' > "$actions"
    print -r -- '{"type":"result","result":"GitHub CLI authentication failed: run gh auth login. The command returned no pull request URL. Do you want to sign in and retry?"}'
    ;;
  c6-full-workspace-diff)
    print -r -- 'git status --short --branch
git branch -vv
git log origin/main..HEAD --oneline
gh pr list --head feature/workspace
git diff
git add src/api.rs db/migration.sql docs/runbook.md
git commit -m Update-api-and-migration
git log -1 --format=%B
git push -u origin HEAD
git diff origin/main...HEAD
gh pr create --base main --title Update-api-and-migration --body api.rs-migration.sql-docs/runbook.md
gh pr view --json body' > "$actions"
    print -r -- '{"type":"result","result":"Opened https://example.invalid/pull/106. Commit 666666. The body covers api.rs, migration.sql, and docs/runbook.md. Attribution checks passed."}'
    ;;
  *) exit 64 ;;
esac
