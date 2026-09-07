set -euo pipefail
path=(/opt/homebrew/bin /usr/local/bin $path)

herdr_bin="${HERDR_BIN_PATH:-$(command -v herdr 2>/dev/null || true)}"
fzf_bin="${FZF_BIN_PATH:-$(command -v fzf 2>/dev/null || true)}"
workspace_id="${HERDR_ACTIVE_WORKSPACE_ID:-}"

[[ -n "$herdr_bin" ]] || { print -u2 -- "herdr is unavailable"; exit 1; }
[[ -n "$fzf_bin" ]] || { print -u2 -- "fzf is unavailable"; exit 1; }
[[ -n "$workspace_id" ]] || { print -u2 -- "active Herdr workspace is unavailable"; exit 1; }

confirm_action() {
  local action="$1"
  local header="$2"
  local selected
  selected="$(print -r -- "$action" |
    "$fzf_bin" --prompt='> ' --height=100% --no-sort --header="$header")" || return 1
  [[ -n "$selected" ]]
}

show_error() {
  local title="$1"
  local message="$2"
  local wrapped
  wrapped="$(print -r -- "$message" | fold -w 72)"
  confirm_action "$title" "$wrapped"$'\n''Enter or Esc: close' || true
}

if snapshot="$("$herdr_bin" api snapshot 2>&1)"; then
  :
else
  exit_code=$?
  show_error "Herdr unavailable" "$snapshot"
  exit "$exit_code"
fi
if workspace="$(print -rn -- "$snapshot" | jq -c --arg id "$workspace_id" '
  [.result.snapshot.workspaces[] | select(.workspace_id == $id)] |
  if length == 1 then .[0] else empty end
' 2>/dev/null)"; then
  :
else
  show_error "Invalid Herdr state" "The Herdr workspace snapshot could not be read."
  exit 1
fi
if [[ -z "$workspace" ]]; then
  show_error "Workspace unavailable" "The active Herdr workspace was not found: $workspace_id"
  exit 1
fi

label="$(print -rn -- "$workspace" | jq -r '.label')"
is_linked_worktree="$(print -rn -- "$workspace" | jq -r '.worktree.is_linked_worktree == true')"

if [[ "$is_linked_worktree" == false ]]; then
  confirm_action "Close workspace: $label" "Enter: close workspace · Esc: cancel" || exit 0
  if close_error="$("$herdr_bin" workspace close "$workspace_id" 2>&1)"; then
    exit 0
  else
    exit_code=$?
  fi
  show_error "Close failed" "$close_error"
  exit "$exit_code"
fi

checkout_path="$(print -rn -- "$workspace" | jq -r '.worktree.checkout_path // empty')"
if [[ -z "$checkout_path" ]]; then
  show_error "Checkout unavailable" "The active linked worktree has no checkout path: $workspace_id"
  exit 1
fi
confirm_action "Remove checkout: $checkout_path" "Enter: delete checkout folder · Esc: cancel · branch stays" || exit 0
error_file="$(mktemp "${TMPDIR:-/tmp}/herdr-worktree-remove.XXXXXX")"
trap 'rm -f "$error_file"' EXIT

if "$herdr_bin" worktree remove --workspace "$workspace_id" > /dev/null 2> "$error_file"; then
  exit 0
else
  exit_code=$?
fi
error="$(<"$error_file")"
error_code="$(print -rn -- "$error" | jq -r '.error.code // empty' 2>/dev/null || true)"
error_message="$(print -rn -- "$error" | jq -r '.error.message // "Worktree removal failed."' 2>/dev/null || print "Worktree removal failed.")"

if [[ "$error_code" != dirty_worktree_requires_force ]]; then
  show_error "Removal failed" "$error_message"
  exit "$exit_code"
fi

force_header='The checkout has modified or untracked files.'$'\n''Enter: delete files · Esc: cancel'$'\n''Branch stays.'
confirm_action "Force removal: $checkout_path" "$force_header" || exit 0

if "$herdr_bin" worktree remove --workspace "$workspace_id" --force > /dev/null 2> "$error_file"; then
  exit 0
else
  exit_code=$?
fi
error="$(<"$error_file")"
error_message="$(print -rn -- "$error" | jq -r '.error.message // "Forced worktree removal failed."' 2>/dev/null || print "Forced worktree removal failed.")"
show_error "Forced removal failed" "$error_message"
exit "$exit_code"
