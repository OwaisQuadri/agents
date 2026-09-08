#!/usr/bin/env bash
# install.sh — config reset installer. symlinks and one cargo build, never rm. see docs/reset-spec.md
# usage: ./install.sh [--dry-run] [--test]
# HOME_TARGET sandboxes every write below (default: the real $HOME). Point it at a scratch
# dir to test a worktree's config end to end without touching the real ~/.pi, ~/.local/bin, or ~/.zshrc — e.g. HOME_TARGET=/tmp/pi-sandbox ./install.sh
# --test is shorthand for that: it pins HOME_TARGET to a scratch dir inside THIS worktree
# (.install-test-home, gitignored), so the worktree tests only itself, every run starts
# from the same state, and nothing leaves the checkout. --test runs the install steps and
# returns; test/run drops into an interactive pi against the sandbox, and test/build_run
# chains both.
set -euo pipefail
shopt -s nullglob

# git runs this through .git/hooks/post-merge, a symlink, so the link is resolved first;
# without that the repo root reads as .git/hooks and every path below misses.
SCRIPT_SELF="${BASH_SOURCE[0]}"
IS_GIT_HOOK=0
case "${SCRIPT_SELF##*/}" in
  post-merge|post-rewrite) IS_GIT_HOOK=1 ;;
esac
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
    --test) IS_TEST=1; REPO_TARGET="$SCRIPT_DIR"; HOME_TARGET="$SCRIPT_DIR/.install-test-home" ;;
  esac
done
SIMSLIM_PROFILE="$REPO_TARGET/config/simslim/main.json"
if [[ -f "$SIMSLIM_PROFILE" ]]; then
  if ! command -v jq >/dev/null 2>&1; then
    echo "FATAL: jq is required to validate $SIMSLIM_PROFILE." >&2
    exit 1
  fi
  if ! jq -e '
    type == "object" and
    keys == ["description", "except", "keep", "name"] and
    .name == "main" and
    (.description | type == "string" and length > 0) and
    .except == [] and
    .keep == []
  ' "$SIMSLIM_PROFILE" >/dev/null; then
    echo "FATAL: $SIMSLIM_PROFILE is not the maximum-isolation SimSlim profile." >&2
    exit 1
  fi
fi
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

# link_config <source> <destination> <description> — symlinks a single managed config
# file, skipping with a warning when the source is absent instead of failing the run.
link_config() {
  local src="$1" dest="$2" description="$3"
  if [[ ! -f "$src" ]]; then
    echo "warn: $src not found, skipping $description" >&2
  else
    run mkdir -p "$(dirname "$dest")"
    link "$dest" "$src"
  fi
}

# build_tool <crate-dir> <name> — cargo release build plus a ~/.local/bin symlink,
# skipped with a warning when cargo is absent, a no-op when the crate is absent.
build_tool() {
  local crate="$1" name="$2"
  [[ -f "$crate/Cargo.toml" ]] || return 0
  if ! command -v cargo >/dev/null 2>&1; then
    echo "warn: cargo not found, skipping the $name build" >&2
    return 0
  fi
  plan "build $crate (release)"
  run cargo build --release --quiet --manifest-path "$crate/Cargo.toml"
  plan "ensure $HOME_TARGET/.local/bin"
  run mkdir -p "$HOME_TARGET/.local/bin"
  link "$HOME_TARGET/.local/bin/$name" "$crate/target/release/$name"
}

# json_update <file> <jq-args...> — rewrite a JSON file ('{}' when absent) through a jq
# filter. A no-op under --dry-run, like the inline writes it replaces.
json_update() {
  local target="$1" current='{}' updated
  shift
  (( IS_DRY )) && return 0
  [[ -f "$target" ]] && current="$(cat "$target")"
  updated="$(printf '%s' "$current" | jq "$@")" \
    || { echo "FATAL: $target is not valid JSON" >&2; exit 1; }
  printf '%s\n' "$updated" > "$target"
}

retire_pi_extension() {
  local src="$1" retired_root dest
  [[ -e "$src" || -L "$src" ]] || return 0
  retired_root="$HOME_TARGET/.pi/agent/retired-extensions"
  dest="$retired_root/$(basename "$src").retired-$STAMP"
  [[ -e "$dest" || -L "$dest" ]] && dest="$dest.$(date +%H%M%S)"
  run mkdir -p "$retired_root"
  plan "retire $src -> $dest"
  run mv "$src" "$dest"
  (( IS_DRY )) || [[ -e "$dest" || -L "$dest" ]] || { echo "FATAL: retired Pi extension missing at $dest" >&2; exit 1; }
}

