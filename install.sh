#!/usr/bin/env bash
# install.sh — config reset installer. symlinks and one cargo build, never rm. see docs/reset-spec.md
# usage: ./install.sh [--dry-run] [--test]
# HOME_TARGET sandboxes every write below (default: the real $HOME). Point it at a scratch
# dir to test a worktree's config end to end without touching the real ~/.claude, ~/.codex,
# ~/.pi, ~/.local/bin, or ~/.zshrc — e.g. HOME_TARGET=/tmp/pi-sandbox ./install.sh
# --test is shorthand for that: it pins HOME_TARGET to a scratch dir inside THIS worktree
# (.install-test-home, gitignored), so the worktree tests only itself, every run starts
# from the same state, and nothing leaves the checkout.
set -euo pipefail
shopt -s nullglob

# git runs this through .git/hooks/post-merge, a symlink, so the link is resolved first;
# without that the repo root reads as .git/hooks and every path below misses.
SCRIPT_SELF="${BASH_SOURCE[0]}"
while [[ -L "$SCRIPT_SELF" ]]; do
  SCRIPT_LINK="$(readlink "$SCRIPT_SELF")"
  [[ "$SCRIPT_LINK" == /* ]] || SCRIPT_LINK="$(dirname "$SCRIPT_SELF")/$SCRIPT_LINK"
  SCRIPT_SELF="$SCRIPT_LINK"
done
SCRIPT_DIR="$(cd "$(dirname "$SCRIPT_SELF")" && pwd)"
REPO_TARGET="${REPO_TARGET:-$SCRIPT_DIR}"
HOME_TARGET="${HOME_TARGET:-$HOME}"
IS_DRY=0
IS_TEST=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) IS_DRY=1 ;;
    --test) IS_TEST=1; HOME_TARGET="$SCRIPT_DIR/.install-test-home" ;;
  esac
done
SKILLS_ROOT="$HOME_TARGET/.agents/skills"
STAMP="$(date +%Y%m%d)"

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
plan "ensure $HOME_TARGET/.claude $HOME_TARGET/.codex"
run mkdir -p "$HOME_TARGET/.claude" "$HOME_TARGET/.codex"
link "$HOME_TARGET/.claude/skills" "$SKILLS_ROOT"
link "$HOME_TARGET/.codex/skills" "$SKILLS_ROOT"

# 5. global CLAUDE.md
link "$HOME_TARGET/.claude/CLAUDE.md" "$REPO_TARGET/CLAUDE.md"
link "$HOME_TARGET/.claude/rules" "$REPO_TARGET/rules"

# 6. agents fleet: one directory symlink, definitions resolve from the repo
link "$HOME_TARGET/.claude/agents" "$REPO_TARGET/agents"

# 6b/6c. ONE agent definition per role, in agents/<name>/<name>.md; config/model-tiers.json
#     assigns it a tier. Everything below is derived from those two, so a generated file
#     is never hand-edited.
TIERS="$REPO_TARGET/config/model-tiers.json"
PI_AGENTS="$HOME_TARGET/.pi/agent/agents"
PI_SETTINGS_TIERS="$HOME_TARGET/.pi/agent/settings.json"
run mkdir -p "$HOME_TARGET/.pi/agent" "$PI_AGENTS"

if ! command -v jq >/dev/null 2>&1; then
  echo "warn: jq not found, skipping agent generation and the model-tier sync" >&2
elif [[ ! -f "$TIERS" ]]; then
  echo "warn: $TIERS not found, skipping agent generation and the model-tier sync" >&2
else
  # Claude Code reaches Anthropic only, so a tier resolves there by walking its chain for
  # the first Anthropic model, and climbing tiers when a chain holds none.
  CLAUDE_ALIAS_JQ='def anthropic_in(t): [t.pi] + t.fallbacks | map(select(startswith("anthropic/"))) | first;
    . as $r
    | ($r.tiers | keys | sort) as $order
    | .agents | to_entries[]
    | .key as $name | .value as $tier
    | ($order | index($tier)) as $at
    | [ $order[$at:][] | $r.tiers[.] | anthropic_in(.) | select(. != null) ] | first
    | select(. != null)
    | sub("^anthropic/claude-";"") | sub("-.*$";"") as $alias
    | [$name, $alias] | @tsv'

  while IFS=$'\t' read -r name alias; do
    src="$REPO_TARGET/agents/$name/$name.md"
    [[ -f "$src" ]] || continue
    if grep -q "^model: $alias$" "$src"; then
      plan "ok   $src model -> $alias"
    else
      plan "set  $src model -> $alias (derived from $TIERS)"
      run sed -i '' "1,12 s/^model: .*$/model: $alias/" "$src"
    fi
  done < <(jq -r "$CLAUDE_ALIAS_JQ" "$TIERS" 2>/dev/null)

  # No model line: pi resolves the tier through agentOverrides. An unmapped tool aborts
  # rather than silently dropping a capability grant.
  for src in "$REPO_TARGET"/agents/*/*.md; do
    name="$(basename "$src" .md)"
    [[ "$(jq -r --arg n "$name" '.agents[$n] // empty' "$TIERS")" != "" ]] || continue
    dest="$PI_AGENTS/$name.md"
    gen="$(awk '
      function map(t) {
        if (t == "Read") return "read"; if (t == "Write") return "write";
        if (t == "Edit") return "edit"; if (t == "Bash") return "bash";
        if (t == "Grep") return "grep"; if (t == "Glob") return "find";
        if (t == "WebSearch") return "web_search"; if (t == "WebFetch") return "fetch_content";
        print "UNMAPPED_TOOL:" t > "/dev/stderr"; exit 3
      }
      NR == 1 && $0 == "---" { print; infm = 1; next }
      infm && /^---$/ { print "tools:"; for (i = 1; i <= n; i++) print "  - " tools[i]; print; infm = 0; next }
      infm && /^name:/ { sub(/^name: */, ""); printf "name: \"%s\"\n", $0; next }
      infm && /^description:/ { sub(/^description: */, ""); printf "description: \"%s\"\n", $0; next }
      infm && /^model:/ { next }
      infm && /^tools:/ { sub(/^tools: */, ""); n = split($0, a, /, */); for (i = 1; i <= n; i++) tools[i] = map(a[i]); next }
      infm { next }
      { print }
    ' "$src")" || { echo "FATAL: unmapped tool in $src" >&2; exit 1; }
    if [[ -f "$dest" ]] && [[ "$gen" == "$(cat "$dest")" ]]; then
      plan "ok   $dest generated from $src"
    else
      plan "gen  $dest from $src"
      (( IS_DRY )) || printf '%s\n' "$gen" > "$dest"
    fi
  done

  [[ -f "$PI_SETTINGS_TIERS" ]] || { plan "init $PI_SETTINGS_TIERS"; run bash -c "echo '{}' > '$PI_SETTINGS_TIERS'"; }
  # agentOverrides.thinking OVERWRITES an agent's own frontmatter (agents.ts applyOverride),
  # so the tier's thinking is emitted only for agents this repo owns. A package agent that
  # tuned its own thinking keeps it, and only its model follows the tier.
  OWNED="$(cd "$REPO_TARGET/agents" 2>/dev/null && ls -d */ 2>/dev/null | tr -d / | jq -R . | jq -sc .)"
  OWNED="${OWNED:-[]}"
  # modelTierFallbacks takes each tier's FIRST backup: the session extension walks one hop
  # per usage limit and re-enters on the new model, so the rest of the chain is reached by
  # repetition rather than by a list. Subagents get the whole ordered list.
  TIER_JQ='.modelTierFallbacks = ($t.tiers | with_entries(
      .value as $tier | { key: $tier.pi, value: $tier.fallbacks[0] }))
    | .subagents = ((.subagents // {})
    | .defaultModel = $t.tiers[$t.orchestrator].pi
    | .defaultThinking = $t.tiers[$t.orchestrator].thinking
    | .agentOverrides = ((.agentOverrides // {}) + ($t.agents | with_entries(
        .key as $name | .value as $tier
        | .value = { model: $t.tiers[$tier].pi,
                     fallbackModels: $t.tiers[$tier].fallbacks }
                   + (if ($owned | index($name)) then { thinking: $t.tiers[$tier].thinking } else {} end)))))'
  if [[ -f "$PI_SETTINGS_TIERS" ]] && jq -e --argjson t "$(cat "$TIERS")" --argjson owned "$OWNED" \
      ". == ($TIER_JQ)" "$PI_SETTINGS_TIERS" >/dev/null 2>&1; then
    plan "ok   $PI_SETTINGS_TIERS subagent routing matches $TIERS"
  else
    plan "set  $PI_SETTINGS_TIERS subagent routing from $TIERS"
    run bash -c "jq --argjson t \"\$(cat '$TIERS')\" --argjson owned '$OWNED' '$TIER_JQ' '$PI_SETTINGS_TIERS' \
      > '$PI_SETTINGS_TIERS.tmp' && mv '$PI_SETTINGS_TIERS.tmp' '$PI_SETTINGS_TIERS'"
  fi
