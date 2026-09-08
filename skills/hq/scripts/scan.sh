#!/bin/zsh
set -euo pipefail

HQ_STATE_BIN=${HQ_STATE_BIN:-hq-state}
exec "$HQ_STATE_BIN" "$@"
