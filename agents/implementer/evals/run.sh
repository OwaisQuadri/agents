#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
command -v claude >/dev/null || { echo 'claude CLI required' >&2; exit 1; }
def="${1:-../implementer.md}"
body=$(awk 'c>=2{print} /^---$/{c++}' "$def")
while IFS= read -r case_line; do
  [[ -n "$case_line" ]] || continue
  id=$(jq -r .id <<<"$case_line")
  expect=$(jq -r .expect <<<"$case_line")
  fix=$(mktemp -d)
  trap 'rm -rf "$fix"' EXIT
  mkdir -p "$fix/src"
  printf 'package = "acme-sdk"\nversion = "1.2.3"\n' > "$fix/pyproject.toml"
  printf '# fixture\n' > "$fix/src/cache.py"
  input=$(jq -r .input <<<"$case_line" | sed "s|__FIXTURE__|$fix|g")
  prompt="You are dispatched as the implementer agent. Execute the dispatch and reply in the exact output contract.\n\n$input"
  out=$(cd "$fix" && printf '%b' "$prompt" | claude -p --append-system-prompt "$body" --allowedTools 'Bash,Read,Write,Edit,Grep,Glob' 2>/dev/null || true)
  score=0; mode='missing-output'
  if grep -q 'status:' <<<"$out" && grep -q 'version_basis:' <<<"$out" && grep -q 'verification:' <<<"$out"; then
    score=6; mode='shape-present'
  fi
  if [[ "$expect" == *'invalid-dispatch'* ]] && grep -q 'invalid-dispatch' <<<"$out"; then score=10; mode='correct-invalid-dispatch'; fi
  if [[ "$expect" == *'out-of-trigger'* ]] && grep -q 'out-of-trigger' <<<"$out"; then score=10; mode='correct-out-of-trigger'; fi
  printf '{"id":%s,"score":%d,"failure_mode":%s}\n' "$(jq -c .id <<<"$case_line")" "$score" "$(jq -Rn --arg m "$mode" '$m')"
  rm -rf "$fix"; trap - EXIT
done < <(if [[ "${1:-}" == --holdout ]]; then jq -c 'select(.holdout == true)' cases.jsonl; else jq -c 'select(.holdout == false)' cases.jsonl; fi)
