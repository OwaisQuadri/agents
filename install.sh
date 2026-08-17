#!/usr/bin/env bash
# install.sh — config reset installer. symlinks and one cargo build, never rm. see docs/reset-spec.md
# usage: ./install.sh [--dry-run]
set -euo pipefail
shopt -s nullglob

REPO_TARGET="${REPO_TARGET:-$HOME/Documents/agents}"
SKILLS_ROOT="$HOME/.agents/skills"
STAMP="$(date +%Y%m%d)"
IS_DRY=0
[[ "${1:-}" == "--dry-run" ]] && IS_DRY=1

plan() { echo "plan: $*"; }

run() {
  if (( IS_DRY )); then echo "dry:  $*"; else "$@"; fi
}

# move (or copy, for single files) an existing path to a .pre-reset-<stamp> backup, verified.
# backups of skills-root entries land in $SKILLS_ROOT.backups/ — a backup inside the live
# skills root surfaces in tool catalogs as a phantom skill
backup() {
  local src="$1" dest
  [[ -e "$src" || -L "$src" ]] || return 0
  if [[ "$(dirname "$src")" == "$SKILLS_ROOT" ]]; then
    run mkdir -p "$SKILLS_ROOT.backups"
    dest="$SKILLS_ROOT.backups/$(basename "$src").pre-reset-$STAMP"
  else
    dest="$src.pre-reset-$STAMP"
  fi
  [[ -e "$dest" || -L "$dest" ]] && dest="$dest.$(date +%H%M%S)"
  plan "backup $src -> $dest"
  if [[ -f "$src" && ! -L "$src" ]]; then
    run cp -p "$src" "$dest"
  else
    run mv "$src" "$dest"
  fi
  (( IS_DRY )) || [[ -e "$dest" || -L "$dest" ]] || { echo "FATAL: backup missing at $dest" >&2; exit 1; }
}

# link <linkpath> <target> — idempotent, pre-write backup
link() {
  local lnk="$1" target="$2"
  if [[ -L "$lnk" && "$(readlink "$lnk")" == "$target" ]]; then
    plan "ok   $lnk -> $target"
    return 0
  fi
  backup "$lnk"
  plan "link $lnk -> $target"
  run ln -sfn "$target" "$lnk"
}

[[ -d "$REPO_TARGET/skills" ]] || { echo "FATAL: $REPO_TARGET/skills not found (set REPO_TARGET)" >&2; exit 1; }
(( IS_DRY )) && plan "dry run — printing, not executing"

# 1. canonical skills root
plan "ensure $SKILLS_ROOT"
run mkdir -p "$SKILLS_ROOT"

