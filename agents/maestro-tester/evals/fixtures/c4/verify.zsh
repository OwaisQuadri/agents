#!/bin/zsh
set -eu
flow=$(find .maestro -type f \( -name '*.yaml' -o -name '*.yml' \) -print -quit)
report=$(find .maestro -type f -name '*.xml' -print -quit)
[[ -n "$flow" && -n "$report" ]]
grep -q 'assert' "$flow"
grep -q 'Zzyzx Quantum Flamingo' "$flow"
grep -Eq 'failures="[1-9][0-9]*"' "$report"
grep -q 'Zzyzx Quantum Flamingo' "$report"
