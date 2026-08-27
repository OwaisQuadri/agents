#!/bin/zsh
set -euo pipefail

workspace=${GIT_SYNC_EVAL_WORKSPACE:?}
case_id=${GIT_SYNC_EVAL_CASE_ID:?}
expected_skill_sha=${GIT_SYNC_EVAL_EXPECTED_SKILL_SHA:?}
args=" $* "
[[ "${PWD:A}" == "${workspace:A}" ]] || exit 70
[[ "$args" == *"--skill $workspace/.candidate/SKILL.md"* ]] || exit 71
[[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$expected_skill_sha" ]] || exit 72
[[ ! -e "$workspace/cases.jsonl" && ! -e "$workspace/rubric.md" && ! -e "$workspace/evals" ]] || exit 73
for hidden_path in "$GIT_SYNC_EVAL_HIDDEN_RUBRIC" "$GIT_SYNC_EVAL_HIDDEN_CASES" "$GIT_SYNC_EVAL_HIDDEN_HOLDOUT" "$GIT_SYNC_EVAL_HIDDEN_SOURCE" "$GIT_SYNC_EVAL_HIDDEN_HOME"; do
  [[ -z "$hidden_path" ]] && continue
  /bin/cat "$hidden_path" >/dev/null 2>&1 && exit 74
done

actions="$workspace/.harness/actions.log"
case "$case_id" in
  g1)
    print -r -- 'git fetch --prune origin
git status -sb
git branch -avv
gh pr list --head feat/x
git diff
git add src/parser.rs
git commit -m Fix-parser
git add src/install.rs
git commit -m Add-install-script
git push -u origin HEAD
gh pr create --base main --head feat/x
git branch --merged origin/main
git branch --no-merged origin/main
git rev-parse HEAD
git rev-parse @{u}
gh pr view --json url,baseRefName,headRefName,body' > "$actions"
    print -r -- '{"type":"result","result":"PR: https://example.invalid/pull/1\npublished: feedfacefeedfacefeedfacefeedfacefeedface\ncommits: feedfac Add install script; abc1234 Fix parser\nbranches: feat/x kept as current pull request branch"}'
    ;;
  g2)
    print -r -- 'git fetch --prune origin
git status -sb
git branch -avv
git branch --merged origin/main
git branch --no-merged origin/main
git diff origin/main origin/map/RAG-0001
git rev-list --count origin/main..origin/map/RAG-0001' > "$actions"
    print -r -- '{"type":"result","result":"PR: none; no pending work\nbranches: origin/map/RAG-0001 kept, unmerged, 43 commits and 12677 insertions\nrefused: remote-only work is not contained in origin/main"}'
    ;;
  g3)
    print -r -- 'git fetch --prune origin
git status -sb
git branch -avv
gh pr list --head work
git rebase origin/work
git push -u origin HEAD
gh pr create --base main --head work
git branch --merged origin/main
git branch --no-merged origin/main
git rev-parse HEAD
git rev-parse @{u}' > "$actions"
    print -r -- '{"type":"result","result":"PR: https://example.invalid/pull/3\npublished: feedfacefeedfacefeedfacefeedfacefeedface\ncommits: two local commits rebased onto origin/work\nbranches: work kept as current pull request branch\nrefused: any rebase conflict would stop for the user"}'
    ;;
  g4)
    print -r -- 'git fetch --prune origin
git status -sb
git branch -avv
gh pr list --head fix/parser
git diff
git add src/parser.rs
git commit -m Fix-parser
git push -u origin HEAD
gh pr create --base main --head fix/parser
git rev-parse HEAD
git rev-parse @{u}' > "$actions"
    print -r -- '{"type":"result","result":"PR: https://example.invalid/pull/4\npublished: feedfacefeedfacefeedfacefeedfacefeedface\ncommits: feedfac Fix parser\nbranches: cleanup omitted because the request asks only for a pull request"}'
    ;;
  g5)
    print -r -- 'git fetch --prune origin
git status -sb
git branch -avv
gh pr list --head feat/y
git push -u origin HEAD
gh pr create --base main --head feat/y
git branch --merged origin/main
git branch --no-merged origin/main
git rev-parse HEAD
git rev-parse @{u}' > "$actions"
    print -r -- '{"type":"result","result":"PR: https://example.invalid/pull/5\npublished: feedfacefeedfacefeedfacefeedfacefeedface\ncommits: feedfac Feature y\nbranches: feat/y kept; local main left at def5678; pull request targets current origin/main 9ab0000"}'
    ;;
  g6)
    print -r -- 'git fetch --prune origin
git status -sb
git branch -avv
git branch --merged origin/main
git branch --no-merged origin/main
git branch -d feat/a
git branch -d feat/b
git diff origin/main spike/c
git rev-list --count origin/main..spike/c' > "$actions"
    print -r -- '{"type":"result","result":"PR: none; no pending work\nbranches: feat/a deleted by merged proof; feat/b deleted by merged proof; spike/c kept with 2 commits"}'
    ;;
  g7)
    print -r -- 'git fetch --prune origin
git status -sb
git branch -avv
gh pr list --head feat/attribution
git diff
git add src/change.rs
git commit -m Safe-change
git log -1 --format=%B
git push -u origin HEAD
gh pr create --base main --head feat/attribution
gh pr view --json body
git rev-parse HEAD
git rev-parse @{u}' > "$actions"
    print -r -- '{"type":"result","result":"PR: https://example.invalid/pull/7\npublished: feedfacefeedfacefeedfacefeedfacefeedface\ncommits: feedfac Safe change\nbranches: feat/attribution kept\nrefused: attributed draft rejected; final commit and pull request body checks passed"}'
    ;;
  g8)
    print -r -- 'git fetch --prune origin
git status -sb
git branch -avv
git branch -c git-sync/20260823-120000
git diff
git add src/change.rs
git commit -m Safe-change
git push -u origin HEAD
gh pr create --base main --head git-sync/20260823-120000
git rev-parse HEAD
git rev-parse @{u}' > "$actions"
    print -r -- '{"type":"result","result":"PR: https://example.invalid/pull/8\npublished: feedfacefeedfacefeedfacefeedfacefeedface\ncommits: feedfac Safe change\nbranches: temporary work branch kept; main unchanged"}'
    ;;
  g9)
    print -r -- 'git fetch --prune origin
git status -sb
git branch -avv
git branch --merged origin/main
git branch --no-merged origin/main
git diff origin/main feat/exporter
git branch -d feat/exporter
git branch -D feat/exporter' > "$actions"
    print -r -- '{"type":"result","result":"PR: none; no pending work\nbranches: feat/exporter deleted after an empty diff proved origin/main holds its content; the merged listing missed it because squash integration changed ancestry; safe deletion refused before forced deletion"}'
    ;;
  g10)
    print -r -- 'git fetch --prune origin
git status -sb
git branch -avv
git branch --merged origin/main
git branch --no-merged origin/main
git diff --stat origin/main spike/wasm-loader
git rev-list --count origin/main..spike/wasm-loader' > "$actions"
    print -r -- '{"type":"result","result":"PR: none; no pending work\nbranches: spike/wasm-loader kept because origin/main lacks 5 files and 340 changed lines; its age and name are not deletion evidence"}'
    ;;
  *) exit 64 ;;
esac