fi

# 7. self-installing pull hooks: a pull that changes the skill set re-runs this installer;
#    post-checkout carries the live checkout's uncommitted work into worktrees cut from main
if [[ -d "$REPO_TARGET/.git/hooks" ]]; then
  link "$REPO_TARGET/.git/hooks/post-merge" "$REPO_TARGET/install.sh"
  link "$REPO_TARGET/.git/hooks/post-rewrite" "$REPO_TARGET/install.sh"
  link "$REPO_TARGET/.git/hooks/post-checkout" "$REPO_TARGET/hooks/post-checkout"
fi

# 8. the rust tools. these are the artifacts the installer compiles rather than links,
#    because each one sits in a path that is waited on: ste-check in the reply path,
#    no-ai-attribution in the PreToolUse path ahead of every commit,
#    session-stats as an on-demand command the user runs by name
for tool in ste-check no-ai-attribution session-stats; do
  CRATE="$REPO_TARGET/tools/$tool"
  [[ -f "$CRATE/Cargo.toml" ]] || continue
  if command -v cargo >/dev/null 2>&1; then
    plan "build $CRATE (release)"
    run cargo build --release --quiet --manifest-path "$CRATE/Cargo.toml"
    plan "ensure $HOME_TARGET/.local/bin"
    run mkdir -p "$HOME_TARGET/.local/bin"
    link "$HOME_TARGET/.local/bin/$tool" "$CRATE/target/release/$tool"
  else
    echo "warn: cargo not found, skipping the $tool build" >&2
  fi
