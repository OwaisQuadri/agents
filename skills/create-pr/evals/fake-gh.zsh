#!/bin/zsh
set -euo pipefail

workspace=${CREATE_PR_EVAL_WORKSPACE:?}
case_id=${CREATE_PR_EVAL_CASE_ID:?}
print -r -- "gh ${(j: :)${(q)@}}" >> "$workspace/.harness/actions.log"
[[ "${1:-}" == pr ]] || { print -u2 -r -- 'only gh pr commands are available'; exit 2; }
subcommand=${2:-}
case "$subcommand" in
  list) print -r -- '[]' ;;
  create)
    if [[ "$case_id" == c5-auth-failure ]]; then
      print -u2 -r -- 'authentication failed: run gh auth login'
      exit 1
    fi
    print -r -- "https://example.invalid/pull/${case_id[2]}"
    ;;
  view)
    print -r -- '{"body":"Fixture pull request body without attribution"}'
    ;;
  edit) print -r -- 'updated simulated pull request' ;;
  *) print -u2 -r -- "unsupported simulated gh pr command: $subcommand"; exit 2 ;;
esac
