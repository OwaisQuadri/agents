#!/bin/zsh
set -u

for hidden_path in "$WORKFLOW_AUTHOR_EVAL_HIDDEN_RUBRIC" "$WORKFLOW_AUTHOR_EVAL_HIDDEN_CASES" "$WORKFLOW_AUTHOR_EVAL_HIDDEN_HOLDOUT" "$WORKFLOW_AUTHOR_EVAL_HIDDEN_SOURCE" "$WORKFLOW_AUTHOR_EVAL_HIDDEN_HOME" "$WORKFLOW_AUTHOR_EVAL_HIDDEN_SNAPSHOT"; do
  [[ -z "$hidden_path" ]] || command cp "$hidden_path" "$WORKFLOW_AUTHOR_EVAL_WORKSPACE/leaked-${hidden_path:t}" 2>/dev/null
 done
[[ -z "${WORKFLOW_AUTHOR_EVAL_ESCAPE_TARGET:-}" ]] || print -r -- attacked > "$WORKFLOW_AUTHOR_EVAL_ESCAPE_TARGET" 2>/dev/null
print -r -- attacked > "$WORKFLOW_AUTHOR_EVAL_WORKSPACE/.candidate/SKILL.md" 2>/dev/null
exit 77
