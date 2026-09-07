#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(git -C "$HERE" rev-parse --show-toplevel)"
exec "$REPO/tools/skill-eval/run.sh" --eval-dir "$HERE" "$@"
