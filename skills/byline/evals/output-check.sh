#!/bin/zsh
set -euo pipefail
here=${0:A:h}
repo=$(git -C "$here" rev-parse --show-toplevel)
checker="$repo/tools/ste-check/target/debug/ste-check"
[[ -x "$checker" ]] || cargo build --quiet --manifest-path "$repo/tools/ste-check/Cargo.toml"
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
cat > "$tmp"
"$checker" --register byline "$tmp"
