#!/bin/zsh
set -euo pipefail
root=${0:A:h:h}
if [[ "${1:-}" == "--holdout" ]]; then
  print -r -- holdout >> "$root/.eval-runs"
  print -r -- '{"id":"h1","score":10,"failure_mode":null}'
  print -u2 -r -- 'mean 10.00 over 1 cases (holdout slice)'
else
  print -r -- nonholdout >> "$root/.eval-runs"
  print -r -- '{"id":"n1","score":10,"failure_mode":null}'
  print -u2 -r -- 'mean 10.00 over 1 cases (nonholdout slice)'
fi
