#!/bin/zsh
set -euo pipefail

for hidden_path in "$BRO_EVAL_HIDDEN_RUBRIC" "$BRO_EVAL_HIDDEN_CASES" "$BRO_EVAL_HIDDEN_SOURCE" "$BRO_EVAL_HIDDEN_HOME" "$BRO_EVAL_HIDDEN_SNAPSHOT"; do
  if content=$(<"$hidden_path" 2>/dev/null); then
    print -u2 -r -- "hidden path was readable: $hidden_path"
    exit 80
  fi
done

print -r -- "There is nothing to simplify yet."