retire_to_trash() {
  local src="$1" dest expected_count actual_count
  [[ -e "$src" || -L "$src" ]] || return 0
  expected_count="$(find "$src" -xdev -print | wc -l | tr -d " ")"
  dest="$HOME_TARGET/.Trash/$(basename "$src").retired-$STAMP"
  [[ -e "$dest" || -L "$dest" ]] && dest="$dest.$(date +%H%M%S)"
  run mkdir -p "$HOME_TARGET/.Trash"
  plan "retire $src -> $dest"
  run mv "$src" "$dest"
  (( IS_DRY )) && return 0
  actual_count="$(find "$dest" -xdev -print | wc -l | tr -d " ")"
  [[ "$actual_count" == "$expected_count" ]] || { echo "FATAL: retired path count mismatch for $src" >&2; exit 1; }
}

[[ -d "$REPO_TARGET/skills" ]] || { echo "FATAL: $REPO_TARGET/skills not found (set REPO_TARGET)" >&2; exit 1; }
(( IS_DRY )) && plan "dry run — printing, not executing"

if [[ "$(uname -s)" == "Darwin" ]]; then
  if (( IS_GIT_HOOK )); then
    plan "skip SimSlim installation (git hook)"
  elif (( IS_DRY )); then
    plan "install or upgrade SimSlim through Homebrew"
  elif (( IS_TEST )) || [[ "$HOME_TARGET" != "$HOME" ]]; then
    plan "skip SimSlim installation (sandbox install)"
  elif ! command -v brew >/dev/null 2>&1; then
    echo "warn: Homebrew not found, skipping SimSlim installation" >&2
  elif brew list --formula mobai-app/tap/simslim >/dev/null 2>&1; then
    plan "upgrade SimSlim through Homebrew"
    brew upgrade mobai-app/tap/simslim || echo "warn: SimSlim upgrade failed" >&2
  else
    plan "install SimSlim through Homebrew"
    brew install mobai-app/tap/simslim || echo "warn: SimSlim installation failed" >&2
  fi
else
  plan "skip SimSlim installation (macOS only)"
fi

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

# 4. Pi provider routing and model configuration.
TIERS="$REPO_TARGET/config/model-tiers.json"
PI_SETTINGS_TIERS="$HOME_TARGET/.pi/agent/settings.json"
PI_MODELS_CONFIG="$HOME_TARGET/.pi/agent/models.json"
PI_MODELS_SOURCE="$REPO_TARGET/config/models.json"
run mkdir -p "$HOME_TARGET/.pi/agent"

if ! command -v jq >/dev/null 2>&1; then
  echo "warn: jq not found, skipping model overrides and provider routing" >&2
elif [[ ! -f "$TIERS" ]]; then
  echo "warn: $TIERS not found, skipping provider routing" >&2