done

# 9. codex reads CLAUDE.md through this symlink: one source, no second file to drift
link "$HOME_TARGET/.codex/AGENTS.md" "$REPO_TARGET/CLAUDE.md"

TOOL_SYNC_CRATE="$REPO_TARGET/tools/tool-sync"
TOOL_SYNC_BIN="$TOOL_SYNC_CRATE/target/release/tool-sync"
if [[ -f "$TOOL_SYNC_CRATE/Cargo.toml" ]]; then
  if command -v cargo >/dev/null 2>&1; then
    plan "build $TOOL_SYNC_CRATE (release)"
    run cargo build --release --quiet --manifest-path "$TOOL_SYNC_CRATE/Cargo.toml"
    plan "ensure $HOME_TARGET/.local/bin"
    run mkdir -p "$HOME_TARGET/.local/bin"
    link "$HOME_TARGET/.local/bin/tool-sync" "$TOOL_SYNC_BIN"
  else
    echo "warn: cargo not found, skipping the tool-sync build" >&2
  fi
fi

if [[ -f "$TOOL_SYNC_CRATE/Cargo.toml" && ! -x "$TOOL_SYNC_BIN" ]]; then
  echo "FATAL: tool-sync is not built; run: cargo build --release --manifest-path $TOOL_SYNC_CRATE/Cargo.toml" >&2
  exit 1
elif [[ -x "$TOOL_SYNC_BIN" ]]; then
  TOOL_SYNC_ARGS=(
    --repository-root "$REPO_TARGET"
    --manifest "$REPO_TARGET/config/tools.toml"
    --home "$HOME_TARGET"
  )
  (( IS_DRY )) && TOOL_SYNC_ARGS+=(--dry-run)
  plan "sync tools: $TOOL_SYNC_BIN ${TOOL_SYNC_ARGS[*]}"
  "$TOOL_SYNC_BIN" "${TOOL_SYNC_ARGS[@]}"
fi

