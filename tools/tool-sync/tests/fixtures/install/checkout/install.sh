#!/bin/sh
set -eu
case "${1:-}" in
  apply|preview) ;;
  *) exit 64 ;;
esac
if [ -n "${TOOL_SYNC_RECORD:-}" ]; then
  printf '%s|%s\n' "$PWD" "$1" >> "$TOOL_SYNC_RECORD"
fi
if [ -n "${TOOL_SYNC_HOME_RECORD:-}" ]; then
  printf '%s|%s|%s|%s\n' "$HOME" "$CARGO_HOME" "$RUSTUP_HOME" "$(command -v cargo)" >> "$TOOL_SYNC_HOME_RECORD"
fi
