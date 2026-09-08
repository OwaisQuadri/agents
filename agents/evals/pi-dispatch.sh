#!/bin/zsh

PI_EVAL_DISPATCH_DIR="${${(%):-%N}:A:h}"

pi_eval_cleanup() {
  local pi_root=$1 tmp_root=${TMPDIR:-/tmp}
  [[ "$tmp_root" == / ]] || tmp_root=${tmp_root%/}
  [[ -d "$pi_root" && ! -L "$pi_root" && "$pi_root" == "$tmp_root"/pi-eval.* ]] || return 1
  rm -rf -- "$pi_root"
}

pi_eval_requirements() {
  REPO_ROOT=$(git -C "$PI_EVAL_DISPATCH_DIR" rev-parse --show-toplevel) || return 1
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
  local tmp_root=${TMPDIR:-/tmp}
  [[ "$tmp_root" == / ]] || tmp_root=${tmp_root%/}
  pi_root=$(mktemp -d "$tmp_root/pi-eval.${agent}.XXXXXXXX") || return 1
  mkdir -p "$pi_root/sessions" || { pi_eval_cleanup "$pi_root"; return 1; }
  body="$pi_root/system-prompt.md"
  awk 'c >= 2 { print } /^---$/ { c += 1 }' "$definition" > "$body" || { pi_eval_cleanup "$pi_root"; return 1; }
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
  pi_eval_cleanup "$pi_root" || return 1
  return "$exit_status"
}
