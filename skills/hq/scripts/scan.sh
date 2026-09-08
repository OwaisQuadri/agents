#!/bin/zsh
set -euo pipefail

scripts_dir=${0:A:h}
exec cargo run --quiet --manifest-path "$scripts_dir/../Cargo.toml" --bin hq-state -- "$@"