CRATE="$REPO_TARGET/tools/pr-review-filter"
if [[ -f "$CRATE/Cargo.toml" ]]; then
  if command -v cargo >/dev/null 2>&1; then
    plan "build $CRATE (release)"
    run cargo build --release --quiet --manifest-path "$CRATE/Cargo.toml"
    plan "ensure $HOME_TARGET/.local/bin"
    run mkdir -p "$HOME_TARGET/.local/bin"
    link "$HOME_TARGET/.local/bin/pr-review-filter" "$CRATE/target/release/pr-review-filter"
  else
    echo "warn: cargo not found, skipping the pr-review-filter build" >&2
  fi
fi

CRATE="$REPO_TARGET/tools/tool-wizard"
if [[ -f "$CRATE/Cargo.toml" ]]; then
  if command -v cargo >/dev/null 2>&1; then
    plan "build $CRATE (release)"
    run cargo build --release --quiet --manifest-path "$CRATE/Cargo.toml"
    plan "ensure $HOME_TARGET/.local/bin"
    run mkdir -p "$HOME_TARGET/.local/bin"
    link "$HOME_TARGET/.local/bin/tool-wizard" "$CRATE/target/release/tool-wizard"
  else
    echo "warn: cargo not found, skipping the tool-wizard build" >&2
  fi
fi

# 10. mcp-sync: rebuilt every pass so the binary matches the manifest it syncs
CRATE="$REPO_TARGET/tools/mcp-sync"
if [[ -f "$CRATE/Cargo.toml" ]]; then
  if command -v cargo >/dev/null 2>&1; then
    plan "build $CRATE (release)"
    run cargo build --release --quiet --manifest-path "$CRATE/Cargo.toml"
    plan "ensure $HOME_TARGET/.local/bin"
    run mkdir -p "$HOME_TARGET/.local/bin"
    link "$HOME_TARGET/.local/bin/mcp-sync" "$CRATE/target/release/mcp-sync"
  else
    echo "warn: cargo not found, skipping the mcp-sync build" >&2
  fi
fi

# 10b. usage-limit-watch: reads a plain-text log for Codex/Claude usage-limit phrasing,
#      secondary to the pi_extension in pi/extensions/usage-limit-continue.ts (that one
#      covers Pi itself via the extension API; this covers headless codex/claude runs
#      whose output is redirected to a file).
CRATE="$REPO_TARGET/tools/usage-limit-watch"
if [[ -f "$CRATE/Cargo.toml" ]]; then
  if command -v cargo >/dev/null 2>&1; then
    plan "build $CRATE (release)"
    run cargo build --release --quiet --manifest-path "$CRATE/Cargo.toml"
    plan "ensure $HOME/.local/bin"
    run mkdir -p "$HOME/.local/bin"
    link "$HOME/.local/bin/usage-limit-watch" "$CRATE/target/release/usage-limit-watch"
  else
    echo "warn: cargo not found, skipping the usage-limit-watch build" >&2
  fi
fi

# 11. sync the live configs. the binary runs outside run() on purpose, so its own
#     --dry-run prints the real plan. no ~/.claude.json yet → skip; the next pull converges
SYNC_BIN="$HOME_TARGET/.local/bin/mcp-sync"
run mkdir -p "$HOME_TARGET/.codex/agents"
if [[ ! -x "$SYNC_BIN" ]]; then
  echo "warn: mcp-sync not built, skipping the config sync" >&2
elif [[ ! -f "$HOME_TARGET/.claude.json" ]]; then
  echo "warn: $HOME_TARGET/.claude.json not found yet, skipping the config sync" >&2
else
  plan "sync configs: MCP_SYNC_REPO=$REPO_TARGET $SYNC_BIN"
  if (( IS_DRY )); then
    MCP_SYNC_REPO="$REPO_TARGET" "$SYNC_BIN" --dry-run
  else
    MCP_SYNC_REPO="$REPO_TARGET" "$SYNC_BIN"
  fi
fi

# 12. shell preferences. Claude Code and Pi need explicit settings. Codex reads the
#     user's login shell, and CLAUDE.md tells every client to keep Z shell syntax.
ZSH_PATH="$(command -v zsh || true)"
if [[ -z "$ZSH_PATH" ]]; then
  echo "warn: zsh not found, skipping agent shell preferences" >&2
elif ! command -v jq >/dev/null 2>&1; then
  echo "warn: jq not found, skipping agent shell preferences" >&2
