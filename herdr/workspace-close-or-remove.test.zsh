set -euo pipefail

repo_root="${0:A:h:h}"
target="$repo_root/herdr/workspace-close-or-remove.zsh"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/workspace-close-or-remove.XXXXXX")"
trap 'rm -rf "$test_root"' EXIT

fail() {
  print -u2 -- "FAIL: $1"
  exit 1
}

assert_contains() {
  grep -Fx -- "$2" "$1" >/dev/null || fail "$3"
}

assert_not_contains() {
  if grep -F -- "$2" "$1" >/dev/null; then
    fail "$3"
  fi
}

assert_has_text() {
  if ! grep -F -- "$2" "$1" >/dev/null; then
    print -u2 -r -- "$(<"$1")"
    fail "$3"
  fi
}

assert_not_line() {
  if grep -Fx -- "$2" "$1" >/dev/null; then
    fail "$3"
  fi
}

make_tools() {
  local case_root="$1"
  mkdir -p "$case_root/bin"
  : > "$case_root/calls"
  cat > "$case_root/bin/fzf" <<'EOF'
#!/bin/zsh
set -euo pipefail
input="$(cat)"
print -r -- "prompt $*" >> "$CALL_LOG"
print -r -- "row $input" >> "$CALL_LOG"
response="$(head -n 1 "$FZF_RESPONSES" || true)"
tail -n +2 "$FZF_RESPONSES" > "$FZF_RESPONSES.next"
mv "$FZF_RESPONSES.next" "$FZF_RESPONSES"
[[ "$response" == accept ]] || exit 130
print -r -- "$input"
EOF
  cat > "$case_root/bin/herdr" <<'EOF'
#!/bin/zsh
set -euo pipefail
print -r -- "$*" >> "$CALL_LOG"
if [[ "$1 $2" == "api snapshot" ]]; then
  if [[ "${SNAPSHOT_RESULT:-ok}" == error ]]; then
    print -u2 -r -- '{"error":{"code":"server_unavailable","message":"simulated snapshot failure"}}'
    exit 1
  fi
  print -r -- "$SNAPSHOT_JSON"
  exit 0
fi
if [[ "$1 $2" == "workspace close" ]]; then
  if [[ "${CLOSE_RESULT:-ok}" == error ]]; then
    print -u2 -r -- '{"error":{"code":"workspace_close_failed","message":"simulated close failure"}}'
    exit 1
  fi
  print -r -- '{"result":{"closed":true}}'
  exit 0
fi
if [[ "$1 $2" == "worktree remove" ]]; then
  if [[ "${REMOVE_RESULT:-git}" == other-error ]]; then
    print -u2 -r -- '{"error":{"code":"worktree_remove_failed","message":"simulated failure"}}'
    exit 1
  fi
  is_force=false
  for arg in "$@"; do
    [[ "$arg" == --force ]] && is_force=true
  done
  command=(git -C "$REPO_ROOT" worktree remove)
  $is_force && command+=(--force)
  command+=("$CHECKOUT_PATH")
  if ! message="$("$command[@]" 2>&1)"; then
    print -u2 -r -- "{\"error\":{\"code\":\"dirty_worktree_requires_force\",\"message\":$(jq -Rn --arg message "$message" '$message')}}"
    exit 1
  fi
  print -r -- '{"result":{"removed":true}}'
  exit 0
fi
print -u2 -- "unexpected fake herdr command: $*"
exit 2
EOF
  chmod +x "$case_root/bin/fzf" "$case_root/bin/herdr"
}

normal_snapshot() {
  jq -cn --arg id "$1" --arg label "$2" '{result:{snapshot:{workspaces:[{workspace_id:$id,label:$label,worktree:null}]}}}'
}

worktree_snapshot() {
  jq -cn --arg id "$1" --arg label "$2" --arg path "$3" '{result:{snapshot:{workspaces:[{workspace_id:$id,label:$label,worktree:{checkout_path:$path,is_linked_worktree:true,repo_key:"repo/.git",repo_name:"repo",repo_root:"repo"}}]}}}'
}

main_repo_snapshot() {
  jq -cn --arg id "$1" --arg label "$2" --arg path "$3" '{result:{snapshot:{workspaces:[{workspace_id:$id,label:$label,worktree:{checkout_path:$path,is_linked_worktree:false,repo_key:"repo/.git",repo_name:"repo",repo_root:$path}}]}}}'
}

run_target() {
  local case_root="$1"
  shift
  env \
    HERDR_ACTIVE_WORKSPACE_ID="$WORKSPACE_ID" \
    HERDR_BIN_PATH="$case_root/bin/herdr" \
    FZF_BIN_PATH="$case_root/bin/fzf" \
    FZF_RESPONSES="$case_root/responses" \
    CALL_LOG="$case_root/calls" \
    SNAPSHOT_JSON="$SNAPSHOT_JSON" \
    SNAPSHOT_RESULT="${SNAPSHOT_RESULT:-ok}" \
    CLOSE_RESULT="${CLOSE_RESULT:-ok}" \
    REMOVE_RESULT="${REMOVE_RESULT:-git}" \
    REPO_ROOT="${REPO_ROOT:-$case_root}" \
    CHECKOUT_PATH="${CHECKOUT_PATH:-$case_root/checkout}" \
    /bin/zsh "$target" "$@"
}

