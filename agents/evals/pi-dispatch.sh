#!/bin/zsh
set -euo pipefail

pi_eval_requirements() {
  REPO_ROOT=$(git -C "${0:A:h}" rev-parse --show-toplevel)
  TIER_DISPATCH=${TIER_DISPATCH_BIN:-"$REPO_ROOT/tools/tier-dispatch/target/debug/tier-dispatch"}
  TIER_CONFIG="$REPO_ROOT/config/model-tiers.json"
  PI_LAUNCHER="$REPO_ROOT/agents/evals/pi-launcher.sh"
  command -v cargo >/dev/null || { print -u2 "cargo is required"; return 1; }
  command -v jq >/dev/null || { print -u2 "jq is required"; return 1; }
  command -v pi >/dev/null || { print -u2 "Pi CLI is required"; return 1; }
  PI_EXECUTABLE=$(command -v pi)
  [[ -r "$TIER_CONFIG" ]] || { print -u2 "tier configuration not found: $TIER_CONFIG"; return 1; }
  cargo build --quiet --manifest-path "$REPO_ROOT/tools/tier-dispatch/Cargo.toml" || return 1
  [[ -x "$TIER_DISPATCH" ]] || { print -u2 "tier dispatcher not found: $TIER_DISPATCH"; return 1; }
}

pi_eval_dispatch() {
  local agent=$1 definition=$2 workdir=$3 input=$4
  local tier pi_root body exit_status

  [[ -r "$definition" ]] || { print -u2 "agent definition not found: $definition"; return 1; }
  tier=$(jq -er --arg agent "$agent" '.agents[$agent] // .orchestrator' "$TIER_CONFIG") || return 1
  pi_root=$(mktemp -d "${TMPDIR:-/tmp}/pi-eval.${agent}.XXXXXXXX") || return 1
  mkdir -p "$pi_root/agents" "$pi_root/sessions"
  cp "$definition" "$pi_root/agents/$agent.md"
  body="$pi_root/system-prompt.md"
  awk 'c >= 2 { print } /^---$/ { c += 1 }' "$definition" > "$body"
  if PI_EVAL_WORKDIR="$workdir" PI_EVAL_PI_DIR="$pi_root" PI_EVAL_PI_EXECUTABLE="$PI_EXECUTABLE" "$TIER_DISPATCH" \
    --tiers-file "$TIER_CONFIG" \
    --tier "$tier" \
    --system-prompt-file "$body" \
    --input "$input" \
    --dispatch-bin "$PI_LAUNCHER"; then
    exit_status=0
  else
    exit_status=$?
  fi
  rm -rf "$pi_root"
  return "$exit_status"
}
