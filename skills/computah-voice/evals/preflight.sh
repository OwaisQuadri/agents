#!/bin/zsh
set -euo pipefail
here=${0:A:h}
repo=$(git -C "$here" rev-parse --show-toplevel)
cargo build --quiet --manifest-path "$repo/tools/ste-check/Cargo.toml"