# 2. one symlink per skill dir in the repo
for dir in "$REPO_TARGET/skills"/*/; do
  dir="${dir%/}"
  link "$SKILLS_ROOT/$(basename "$dir")" "$dir"
done

# 3. prune dangling symlinks into a holding dir (outside the skills root), never rm
PRUNED="$SKILLS_ROOT.backups/pruned-$STAMP"
for lnk in "$SKILLS_ROOT"/*; do
  [[ -L "$lnk" && ! -e "$lnk" ]] || continue
  plan "prune $lnk (target gone) -> $PRUNED/"
  run mkdir -p "$PRUNED"
  run mv "$lnk" "$PRUNED/"
  (( IS_DRY )) || [[ -L "$PRUNED/$(basename "$lnk")" ]] || { echo "FATAL: prune missing at $PRUNED/$(basename "$lnk")" >&2; exit 1; }
done

# 4. agent skill roots become single directory symlinks
plan "ensure $HOME/.claude $HOME/.codex"
run mkdir -p "$HOME/.claude" "$HOME/.codex"
link "$HOME/.claude/skills" "$SKILLS_ROOT"
link "$HOME/.codex/skills" "$SKILLS_ROOT"

# 5. global CLAUDE.md
link "$HOME/.claude/CLAUDE.md" "$REPO_TARGET/CLAUDE.md"

# 6. agents fleet: one directory symlink, definitions resolve from the repo
link "$HOME/.claude/agents" "$REPO_TARGET/agents"

# 7. self-installing pull hooks: a pull that changes the skill set re-runs this installer;
#    post-checkout carries the live checkout's uncommitted work into worktrees cut from main
if [[ -d "$REPO_TARGET/.git/hooks" ]]; then
  link "$REPO_TARGET/.git/hooks/post-merge" "$REPO_TARGET/install.sh"
  link "$REPO_TARGET/.git/hooks/post-rewrite" "$REPO_TARGET/install.sh"
  link "$REPO_TARGET/.git/hooks/post-checkout" "$REPO_TARGET/hooks/post-checkout"
fi

# 8. the rust tools. these are the artifacts the installer compiles rather than links,
#    because each one sits in a path that is waited on: ste-check in the reply path,
#    no-ai-attribution in the PreToolUse path ahead of every commit
BUILT_TOOLS=()
for tool in ste-check no-ai-attribution; do
  CRATE="$REPO_TARGET/tools/$tool"
  [[ -f "$CRATE/Cargo.toml" ]] || continue
  if command -v cargo >/dev/null 2>&1; then
    plan "build $CRATE (release)"
    run cargo build --release --quiet --manifest-path "$CRATE/Cargo.toml"
    plan "ensure $HOME/.local/bin"
    run mkdir -p "$HOME/.local/bin"
    link "$HOME/.local/bin/$tool" "$CRATE/target/release/$tool"
    BUILT_TOOLS+=("$tool")
  else
    echo "warn: cargo not found, skipping the $tool build" >&2
  fi
done

# 9. codex reads CLAUDE.md through this symlink: one source, no second file to drift
link "$HOME/.codex/AGENTS.md" "$REPO_TARGET/CLAUDE.md"

TOOL_SYNC_CRATE="$REPO_TARGET/tools/tool-sync"
TOOL_SYNC_BIN="$TOOL_SYNC_CRATE/target/release/tool-sync"
if [[ -f "$TOOL_SYNC_CRATE/Cargo.toml" ]]; then
  if command -v cargo >/dev/null 2>&1; then
    plan "build $TOOL_SYNC_CRATE (release)"
    run cargo build --release --quiet --manifest-path "$TOOL_SYNC_CRATE/Cargo.toml"
    plan "ensure $HOME/.local/bin"
    run mkdir -p "$HOME/.local/bin"
    link "$HOME/.local/bin/tool-sync" "$TOOL_SYNC_BIN"
    BUILT_TOOLS+=("tool-sync")
  else
    echo "warn: cargo not found, skipping the tool-sync build" >&2
  fi
fi

if [[ ! -x "$TOOL_SYNC_BIN" ]]; then
  echo "warn: tool-sync not built, skipping the tool sync" >&2
else
  TOOL_SYNC_ARGS=(
    --repository-root "$REPO_TARGET"
    --manifest "$REPO_TARGET/config/tools.toml"
    --home "$HOME"
  )
  (( IS_DRY )) && TOOL_SYNC_ARGS+=(--dry-run)
  plan "sync tools: $TOOL_SYNC_BIN ${TOOL_SYNC_ARGS[*]}"
  "$TOOL_SYNC_BIN" "${TOOL_SYNC_ARGS[@]}"
fi

# 10. mcp-sync: rebuilt every pass so the binary matches the manifest it syncs
CRATE="$REPO_TARGET/tools/mcp-sync"
if [[ -f "$CRATE/Cargo.toml" ]]; then
  if command -v cargo >/dev/null 2>&1; then
    plan "build $CRATE (release)"
    run cargo build --release --quiet --manifest-path "$CRATE/Cargo.toml"
    plan "ensure $HOME/.local/bin"
    run mkdir -p "$HOME/.local/bin"
    link "$HOME/.local/bin/mcp-sync" "$CRATE/target/release/mcp-sync"
    BUILT_TOOLS+=("mcp-sync")
  else
    echo "warn: cargo not found, skipping the mcp-sync build" >&2
  fi
fi

# 11. sync the live configs. the binary runs outside run() on purpose, so its own
#     --dry-run prints the real plan. no ~/.claude.json yet → skip; the next pull converges
SYNC_BIN="$HOME/.local/bin/mcp-sync"
run mkdir -p "$HOME/.codex/agents"
if [[ ! -x "$SYNC_BIN" ]]; then
  echo "warn: mcp-sync not built, skipping the config sync" >&2
elif [[ ! -f "$HOME/.claude.json" ]]; then
  echo "warn: $HOME/.claude.json not found yet, skipping the config sync" >&2
else
  plan "sync configs: MCP_SYNC_REPO=$REPO_TARGET $SYNC_BIN"
  if (( IS_DRY )); then
    MCP_SYNC_REPO="$REPO_TARGET" "$SYNC_BIN" --dry-run
  else
    MCP_SYNC_REPO="$REPO_TARGET" "$SYNC_BIN"
  fi
fi

# 12. status line. the script is repo-owned and linked like every other repo artifact, but
#     settings.json stays machine-local and only gains the pointer key: preferences and MCP
#     config are separate concerns, so mcp-sync does not reach into this file
SL_SRC="$REPO_TARGET/config/statusline.sh"
SETTINGS="$HOME/.claude/settings.json"
SL_LINK="$HOME/.claude/statusline.sh"
if [[ ! -f "$SL_SRC" ]]; then
  echo "warn: $SL_SRC not found, skipping the status line" >&2
elif ! command -v jq >/dev/null 2>&1; then
  echo "warn: jq not found, skipping the status line" >&2
else
  run chmod +x "$SL_SRC"
  link "$SL_LINK" "$SL_SRC"
  if [[ -f "$SETTINGS" ]] && jq -e --arg c "$SL_LINK" '.statusLine.command == $c' "$SETTINGS" >/dev/null 2>&1; then
    plan "ok   $SETTINGS statusLine -> $SL_LINK"
  else
    backup "$SETTINGS"
    plan "set  $SETTINGS statusLine -> $SL_LINK"
    if (( IS_DRY == 0 )); then
      CURRENT='{}'
      [[ -f "$SETTINGS" ]] && CURRENT="$(cat "$SETTINGS")"
      UPDATED="$(printf '%s' "$CURRENT" | jq --arg c "$SL_LINK" \
        '. + {statusLine: {type: "command", command: $c}}')" \
        || { echo "FATAL: $SETTINGS is not valid JSON" >&2; exit 1; }
      printf '%s\n' "$UPDATED" > "$SETTINGS"
    fi
  fi
fi

plan "done"