case_root="$test_root/normal-cancel"
make_tools "$case_root"
print -r -- cancel > "$case_root/responses"
WORKSPACE_ID=w-normal SNAPSHOT_JSON="$(normal_snapshot w-normal normal)" run_target "$case_root"
assert_not_contains "$case_root/calls" "workspace close" "normal cancellation closed the workspace"

case_root="$test_root/normal-accept"
make_tools "$case_root"
print -r -- accept > "$case_root/responses"
WORKSPACE_ID=w-normal SNAPSHOT_JSON="$(normal_snapshot w-normal normal)" run_target "$case_root"
assert_contains "$case_root/calls" "workspace close w-normal" "normal acceptance did not close the exact workspace"
assert_has_text "$case_root/calls" "Enter: close workspace · Esc: cancel" "normal confirmation instructions changed"
assert_contains "$case_root/calls" "row Close workspace: normal" "normal confirmation hid the workspace label"
assert_not_contains "$case_root/calls" "--with-nth" "normal confirmation truncated its action row"

case_root="$test_root/main-repo-accept"
make_tools "$case_root"
print -r -- accept > "$case_root/responses"
WORKSPACE_ID=w-main SNAPSHOT_JSON="$(main_repo_snapshot w-main main "$case_root/repo")" run_target "$case_root"
assert_contains "$case_root/calls" "workspace close w-main" "main repository workspace did not close"
assert_not_contains "$case_root/calls" "worktree remove" "main repository workspace attempted worktree removal"

case_root="$test_root/close-error"
make_tools "$case_root"
printf 'accept\naccept\n' > "$case_root/responses"
set +e
WORKSPACE_ID=w-close-error SNAPSHOT_JSON="$(normal_snapshot w-close-error normal)" CLOSE_RESULT=error run_target "$case_root" > "$case_root/stdout" 2> "$case_root/stderr"
close_exit_code=$?
set -e
[[ $close_exit_code -eq 1 ]] || fail "workspace close error did not fail"
assert_contains "$case_root/calls" "row Close failed" "workspace close error was not held open"
assert_has_text "$case_root/calls" "workspace_close_failed" "workspace close error hid its message"

case_root="$test_root/snapshot-error"
make_tools "$case_root"
print -r -- accept > "$case_root/responses"
set +e
WORKSPACE_ID=w-snapshot SNAPSHOT_JSON='{}' SNAPSHOT_RESULT=error run_target "$case_root" > "$case_root/stdout" 2> "$case_root/stderr"
snapshot_exit_code=$?
set -e
[[ $snapshot_exit_code -eq 1 ]] || fail "snapshot error did not fail"
assert_contains "$case_root/calls" "row Herdr unavailable" "snapshot error was not held open"
assert_has_text "$case_root/calls" "server_unavailable" "snapshot error hid its message"

case_root="$test_root/worktree-cancel"
make_tools "$case_root"
print -r -- cancel > "$case_root/responses"
WORKSPACE_ID=w-worktree SNAPSHOT_JSON="$(worktree_snapshot w-worktree linked "$case_root/checkout")" run_target "$case_root"
assert_not_contains "$case_root/calls" "worktree remove" "worktree cancellation attempted removal"

make_git_fixture() {
  local case_root="$1"
  mkdir -p "$case_root/repo"
  git -C "$case_root/repo" init -q
  git -C "$case_root/repo" config user.name test
  git -C "$case_root/repo" config user.email test@example.com
  print -r -- base > "$case_root/repo/file"
  git -C "$case_root/repo" add file
  git -C "$case_root/repo" commit -qm base
  git -C "$case_root/repo" branch feature
  git -C "$case_root/repo" worktree add -q "$case_root/checkout" feature
}

case_root="$test_root/worktree-accept"
make_tools "$case_root"
make_git_fixture "$case_root"
print -r -- accept > "$case_root/responses"
WORKSPACE_ID=w-worktree SNAPSHOT_JSON="$(worktree_snapshot w-worktree linked "$case_root/checkout")" REPO_ROOT="$case_root/repo" CHECKOUT_PATH="$case_root/checkout" run_target "$case_root"
[[ ! -d "$case_root/checkout" ]] || fail "accepted removal kept the checkout folder"
git -C "$case_root/repo" show-ref --verify --quiet refs/heads/feature || fail "accepted removal deleted the branch"
assert_contains "$case_root/calls" "worktree remove --workspace w-worktree" "safe removal did not target the exact workspace"
assert_not_contains "$case_root/calls" "--force" "clean removal used force"
assert_has_text "$case_root/calls" "Enter: delete checkout folder · Esc: cancel · branch stays" "worktree confirmation instructions changed"
assert_contains "$case_root/calls" "row Remove checkout: $case_root/checkout" "worktree confirmation hid the checkout path"
assert_not_contains "$case_root/calls" "--with-nth" "worktree confirmation truncated its action row"

