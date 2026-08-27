#!/bin/zsh
set -euo pipefail

workspace=${CREATE_PR_EVAL_WORKSPACE:?}
case_id=${CREATE_PR_EVAL_CASE_ID:?}
print -r -- "git ${(j: :)${(q)@}}" >> "$workspace/.harness/actions.log"
command_name=${1:-}
shift || true

case "$command_name" in
  status)
    case "$case_id" in
      c3-clean-ahead) print -r -- '## feature/complete-diff...origin/feature/complete-diff [ahead 3]' ;;
      c1-branch-mismatch) print -r -- '## OwaisQuadri/create-pr
 M src/parser.rs
 M docs/parser.md' ;;
      c2-no-attribution) print -r -- '## feature/export
 M src/export.rs
 M docs/export.md' ;;
      c4-push-only) print -r -- '## feature/local
 M src/local.rs' ;;
      c5-auth-failure) print -r -- '## feature/auth
 M src/auth.rs' ;;
      c6-full-workspace-diff) print -r -- '## feature/workspace
 M src/api.rs
 M db/migration.sql
 M docs/runbook.md' ;;
    esac
    ;;
  branch)
    case "$case_id" in
      c1-branch-mismatch) print -r -- '* OwaisQuadri/create-pr abc123 [origin/OwaisQuadri/create-pr] pending changes' ;;
      c2-no-attribution) print -r -- '* feature/export def456 [origin/feature/export] pending changes' ;;
      c3-clean-ahead) print -r -- '* feature/complete-diff 333333 [origin/feature/complete-diff: ahead 3] complete diff' ;;
      c4-push-only) print -r -- '* feature/local 444444 [origin/feature/local] local change' ;;
      c5-auth-failure) print -r -- '* feature/auth 555555 [origin/feature/auth] auth change' ;;
      c6-full-workspace-diff) print -r -- '* feature/workspace 666666 [origin/feature/workspace] workspace changes' ;;
    esac
    ;;
  log)
    if [[ " $* " == *' -1 '* ]]; then
      print -r -- 'Update repository files'
    elif [[ "$case_id" == c3-clean-ahead ]]; then
      print -r -- '333333 Document resolver
222222 Add cache
111111 Update parser'
    fi
    ;;
  diff)
    case "$case_id" in
      c1-branch-mismatch) print -r -- 'diff --git a/src/parser.rs b/src/parser.rs
diff --git a/docs/parser.md b/docs/parser.md' ;;
      c2-no-attribution) print -r -- 'diff --git a/src/export.rs b/src/export.rs
diff --git a/docs/export.md b/docs/export.md' ;;
      c3-clean-ahead) print -r -- 'diff --git a/src/parser.rs b/src/parser.rs
diff --git a/src/cache.rs b/src/cache.rs
diff --git a/README.md b/README.md' ;;
      c4-push-only) print -r -- 'diff --git a/src/local.rs b/src/local.rs' ;;
      c5-auth-failure) print -r -- 'diff --git a/src/auth.rs b/src/auth.rs' ;;
      c6-full-workspace-diff) print -r -- 'diff --git a/src/api.rs b/src/api.rs
diff --git a/db/migration.sql b/db/migration.sql
diff --git a/docs/runbook.md b/docs/runbook.md' ;;
    esac
    ;;
  rev-parse)
    if [[ "${1:-}" == '--show-toplevel' ]]; then print -r -- "$workspace"; else print -r -- 'abc123'; fi
    ;;
  remote) print -r -- 'origin' ;;
  add|commit|push) print -r -- "simulated $command_name; no repository or remote mutation occurred" ;;
  show) print -r -- 'Update repository files' ;;
  *) print -u2 -r -- "unsupported simulated git command: $command_name $*"; exit 2 ;;
esac