else
  if [[ ! -f "$PI_MODELS_SOURCE" ]]; then
    echo "warn: $PI_MODELS_SOURCE not found, skipping managed model overrides" >&2
  elif [[ -f "$PI_MODELS_CONFIG" ]] && jq -e --slurpfile managed "$PI_MODELS_SOURCE" \
    'contains($managed[0])' "$PI_MODELS_CONFIG" >/dev/null 2>&1; then
    plan "ok   $PI_MODELS_CONFIG managed context windows"
  else
    backup "$PI_MODELS_CONFIG"
    plan "merge $PI_MODELS_SOURCE -> $PI_MODELS_CONFIG"
    if (( IS_DRY == 0 )); then
      CURRENT='{}'
      [[ -f "$PI_MODELS_CONFIG" ]] && CURRENT="$(cat "$PI_MODELS_CONFIG")"
      UPDATED="$(jq -s '.[0] * .[1]' <(printf '%s' "$CURRENT") "$PI_MODELS_SOURCE")" \
        || { echo "FATAL: model configuration is not valid JSON" >&2; exit 1; }
      printf '%s\n' "$UPDATED" > "$PI_MODELS_CONFIG"
    fi
  fi

  [[ -f "$PI_SETTINGS_TIERS" ]] || { plan "init $PI_SETTINGS_TIERS"; run bash -c "echo '{}' > '$PI_SETTINGS_TIERS'"; }
  # agentOverrides.thinking OVERWRITES an agent's own frontmatter (agents.ts applyOverride),
  # so the tier's thinking is emitted only for agents this repo owns. A package agent that
  # tuned its own thinking keeps it, and only its model follows the tier.
  OWNED="$(cd "$REPO_TARGET/agents" 2>/dev/null && ls -d */ 2>/dev/null | tr -d / | jq -R . | jq -sc .)"
  OWNED="${OWNED:-[]}"
  # modelTierFallbacks maps each model in a tier's chain to its next hop, one map per tier,
  # so a model in two tiers' chains keeps a distinct next hop in each. tierPrimaries records
  # each tier's primary model+thinking — how a session resolves, once, which map it walks.
  # tierClimb names the tier a walk continues on when its chain runs out (T1 rises, T5 drops).
  TIER_JQ='.modelTierFallbacks = ($t.tiers | with_entries(
      .value as $tier
      | ([$tier.pi] + $tier.fallbacks) as $chain
      | .value = ([range(0; ($chain | length) - 1)
          | { key: $chain[.].model, value: { model: $chain[. + 1].model, thinking: $chain[. + 1].thinking } }]
          | from_entries)))
    | .tierPrimaries = ($t.tiers | with_entries(.value = .value.pi))
    | .tierClimb = ($t.tiers | with_entries(select(.value.climbOnExhaustion != null) | .value = .value.climbOnExhaustion))
    | .subagents = ((.subagents // {})
    | .defaultModel = $t.tiers[$t.orchestrator].pi.model
    | .defaultThinking = $t.tiers[$t.orchestrator].pi.thinking
    | .agentOverrides = ((.agentOverrides // {}) + ($t.agents | with_entries(
        .key as $name | .value as $tier
        | .value = { model: $t.tiers[$tier].pi.model,
                     fallbackModels: ($t.tiers[$tier].fallbacks | map(.model)) }
                   + (if ($owned | index($name)) then { thinking: $t.tiers[$tier].pi.thinking } else {} end)))))'
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
#    post-checkout starts each worktree's sandbox build without copying checkout content;
#    pre-push rejects any push that updates main, so main only moves through a PR
if [[ -d "$REPO_TARGET/.git/hooks" ]]; then
  link "$REPO_TARGET/.git/hooks/post-merge" "$REPO_TARGET/install.sh"
  link "$REPO_TARGET/.git/hooks/post-rewrite" "$REPO_TARGET/install.sh"
  link "$REPO_TARGET/.git/hooks/post-checkout" "$REPO_TARGET/hooks/post-checkout"
  link "$REPO_TARGET/.git/hooks/pre-push" "$REPO_TARGET/hooks/pre-push"
  link "$REPO_TARGET/.git/hooks/pre-commit" "$REPO_TARGET/hooks/pre-commit"
fi

# 8. the rust tools. these are the artifacts the installer compiles rather than links,
#    because each one sits in a path that is waited on: ste-check in the reply path,
#    no-ai-attribution in the PreToolUse path ahead of every commit,
#    session-stats as an on-demand command the user runs by name,
#    preferred-cli-guard in the pi/extensions/preferred-cli-guard tool_call path ahead of
#    every bash call, blocking a literal find/grep invocation in favor of fd/rg,
#    warnings-check in the PreToolUse path ahead of every commit that stages a .rs file
#    under tools/, alongside no-ai-attribution on the same hook,
#    comment-check in the PreToolUse path ahead of every commit that stages a source
#    file, denying a non-doc comment block over docs/comment-style.md's length budget,
#    gepa-due in the daily workflows/gepa-due launchd job, which runs with the minimal
#    PATH set in its plist (no cargo) — it needs the built binary on that PATH already,
#    not a live `cargo build` attempted inside the launchd environment,
#    transcript-directed-video-processor as an on-demand command the user runs by name,
#    privacy-lint in the pre-commit path, blocking staged private network identifiers,
#    worktree-hygiene in the 5-minute launchd hygiene job — its plist sets a minimal PATH
#    with no cargo on it, exactly like gepa-due above, so the binary must already be built
#    and symlinked here rather than compiled inside the launchd environment
for tool in ste-check no-ai-attribution session-stats preferred-cli-guard warnings-check comment-check edit-time-check gepa-due transcript-directed-video-processor privacy-lint worktree-hygiene autonomous-engineer-state; do
  build_tool "$REPO_TARGET/tools/$tool" "$tool"
