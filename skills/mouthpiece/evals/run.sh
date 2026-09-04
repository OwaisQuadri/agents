#!/bin/zsh
set -euo pipefail
here=${0:A:h}
repo=$(git -C "$here" rev-parse --show-toplevel)
exec "$repo/tools/skill-eval/run.sh" --eval-dir "$here" "$@"
