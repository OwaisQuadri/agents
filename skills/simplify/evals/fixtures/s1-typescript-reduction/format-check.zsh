#!/bin/zsh
set -euo pipefail
! grep -Eq ';.*;' pricing.ts