else
  CLAUDE_SETTINGS="$HOME_TARGET/.claude/settings.json"
  PI_SETTINGS="$HOME_TARGET/.pi/agent/settings.json"
  run mkdir -p "$HOME_TARGET/.pi/agent"

  if [[ -f "$CLAUDE_SETTINGS" ]] && jq -e --arg s "$ZSH_PATH" '.env.CLAUDE_CODE_SHELL == $s' "$CLAUDE_SETTINGS" >/dev/null 2>&1; then
    plan "ok   $CLAUDE_SETTINGS shell -> $ZSH_PATH"
  else
    backup "$CLAUDE_SETTINGS"
    plan "set  $CLAUDE_SETTINGS shell -> $ZSH_PATH"
    if (( IS_DRY == 0 )); then
      CURRENT='{}'
      [[ -f "$CLAUDE_SETTINGS" ]] && CURRENT="$(cat "$CLAUDE_SETTINGS")"
      UPDATED="$(printf '%s' "$CURRENT" | jq --arg s "$ZSH_PATH" \
        '.env = ((.env // {}) + {CLAUDE_CODE_SHELL: $s})')" \
        || { echo "FATAL: $CLAUDE_SETTINGS is not valid JSON" >&2; exit 1; }
      printf '%s\n' "$UPDATED" > "$CLAUDE_SETTINGS"
    fi
  fi

  # shellPath is the shell preference; warnings.anthropicExtraUsage=false suppresses pi's
  # Anthropic extra-usage billing warning on subscription auth.
  if [[ -f "$PI_SETTINGS" ]] && jq -e --arg s "$ZSH_PATH" \
    '.shellPath == $s and .warnings.anthropicExtraUsage == false' "$PI_SETTINGS" >/dev/null 2>&1; then
    plan "ok   $PI_SETTINGS shell -> $ZSH_PATH, anthropic warning off"
  else
    backup "$PI_SETTINGS"
    plan "set  $PI_SETTINGS shell -> $ZSH_PATH, anthropic warning off"
    if (( IS_DRY == 0 )); then
      CURRENT='{}'
      [[ -f "$PI_SETTINGS" ]] && CURRENT="$(cat "$PI_SETTINGS")"
      UPDATED="$(printf '%s' "$CURRENT" | jq --arg s "$ZSH_PATH" \
        '.shellPath = $s | .warnings.anthropicExtraUsage = false')" \
        || { echo "FATAL: $PI_SETTINGS is not valid JSON" >&2; exit 1; }
      printf '%s\n' "$UPDATED" > "$PI_SETTINGS"
    fi
  fi
fi

# web_search curator: never open the browser. workflow=none skips the curator entirely and
# returns raw results; autoOpenBrowser=false keeps the window shut even if the curator runs.
# Config lives in its own file, separate from ~/.pi/agent/settings.json.
if ! command -v jq >/dev/null 2>&1; then
  echo "warn: jq not found, skipping web_search curator preference" >&2
else
  PI_WEBSEARCH="$HOME_TARGET/.pi/web-search.json"
  run mkdir -p "$HOME_TARGET/.pi"
  if [[ -f "$PI_WEBSEARCH" ]] && jq -e '.workflow == "none" and .autoOpenBrowser == false' "$PI_WEBSEARCH" >/dev/null 2>&1; then
    plan "ok   $PI_WEBSEARCH curator off"
  else
    backup "$PI_WEBSEARCH"
    plan "set  $PI_WEBSEARCH curator off"
    if (( IS_DRY == 0 )); then
      CURRENT='{}'
      [[ -f "$PI_WEBSEARCH" ]] && CURRENT="$(cat "$PI_WEBSEARCH")"
      UPDATED="$(printf '%s' "$CURRENT" | jq '.workflow = "none" | .autoOpenBrowser = false')" \
        || { echo "FATAL: $PI_WEBSEARCH is not valid JSON" >&2; exit 1; }
      printf '%s\n' "$UPDATED" > "$PI_WEBSEARCH"
    fi
  fi
fi

# 13. status line. the script is repo-owned and linked like every other repo artifact, but
#     settings.json stays machine-local. Client preferences and MCP config are separate
#     concerns, so mcp-sync does not reach into this file.
SL_SRC="$REPO_TARGET/config/statusline.sh"
SETTINGS="$HOME_TARGET/.claude/settings.json"
SL_LINK="$HOME_TARGET/.claude/statusline.sh"
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

