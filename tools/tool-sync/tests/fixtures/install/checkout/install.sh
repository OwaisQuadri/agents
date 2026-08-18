#!/bin/sh
set -eu
case "${1:-}" in
  apply|preview) ;;
  *) exit 64 ;;
esac
if [ -n "${TOOL_SYNC_RECORD:-}" ]; then
  printf '%s|%s\n' "$PWD" "$1" >> "$TOOL_SYNC_RECORD"
fi
