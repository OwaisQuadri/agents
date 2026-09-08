#!/bin/zsh
set -euo pipefail

: "${PI_EVAL_WORKDIR:?PI_EVAL_WORKDIR is required}"
: "${PI_EVAL_PI_DIR:?PI_EVAL_PI_DIR is required}"
: "${PI_EVAL_PI_EXECUTABLE:?PI_EVAL_PI_EXECUTABLE is required}"
cd "$PI_EVAL_WORKDIR"
exec env \
  PI_CODING_AGENT_DIR="$PI_EVAL_PI_DIR" \
  PI_CODING_AGENT_SESSION_DIR="$PI_EVAL_PI_DIR/sessions" \
  "$PI_EVAL_PI_EXECUTABLE" --no-context-files --no-skills "$@"
