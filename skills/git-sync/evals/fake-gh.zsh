#!/bin/zsh
set -euo pipefail

workspace=${GIT_SYNC_EVAL_WORKSPACE:?}
case_id=${GIT_SYNC_EVAL_CASE_ID:?}
print -r -- "gh ${(j: :)${(q)@}}" >> "$workspace/.harness/actions.log"
[[ "${1:-}" == pr ]] || { print -u2 -r -- 'only simulated pull request commands are available'; exit 2; }
case "${2:-}" in
  list) print -r -- '[]' ;;
  create) print -r -- "https://example.invalid/pull/${case_id#g}" ;;
  view) print -r -- '{"url":"https://example.invalid/pull/fixture","baseRefName":"main","headRefName":"fixture","body":"Fixture body without attribution"}' ;;
  edit) print -r -- 'simulated pull request update' ;;
  *) print -u2 -r -- "unsupported simulated gh command: $*"; exit 2 ;;
esac
