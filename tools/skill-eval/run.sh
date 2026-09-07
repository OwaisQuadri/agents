#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
repo_root=${script_dir:h:h}

cargo build --manifest-path "$repo_root/tools/tier-dispatch/Cargo.toml"
cargo build --manifest-path "$script_dir/Cargo.toml"

dispatch_target=${CARGO_TARGET_DIR:-"$repo_root/tools/tier-dispatch/target"}
skill_target=${CARGO_TARGET_DIR:-"$script_dir/target"}
export TIER_DISPATCH_BIN=${TIER_DISPATCH_BIN:-"$dispatch_target/debug/tier-dispatch"}
export TIERS_FILE=${TIERS_FILE:-"$repo_root/config/model-tiers.json"}
exec "$skill_target/debug/skill-eval" "$@"