# 14. cld: claude with permission prompts off, reaching every shell. the .zshrc alias
#     covers interactive sessions; the ~/.local/bin shim covers scripts, git hooks, and
#     an agent's own Bash tool, none of which source .zshrc. where both apply zsh expands
#     the alias first. AGNT-0009 replaces the block with the rendered managed shell file.
CLD_SRC="$REPO_TARGET/config/cld"
if [[ -f "$CLD_SRC" ]]; then
  run chmod +x "$CLD_SRC"
  plan "ensure $HOME_TARGET/.local/bin"
  run mkdir -p "$HOME_TARGET/.local/bin"
  link "$HOME_TARGET/.local/bin/cld" "$CLD_SRC"
fi

ZSHRC="$HOME_TARGET/.zshrc"
CLD_BEGIN="# >>> agents managed (cld) >>>"
CLD_END="# <<< agents managed (cld) <<<"
CLD_BLOCK="$CLD_BEGIN
alias cld='claude --dangerously-skip-permissions'
$CLD_END"
CURRENT=""
[[ -f "$ZSHRC" ]] && CURRENT="$(cat "$ZSHRC")"
# drop any previous block, then re-append: an edit to the alias lands on the next run
STRIPPED="$(printf '%s\n' "$CURRENT" | awk -v b="$CLD_BEGIN" -v e="$CLD_END" '
  $0 == b { skip = 1 }
  !skip   { print }
  $0 == e { skip = 0 }' | sed -e :a -e '/^$/{$d;N;ba' -e '}')"
DESIRED="$STRIPPED

$CLD_BLOCK"
if [[ "$CURRENT" == "$DESIRED" ]]; then
  plan "ok   $ZSHRC alias cld"
else
  backup "$ZSHRC"
  plan "set  $ZSHRC alias cld -> claude --dangerously-skip-permissions"
  (( IS_DRY )) || printf '%s\n' "$DESIRED" > "$ZSHRC"
fi

# 15. prune the Codex import residue. the ChatGPT app mirrors ~/.codex/config.toml's
#     [marketplaces.*] tables into the Claude and Pi settings, but its source is a flat
#     string where Claude Code wants a tagged union, so every mirrored entry fails schema
#     validation. drop any marketplace whose source carries no known tag, then drop the
#     plugin keys left pointing at a marketplace no longer registered. re-runs after each
#     import, so the settings converge instead of drifting.
CLAUDE_SETTINGS="$HOME_TARGET/.claude/settings.json"
PI_SETTINGS="$HOME_TARGET/.pi/agent/settings.json"
KNOWN_MARKETPLACES="$HOME_TARGET/.claude/plugins/known_marketplaces.json"
PRUNE_JQ='
def tags: ["github","git","directory","url","npm","archive","command"];
def ok_source: (.source | objects | .source | strings) as $t
  | ($t != null and (tags | index($t) != null));
(if has("extraKnownMarketplaces")
   then .extraKnownMarketplaces |= with_entries(select(.value | ok_source))
   else . end)
