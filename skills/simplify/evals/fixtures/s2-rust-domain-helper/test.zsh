#!/bin/zsh
set -euo pipefail
rustc --emit=metadata policy.rs -o "$TMPDIR/policy.rmeta"
grep -Fq 'match parse_customer_policy(value)' policy.rs
print -r -- 'policy=retail'
print -r -- 'error=unknown customer policy'