done

TOOL_SYNC_CRATE="$REPO_TARGET/tools/tool-sync"
TOOL_SYNC_BIN="$TOOL_SYNC_CRATE/target/release/tool-sync"
build_tool "$TOOL_SYNC_CRATE" tool-sync

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

build_tool "$REPO_TARGET/tools/pr-review-filter" pr-review-filter
build_tool "$REPO_TARGET/tools/tool-wizard" tool-wizard

# 9. Pi shell preferences.
ZSH_PATH="$(command -v zsh || true)"
if [[ -z "$ZSH_PATH" ]]; then
  echo "warn: zsh not found, skipping Pi shell preferences" >&2
elif ! command -v jq >/dev/null 2>&1; then
  echo "warn: jq not found, skipping Pi shell preferences" >&2
else
  PI_SETTINGS="$HOME_TARGET/.pi/agent/settings.json"
  run mkdir -p "$HOME_TARGET/.pi/agent"

  if [[ -f "$PI_SETTINGS" ]] && jq -e --arg s "$ZSH_PATH" \
    '.shellPath == $s and .warnings.anthropicExtraUsage == false' "$PI_SETTINGS" >/dev/null 2>&1; then
    plan "ok   $PI_SETTINGS shell -> $ZSH_PATH, anthropic warning off"
  else
    backup "$PI_SETTINGS"
    plan "set  $PI_SETTINGS shell -> $ZSH_PATH, anthropic warning off"
    json_update "$PI_SETTINGS" --arg s "$ZSH_PATH" '.shellPath = $s | .warnings.anthropicExtraUsage = false'
  fi
fi

link_config "$REPO_TARGET/pi/themes/owais.json" "$HOME_TARGET/.pi/agent/themes/owais.json" "Pi theme link"
link_config "$REPO_TARGET/config/herdr/config.toml" "$HOME_TARGET/.config/herdr/config.toml" "Herdr config link"
link_config "$SIMSLIM_PROFILE" "$HOME_TARGET/.config/simslim/main.json" "SimSlim profile link"
link_config "$REPO_TARGET/config/pi-transcribe.json" "$HOME_TARGET/.pi/agent/pi-transcribe.json" "Pi transcription configuration link"
link_config "$REPO_TARGET/config/plannotator.json" "$HOME_TARGET/.pi/agent/plannotator.json" "Plannotator configuration link"
link_config "$REPO_TARGET/config/pi-keybindings.json" "$HOME_TARGET/.pi/agent/keybindings.json" "Pi keybindings link"

GIT_DELTA_SOURCE="$REPO_TARGET/config/git-delta.gitconfig"
if [[ ! -f "$GIT_DELTA_SOURCE" ]]; then
  echo "warn: $GIT_DELTA_SOURCE not found, skipping the delta Git configuration" >&2
elif ! command -v git >/dev/null 2>&1; then
  echo "warn: git not found, skipping the delta Git configuration" >&2
