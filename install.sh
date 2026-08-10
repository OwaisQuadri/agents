#!/usr/bin/env bash
# install.sh — config reset installer. symlinks and one cargo build, never rm. see docs/reset-spec.md
# usage: ./install.sh [--dry-run]
set -euo pipefail
shopt -s nullglob

REPO_TARGET="${REPO_TARGET:-$HOME/Documents/agents}"
SKILLS_ROOT="$HOME/.agents/skills"
STAMP="$(date +%Y%m%d)"
DRY=0
[[ "${1:-}" == "--dry-run" ]] && DRY=1

plan() { echo "plan: $*"; }

run() {
  if (( DRY )); then echo "dry:  $*"; else "$@"; fi
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
  (( DRY )) || [[ -e "$dest" || -L "$dest" ]] || { echo "FATAL: backup missing at $dest" >&2; exit 1; }
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
(( DRY )) && plan "dry run — printing, not executing"

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
  (( DRY )) || [[ -L "$PRUNED/$(basename "$lnk")" ]] || { echo "FATAL: prune missing at $PRUNED/$(basename "$lnk")" >&2; exit 1; }
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

# 8. ste-check: the prose checker every register runs. this is the one artifact the
#    installer compiles rather than links, because a checker in the reply path is waited on
CRATE="$REPO_TARGET/tools/ste-check"
if [[ -f "$CRATE/Cargo.toml" ]]; then
  if command -v cargo >/dev/null 2>&1; then
    plan "build $CRATE (release)"
    run cargo build --release --quiet --manifest-path "$CRATE/Cargo.toml"
    plan "ensure $HOME/.local/bin"
    run mkdir -p "$HOME/.local/bin"
    link "$HOME/.local/bin/ste-check" "$CRATE/target/release/ste-check"
  else
    echo "warn: cargo not found, skipping the ste-check build" >&2
  fi
fi

plan "done"