case_root="$test_root/dirty-cancel"
make_tools "$case_root"
make_git_fixture "$case_root"
print -r -- changed >> "$case_root/checkout/file"
printf 'accept\ncancel\n' > "$case_root/responses"
WORKSPACE_ID=w-dirty SNAPSHOT_JSON="$(worktree_snapshot w-dirty dirty "$case_root/checkout")" REPO_ROOT="$case_root/repo" CHECKOUT_PATH="$case_root/checkout" run_target "$case_root"
[[ -d "$case_root/checkout" ]] || fail "dirty cancellation removed the checkout folder"
assert_not_line "$case_root/calls" "worktree remove --workspace w-dirty --force" "dirty cancellation used force"

case_root="$test_root/dirty-accept"
make_tools "$case_root"
make_git_fixture "$case_root"
print -r -- changed >> "$case_root/checkout/file"
printf 'accept\naccept\n' > "$case_root/responses"
WORKSPACE_ID=w-dirty SNAPSHOT_JSON="$(worktree_snapshot w-dirty dirty "$case_root/checkout")" REPO_ROOT="$case_root/repo" CHECKOUT_PATH="$case_root/checkout" run_target "$case_root"
[[ ! -d "$case_root/checkout" ]] || fail "forced removal kept the checkout folder"
git -C "$case_root/repo" show-ref --verify --quiet refs/heads/feature || fail "forced removal deleted the branch"
assert_contains "$case_root/calls" "worktree remove --workspace w-dirty --force" "forced removal did not target the exact workspace"
assert_has_text "$case_root/calls" "Enter: delete files · Esc: cancel" "force confirmation instructions changed"
assert_has_text "$case_root/calls" "Branch stays." "force confirmation branch guarantee changed"
assert_contains "$case_root/calls" "row Force removal: $case_root/checkout" "force confirmation hid the checkout path"

case_root="$test_root/other-error"
make_tools "$case_root"
printf 'accept\naccept\n' > "$case_root/responses"
set +e
WORKSPACE_ID=w-error SNAPSHOT_JSON="$(worktree_snapshot w-error linked "$case_root/checkout")" REMOVE_RESULT=other-error run_target "$case_root" > "$case_root/stdout" 2> "$case_root/stderr"
other_exit_code=$?
set -e
[[ $other_exit_code -eq 1 ]] || fail "non-dirty removal error did not fail"
assert_not_line "$case_root/calls" "worktree remove --workspace w-error --force" "non-dirty removal error enabled force"
assert_not_contains "$case_root/calls" "row Force removal:" "non-dirty Herdr error opened a force confirmation"
assert_contains "$case_root/calls" "row Removal failed" "non-dirty Herdr error was not held open"
assert_has_text "$case_root/calls" "simulated failure" "non-dirty Herdr error hid its message"

case_root="$test_root/stale-workspace"
make_tools "$case_root"
print -r -- accept > "$case_root/responses"
set +e
WORKSPACE_ID=w-missing SNAPSHOT_JSON="$(normal_snapshot w-other other)" run_target "$case_root" > "$case_root/stdout" 2> "$case_root/stderr"
stale_exit_code=$?
set -e
[[ $stale_exit_code -ne 0 ]] || fail "missing workspace state succeeded"
assert_not_contains "$case_root/calls" "workspace close" "missing workspace state closed a workspace"
assert_not_contains "$case_root/calls" "worktree remove" "missing workspace state removed a worktree"

case_root="$test_root/malformed-worktree"
make_tools "$case_root"
print -r -- accept > "$case_root/responses"
malformed_snapshot="$(jq -cn '{result:{snapshot:{workspaces:[{workspace_id:"w-malformed",label:"malformed",worktree:{is_linked_worktree:true}}]}}}')"
set +e
WORKSPACE_ID=w-malformed SNAPSHOT_JSON="$malformed_snapshot" run_target "$case_root" > "$case_root/stdout" 2> "$case_root/stderr"
malformed_exit_code=$?
set -e
[[ $malformed_exit_code -ne 0 ]] || fail "worktree without a checkout path succeeded"
assert_not_contains "$case_root/calls" "workspace close" "malformed worktree state closed the workspace"
assert_not_contains "$case_root/calls" "worktree remove" "malformed worktree state attempted removal"

binding_count="$(grep -c 'key = "prefix+shift+d"' "$repo_root/config/herdr/config.toml" || true)"
[[ "$binding_count" -eq 1 ]] || fail "managed config does not contain exactly one prefix+shift+d binding"
grep -Fx 'command = "/bin/zsh /Users/owaisquadri/Documents/agents/herdr/workspace-close-or-remove.zsh"' "$repo_root/config/herdr/config.toml" >/dev/null || fail "managed binding points at the wrong command"

print "PASS: workspace close or remove"