else
  GIT_XDG_CONFIG_HOME="$HOME_TARGET/.config"
  if [[ "$HOME_TARGET" == "$HOME" && -n "${XDG_CONFIG_HOME:-}" ]]; then
    GIT_XDG_CONFIG_HOME="$XDG_CONFIG_HOME"
  fi
  if [[ -e "$HOME_TARGET/.gitconfig" ]]; then
    GIT_GLOBAL_CONFIG="$HOME_TARGET/.gitconfig"
  elif [[ -e "$GIT_XDG_CONFIG_HOME/git/config" || -L "$GIT_XDG_CONFIG_HOME/git/config" ]]; then
    GIT_GLOBAL_CONFIG="$GIT_XDG_CONFIG_HOME/git/config"
  else
    GIT_GLOBAL_CONFIG="$HOME_TARGET/.gitconfig"
  fi
  GIT_CONFIG_ENV=(
    env -u GIT_CONFIG -u GIT_CONFIG_GLOBAL -u GIT_CONFIG_SYSTEM -u GIT_CONFIG_COUNT
    HOME="$HOME_TARGET" XDG_CONFIG_HOME="$GIT_XDG_CONFIG_HOME"
  )
  if (( ! IS_DRY )) && [[ ! -x "$HOME_TARGET/.local/bin/delta" ]]; then
    echo "FATAL: managed delta is missing at $HOME_TARGET/.local/bin/delta; rerun: $REPO_TARGET/install.sh" >&2
    exit 1
  fi
  "${GIT_CONFIG_ENV[@]}" git config --file "$GIT_DELTA_SOURCE" --list >/dev/null \
    || { echo "FATAL: $GIT_DELTA_SOURCE is not valid Git configuration" >&2; exit 1; }
  GIT_DELTA_KEYS=()
  while IFS= read -r key; do
    GIT_DELTA_KEYS+=("$key")
  done < <("${GIT_CONFIG_ENV[@]}" git config --file "$GIT_DELTA_SOURCE" --name-only --list)
  (( ${#GIT_DELTA_KEYS[@]} > 0 )) \
    || { echo "FATAL: $GIT_DELTA_SOURCE contains no Git settings" >&2; exit 1; }
  IS_GIT_DELTA_CHANGE=0
  for key in "${GIT_DELTA_KEYS[@]}"; do
    desired="$("${GIT_CONFIG_ENV[@]}" git config --file "$GIT_DELTA_SOURCE" --get "$key")"
    target_current="$("${GIT_CONFIG_ENV[@]}" git config --file "$GIT_GLOBAL_CONFIG" --get "$key" 2>/dev/null || true)"
    effective="$("${GIT_CONFIG_ENV[@]}" git config --includes --global --get-all "$key" 2>/dev/null || true)"
    effective="${effective//$'\n'/, }"
    printf -v target_display '%q' "${target_current:-unset}"
    printf -v effective_display '%q' "${effective:-unset}"
    if [[ "$target_current" == "$desired" ]]; then
      plan "ok   $GIT_GLOBAL_CONFIG $key -> $desired"
    else
      plan "set  $GIT_GLOBAL_CONFIG $key: $target_display -> $desired (effective: $effective_display)"
      IS_GIT_DELTA_CHANGE=1
    fi
  done
  if (( IS_GIT_DELTA_CHANGE )); then
    if [[ -L "$GIT_GLOBAL_CONFIG" && ! -e "$GIT_GLOBAL_CONFIG" ]]; then
      backup "$GIT_GLOBAL_CONFIG"
    elif [[ -e "$GIT_GLOBAL_CONFIG" ]]; then
      GIT_CONFIG_BACKUP="$GIT_GLOBAL_CONFIG.pre-reset-$STAMP"
      [[ -e "$GIT_CONFIG_BACKUP" || -L "$GIT_CONFIG_BACKUP" ]] \
        && GIT_CONFIG_BACKUP="$GIT_CONFIG_BACKUP.$(date +%H%M%S)"
      plan "backup $GIT_GLOBAL_CONFIG -> $GIT_CONFIG_BACKUP"
      run cp -pL "$GIT_GLOBAL_CONFIG" "$GIT_CONFIG_BACKUP"
      (( IS_DRY )) || cmp -s "$GIT_GLOBAL_CONFIG" "$GIT_CONFIG_BACKUP" \
        || { echo "FATAL: backup mismatch at $GIT_CONFIG_BACKUP" >&2; exit 1; }
    fi
    run mkdir -p "$(dirname "$GIT_GLOBAL_CONFIG")"
    for key in "${GIT_DELTA_KEYS[@]}"; do
      desired="$("${GIT_CONFIG_ENV[@]}" git config --file "$GIT_DELTA_SOURCE" --get "$key")"
      run "${GIT_CONFIG_ENV[@]}" git config --file "$GIT_GLOBAL_CONFIG" --replace-all "$key" "$desired"
    done
  fi
  if (( ! IS_DRY )); then
    for key in "${GIT_DELTA_KEYS[@]}"; do
      desired="$("${GIT_CONFIG_ENV[@]}" git config --file "$GIT_DELTA_SOURCE" --get "$key")"
      target_actual="$("${GIT_CONFIG_ENV[@]}" git config --file "$GIT_GLOBAL_CONFIG" --get "$key" 2>/dev/null || true)"
      [[ "$target_actual" == "$desired" ]] \
        || { echo "FATAL: $GIT_GLOBAL_CONFIG $key is $target_actual, expected $desired" >&2; exit 1; }
      effective="$("${GIT_CONFIG_ENV[@]}" git config --includes --global --get "$key" 2>/dev/null || true)"
      printf -v effective_display '%q' "${effective:-unset}"
      [[ "$effective" == "$desired" ]] \
        || { echo "FATAL: effective $key=$effective_display overrides $GIT_GLOBAL_CONFIG" >&2; exit 1; }
    done
  fi
fi

if [[ "$HOME_TARGET" == "$HOME" && "$IS_DRY" == 0 && "$IS_TEST" == 0 && "$(uname -s)" == "Darwin" ]]; then
  if ! command -v uv >/dev/null 2>&1; then
    echo "FATAL: uv is required for local transcription. Install uv, then rerun install.sh." >&2
    exit 1
  fi
  plan "install pinned parakeet-mlx 0.5.2"
  uv tool install --reinstall "parakeet-mlx==0.5.2"
else
  plan "skip local transcription setup (dry run or sandbox install)"
fi

for obsolete in \
  "$HOME_TARGET/.pi/agent/extensions/pi-chrome-devtools" \
  "$HOME_TARGET/.pi/agent/extensions/pi-voice-stt" \
  "$HOME_TARGET/.pi/agent/extensions/voice.ts" \
  "$HOME_TARGET"/.pi/agent/extensions/pi-voice-stt.pre-reset-* \
  "$HOME_TARGET"/.pi/agent/extensions/voice.ts.pre-reset-*; do
  retire_pi_extension "$obsolete"
done
PI_EVENT_TIMESTAMP_PATH="$HOME_TARGET/.pi/agent/extensions/event-timestamps.ts"
if [[ -L "$PI_EVENT_TIMESTAMP_PATH" && "$(readlink "$PI_EVENT_TIMESTAMP_PATH")" == "$REPO_TARGET/pi/extensions/event-timestamps.ts" ]]; then
  retire_pi_extension "$PI_EVENT_TIMESTAMP_PATH"
fi
if [[ "$HOME_TARGET" == "$HOME" && "$IS_DRY" == 0 && "$IS_TEST" == 0 ]]; then
  retire_to_trash "$HOME_TARGET/.pi/agent/pi-voice-server"
  retire_to_trash "$HOME_TARGET/.local/bin/pi-voice-server"
  retire_to_trash "$HOME_TARGET/.pi/agent/stt.json"
  retire_to_trash "$HOME_TARGET/.pi/agent/extensions/theme-preview.ts"
  retire_to_trash "$HOME_TARGET/.pi/agent/extensions/spinner-preview.ts"
fi

link_config "$REPO_TARGET/config/world-clock.json" "$HOME_TARGET/.pi/agent/world-clock.json" "world-clock configuration link"

PI_LEGACY_COMPACT_PATH="$HOME_TARGET/.pi/agent/extensions/compact-path.ts"
if [[ -L "$PI_LEGACY_COMPACT_PATH" && "$(readlink "$PI_LEGACY_COMPACT_PATH")" == "$REPO_TARGET/pi/extensions/compact-path.ts" ]]; then
  plan "unlink retired $PI_LEGACY_COMPACT_PATH"
  run rm "$PI_LEGACY_COMPACT_PATH"
fi

link_config "$REPO_TARGET/config/ghostty.config" "$HOME_TARGET/Library/Application Support/com.mitchellh.ghostty/config" "Ghostty configuration link"

PI_MANAGED_SETTINGS="$REPO_TARGET/config/pi-settings.json"
PI_SETTINGS="$HOME_TARGET/.pi/agent/settings.json"
if ! command -v jq >/dev/null 2>&1; then
  echo "warn: jq not found, skipping managed Pi settings" >&2
elif [[ ! -f "$PI_MANAGED_SETTINGS" ]]; then
  echo "warn: $PI_MANAGED_SETTINGS not found, skipping managed Pi settings" >&2
else
  run mkdir -p "$HOME_TARGET/.pi/agent"
  CURRENT='{}'
  [[ -f "$PI_SETTINGS" ]] && CURRENT="$(cat "$PI_SETTINGS")"
  UPDATED="$(jq -s '.[0] * .[1]' <(printf '%s' "$CURRENT") "$PI_MANAGED_SETTINGS")" \
    || { echo "FATAL: Pi settings are not valid JSON" >&2; exit 1; }
  if [[ "$(printf '%s' "$UPDATED" | jq -S .)" == "$(jq -S . "$PI_SETTINGS" 2>/dev/null)" ]]; then
    plan "ok   $PI_SETTINGS managed Pi settings"
  else
    backup "$PI_SETTINGS"
    plan "merge $PI_MANAGED_SETTINGS -> $PI_SETTINGS"
    (( IS_DRY )) || printf '%s\n' "$UPDATED" > "$PI_SETTINGS"
  fi
fi

# DonSeTch owns web_search; the legacy package reads its separate config for the fallback name.
if ! command -v jq >/dev/null 2>&1; then
  echo "warn: jq not found, skipping web search fallback configuration" >&2
else
  PI_WEBSEARCH="$HOME_TARGET/.pi/web-search.json"
  run mkdir -p "$HOME_TARGET/.pi"
  if [[ -f "$PI_WEBSEARCH" ]] && jq -e '.workflow == "none" and .autoOpenBrowser == false and .tools.webSearch.enabled == true and .toolNames.webSearch == "fallback_web_search"' "$PI_WEBSEARCH" >/dev/null 2>&1; then
    plan "ok   $PI_WEBSEARCH fallback search enabled, curator off"
  else
    backup "$PI_WEBSEARCH"
    plan "set  $PI_WEBSEARCH fallback search enabled, curator off"
    json_update "$PI_WEBSEARCH" '.workflow = "none" | .autoOpenBrowser = false | .tools.webSearch.enabled = true | .toolNames.webSearch = "fallback_web_search"'
  fi
fi

# 18. launchd scheduled jobs — every plist under */launchd/*.plist in this repo (hq's
#     heartbeat, scheduled-ideation's daily 3pm sweep, and any future one) gets installed
#     and its registration refreshed on every install.sh run, on every device, so the
#     schedule is never something a human has to remember to set up by hand. Refresh means
#     re-copying the plist and re-bootstrapping so a content change actually takes effect —
#     launchd does not reread a changed file on its own. It never means kickstart: this
#     step also runs from the post-merge git hook, so an unconditional kickstart here would
#     re-fire every scheduled job (hq's heartbeat, a daily ideation sweep) on every git
#     checkout, not just once. kickstart fires only the first time a label is newly
#     installed, matching "run once now" for a fresh install without repeating it forever.
if [[ "$HOME_TARGET" != "$HOME" ]]; then
  # sandboxed/test run: no real launchd session to touch, and a sandboxed install must
  # never register a real system-level scheduled job
  plan "skip launchd install (sandboxed HOME_TARGET)"
elif ! command -v launchctl >/dev/null 2>&1; then
  plan "skip launchd install (not macOS or launchctl unavailable)"
else
  # every plist lives at <category>/<name>/launchd/*.plist — skills/hq/launchd/*.plist,
  # workflows/scheduled-ideation/launchd/*.plist, and any future one at that same depth
  LAUNCHD_AGENTS_DIR="$HOME_TARGET/Library/LaunchAgents"
  for PLIST_SRC in "$REPO_TARGET"/*/*/launchd/*.plist; do
    [[ -f "$PLIST_SRC" ]] || continue
    LABEL="$(basename "$PLIST_SRC" .plist)"
    PLIST_DEST="$LAUNCHD_AGENTS_DIR/$LABEL.plist"
    ALREADY_LOADED=0
    launchctl print "gui/$(id -u)/$LABEL" >/dev/null 2>&1 && ALREADY_LOADED=1
    if [[ -f "$PLIST_DEST" ]] && diff -q "$PLIST_SRC" "$PLIST_DEST" >/dev/null 2>&1; then
      plan "ok   $LABEL launchd plist already current"
      continue
    fi
    plan "install launchd job $LABEL"
    if (( ! IS_DRY )); then
      mkdir -p "$LAUNCHD_AGENTS_DIR"
      cp "$PLIST_SRC" "$PLIST_DEST"
      launchctl bootout "gui/$(id -u)/$LABEL" >/dev/null 2>&1 || true
      launchctl bootstrap "gui/$(id -u)" "$PLIST_DEST"
      if (( ! ALREADY_LOADED )); then
        launchctl kickstart "gui/$(id -u)/$LABEL"
        plan "kickstart $LABEL (first install on this device)"
      fi
    fi
  done
fi

plan "done"

# 19. --test borrows the real pi credential through a symlink instead of copying it: the
#     sandbox never holds its own token, and deleting .install-test-home deletes only the
#     link. The interactive drop into pi lives in test/run, opt-in, never here — --test
#     stays non-interactive so it can run from hooks and background checks.
if (( IS_TEST )) && ! (( IS_DRY )); then
  if [[ -f "$HOME/.pi/agent/auth.json" ]]; then
    link "$HOME_TARGET/.pi/agent/auth.json" "$HOME/.pi/agent/auth.json"
  fi
fi
