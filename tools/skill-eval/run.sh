#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
repo_root=${script_dir:h:h}

cargo build --manifest-path "$repo_root/tools/tier-dispatch/Cargo.toml"
cargo build --manifest-path "$script_dir/Cargo.toml"

export TIER_DISPATCH_BIN=${TIER_DISPATCH_BIN:-"$repo_root/tools/tier-dispatch/target/debug/tier-dispatch"}
export TIERS_FILE=${TIERS_FILE:-"$repo_root/config/model-tiers.json"}
exec "$script_dir/target/debug/skill-eval" "$@"