| (((.extraKnownMarketplaces // {}) | keys) + ($known | keys)) as $mp
| (if has("enabledPlugins")
     then .enabledPlugins |= with_entries(
       (.key | split("@")) as $p
       | select(($p | length) == 2 and ($mp | index($p[1]) != null)))
     else . end)
| (if has("plugins")
     then .plugins |= with_entries(select(.key | test("@")))
     else . end)
'
if ! command -v jq >/dev/null 2>&1; then
  echo "warn: jq not found, skipping the settings prune" >&2
else
  KNOWN='{}'
  [[ -f "$KNOWN_MARKETPLACES" ]] && KNOWN="$(cat "$KNOWN_MARKETPLACES")"
  for target in "$CLAUDE_SETTINGS" "$PI_SETTINGS"; do
    [[ -f "$target" ]] || continue
    PRUNED="$(jq --argjson known "$KNOWN" "$PRUNE_JQ" "$target")" \
      || { echo "FATAL: $target is not valid JSON" >&2; exit 1; }
    if [[ "$(printf '%s' "$PRUNED" | jq -S .)" == "$(jq -S . "$target")" ]]; then
      plan "ok   $target has no import residue"
      continue
    fi
    backup "$target"
    plan "prune $target (malformed marketplaces, unresolvable plugin keys)"
    (( IS_DRY )) || printf '%s\n' "$PRUNED" > "$target"
  done
fi

# 16. workspace trust for this repo's own worktrees. Claude Code skips the status line
#     AND every hook in a directory whose trust dialog was never accepted, so a worktree
#     silently loses rag-recall and the attribution guard with no error. Trust follows the
#     repo: only git worktrees of REPO_TARGET, and only those under $HOME, so a throwaway
#     checkout under /tmp never becomes a trusted directory.
CLAUDE_JSON="$HOME_TARGET/.claude.json"
if ! command -v jq >/dev/null 2>&1; then
  echo "warn: jq not found, skipping the worktree trust sync" >&2
elif [[ ! -f "$CLAUDE_JSON" ]]; then
  echo "warn: $CLAUDE_JSON not found yet, skipping the worktree trust sync" >&2
else
  TRUST_PATHS="$(git -C "$REPO_TARGET" worktree list --porcelain 2>/dev/null \
    | awk '/^worktree /{print $2}' | grep "^$HOME/" | grep -v '^$' || true)"
  if [[ -z "$TRUST_PATHS" ]]; then
    plan "ok   no in-home worktrees to trust"
  else
    TRUST_JSON="$(printf '%s\n' "$TRUST_PATHS" | jq -R . | jq -s .)"
    TRUSTED="$(jq --argjson paths "$TRUST_JSON" \
      'reduce $paths[] as $p (.; .projects[$p].hasTrustDialogAccepted = true)' "$CLAUDE_JSON")" \
      || { echo "FATAL: $CLAUDE_JSON is not valid JSON" >&2; exit 1; }
    if [[ "$(printf '%s' "$TRUSTED" | jq -S .)" == "$(jq -S . "$CLAUDE_JSON")" ]]; then
      plan "ok   every in-home worktree is already trusted"
    else
      printf '%s\n' "$TRUST_PATHS" | while IFS= read -r wt; do
        jq -e --arg p "$wt" '.projects[$p].hasTrustDialogAccepted == true' "$CLAUDE_JSON" >/dev/null 2>&1 \
          || plan "trust $wt"
      done
      backup "$CLAUDE_JSON"
      (( IS_DRY )) || printf '%s\n' "$TRUSTED" > "$CLAUDE_JSON"
    fi
  fi
fi

# 17. policy drift check, never a write. The policy file outranks every writer of the user
#     settings, which is the point of it, but it lives under /Library and needs sudo, and
#     post-merge runs this installer from a git hook with no terminal to prompt on. So this
#     reports drift and install-policy.sh owns the escalation.
POLICY_SRC="$REPO_TARGET/config/managed-settings.json"
POLICY_DEST="/Library/Application Support/ClaudeCode/managed-settings.json"
if [[ "$HOME_TARGET" != "$HOME" ]]; then
  # the policy file is per-machine and rendered against the canonical checkout; a
  # sandboxed run has no machine state to check and its paths never match
  plan "skip policy drift check (sandboxed HOME_TARGET)"
elif [[ -f "$POLICY_SRC" ]] && command -v jq >/dev/null 2>&1; then
  POLICY_RENDERED="$(sed -e "s|\$REPO_TARGET|$REPO_TARGET|g" -e "s|\$HOME|$HOME|g" "$POLICY_SRC")"
  if [[ -f "$POLICY_DEST" ]] \
    && [[ "$(printf '%s' "$POLICY_RENDERED" | jq -S .)" == "$(jq -S . "$POLICY_DEST" 2>/dev/null)" ]]; then
    plan "ok   $POLICY_DEST is current"
  else
    echo "warn: the Claude Code policy file is missing or stale; run: $REPO_TARGET/install-policy.sh" >&2
  fi
fi

plan "done"

# 18. --test drops into pi against the sandbox home so the run is inspectable right away.
#     The auth symlink borrows the real credential instead of copying it: the sandbox never
#     holds its own token, and deleting .install-test-home deletes only the link.
if (( IS_TEST )) && ! (( IS_DRY )); then
  if [[ -f "$HOME/.pi/agent/auth.json" ]]; then
    link "$HOME_TARGET/.pi/agent/auth.json" "$HOME/.pi/agent/auth.json"
  fi
  plan "launch pi with HOME=$HOME_TARGET"
  exec env HOME="$HOME_TARGET" pi
fi
