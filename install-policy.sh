#!/usr/bin/env bash
# install-policy.sh — pin the infrastructure settings in Claude Code's policy file, which
# outranks every writer of ~/.claude/settings.json. Needs sudo, so it stays out of the pull
# path install.sh runs from. see docs/reset-spec.md
# usage: ./install-policy.sh [--dry-run]
set -euo pipefail

REPO_TARGET="${REPO_TARGET:-$HOME/Documents/agents}"
SRC="$REPO_TARGET/config/managed-settings.json"
DEST="/Library/Application Support/ClaudeCode/managed-settings.json"
IS_DRY=0
[[ "${1:-}" == "--dry-run" ]] && IS_DRY=1

[[ -f "$SRC" ]] || { echo "FATAL: $SRC not found (set REPO_TARGET)" >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "FATAL: jq not found" >&2; exit 1; }

RENDERED="$(sed -e "s|\$REPO_TARGET|$REPO_TARGET|g" -e "s|\$HOME|$HOME|g" "$SRC")"
printf '%s' "$RENDERED" | jq empty \
  || { echo "FATAL: $SRC does not render to valid JSON" >&2; exit 1; }

if [[ -f "$DEST" ]] && [[ "$(printf '%s' "$RENDERED" | jq -S .)" == "$(jq -S . "$DEST" 2>/dev/null)" ]]; then
  echo "ok   $DEST is current"
  exit 0
fi

echo "plan: write $DEST"
if [[ -f "$DEST" ]]; then
  diff <(jq -S . "$DEST") <(printf '%s' "$RENDERED" | jq -S .) || true
else
  echo "      (no policy file yet; it will contain)"
  printf '%s\n' "$RENDERED" | sed 's/^/      /'
fi

if (( IS_DRY )); then
  echo "dry:  sudo install -m 644 -o root -g wheel <rendered> $DEST"
  exit 0
fi

TMP="$(mktemp)"
printf '%s\n' "$RENDERED" > "$TMP"
sudo mkdir -p "$(dirname "$DEST")"
sudo install -m 644 -o root -g wheel "$TMP" "$DEST"
rm -f "$TMP"
jq empty "$DEST" || { echo "FATAL: the installed policy is not valid JSON" >&2; exit 1; }
echo "ok   wrote $DEST"
echo "     to remove it: sudo rm '$DEST'"
