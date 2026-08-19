#!/bin/zsh
# worktree-layout — herdr worktree.created/worktree.opened hook: apply the
# two-tab agent/editor layout to the event's workspace.
# Input:  HERDR_PLUGIN_EVENT_JSON — herdr EventEnvelope; workspace id and the
#         worktree path come from its data.
# Exit:   0 when the layout is applied, already correct, or another run holds
#         the workspace lock; non-zero when herdr or the event is unusable.
set -euo pipefail

TAG="[worktree-layout]"

fail() {
  print -u2 -- "$TAG error: $1"
  exit 1
}

# event_field <jq_filter> <description> — parse HERDR_PLUGIN_EVENT_JSON, print
# the filtered string; exit 1 tagged when the env or the field is unusable.
event_field() {
  local filter="$1" what="$2" value
  [[ -n "${HERDR_PLUGIN_EVENT_JSON:-}" ]] || fail "HERDR_PLUGIN_EVENT_JSON is not set"
  value="$(print -rn -- "$HERDR_PLUGIN_EVENT_JSON" | jq -er "$filter" 2>/dev/null)" ||
    fail "event has no $what"
  [[ -n "$value" ]] || fail "event has an empty $what"
  print -rn -- "$value"
}

# event_workspace_id — parse HERDR_PLUGIN_EVENT_JSON, print the workspace id.
event_workspace_id() {
  event_field '.data.workspace.id // .data.worktree.open_workspace_id // empty' "workspace id"
}

# event_worktree_path — parse HERDR_PLUGIN_EVENT_JSON, print the worktree path.
event_worktree_path() {
  local wt_path
  wt_path="$(event_field '.data.worktree.path // empty' "worktree.path")"
  [[ "$wt_path" == /* ]] || fail "worktree.path is not absolute: $wt_path"
  print -rn -- "$wt_path"
}

# acquire_workspace_lock <workspace_id> — single-flight per workspace: mkdir
# lock under TMPDIR, stale after 120s; a losing concurrent run exits 0 without
# mutating. Different workspaces never serialize.
LOCK_DIR=""
acquire_workspace_lock() {
  local ws="$1" mtime now
  LOCK_DIR="${TMPDIR:-/tmp}/agents-worktree-layout-${ws//\//_}.lock"
  if ! mkdir "$LOCK_DIR" 2>/dev/null; then
    mtime="$(stat -f %m "$LOCK_DIR" 2>/dev/null || stat -c %Y "$LOCK_DIR" 2>/dev/null)" || {
      print -- "$TAG another run holds the lock for $ws; leaving it alone"
      exit 0
    }
    now="$(date +%s)"
    if (( now - mtime > 120 )); then
      rmdir "$LOCK_DIR" 2>/dev/null || true
      mkdir "$LOCK_DIR" 2>/dev/null || {
        print -- "$TAG another run holds the lock for $ws; leaving it alone"
        exit 0
      }
    else
      print -- "$TAG another run holds the lock for $ws; leaving it alone"
      exit 0
    fi
  fi
  # Lock acquired successfully; trap will be set in main() after return
}

# tab_id_by_label <workspace_id> <label> — print the tab id carrying <label> in
# the workspace, or nothing when absent.
tab_id_by_label() {
  local ws="$1" label="$2" listing
  listing="$(herdr tab list --workspace "$ws")" || fail "herdr tab list failed for $ws"
  print -rn -- "$listing" |
    jq -r --arg label "$label" '.result.tabs[] | select(.label == $label) | .tab_id' |
    head -n 1
}

# pane_foreground <pane_id> — name, argv0, and cmdline basenames of the pane's
# first foreground process, one space-joined string (pi reports name=node
# argv0=pi with no cmdline).
pane_foreground() {
  herdr pane process-info --pane "$1" 2>/dev/null |
    jq -r '.result.process_info.foreground_processes[0]
           | [(.name // empty), (.argv0 // empty),
              ((.cmdline // "") | split(" ") | map(split("/")[-1]) | join(" "))]
           | join(" ")'
}

# pane_runs_main <pane_id> <main_word> — exit 0 when the pane's foreground or
# terminal title identifies main_word (pi titles as π).
pane_runs_main() {
  local pane="$1" main_word="$2" fg title
  fg="$(pane_foreground "$pane")"
  [[ " $fg " == *" $main_word "* ]] && return 0
  title="$(herdr pane get "$pane" 2>/dev/null |
    jq -r '.result.pane | (.terminal_title_stripped // .terminal_title // "")')"
  if [[ "$main_word" == "pi" ]]; then
    [[ "$title" == *pi* || "$title" == *π* ]]
  else
    [[ "$title" == *"$main_word"* ]]
  fi
}

# tab_pane_ids <panes_json> <tab_id> — print the tab's pane ids, one per line.
tab_pane_ids() {
  print -rn -- "$1" | jq -r --arg tab "$2" \
    '.result.panes[] | select(.tab_id == $tab) | .pane_id'
}

# tab_main_pane <panes_json> <tab_id> <main_word> — print the first pane in the
# tab running main_word, or nothing.
tab_main_pane() {
  local panes="$1" tab="$2" main_word="$3" pane_id
  for pane_id in $(tab_pane_ids "$panes" "$tab"); do
    if pane_runs_main "$pane_id" "$main_word"; then
      print -rn -- "$pane_id"
      return 0
    fi
  done
  return 1
}

# tab_is_all_shells <panes_json> <tab_id> — exit 0 when every pane's foreground
# is a plain shell; a pane whose process-info fails counts as not a shell.
tab_is_all_shells() {
  local panes="$1" tab="$2" pane_id fg
  for pane_id in $(tab_pane_ids "$panes" "$tab"); do
    fg="$(pane_foreground "$pane_id")"
    case "${fg%% *}" in
      zsh|-zsh|bash|-bash|sh|-sh|fish|-fish) ;;
      *) return 1 ;;
    esac
  done
  return 0
}

# retire_labeled_tab <panes_json> <tab_id> <label> — clear a labeled tab that
# lost its role: all-shell tabs close (the bare-tab rule); tabs holding any
# non-shell work rename aside to <label>~old and are never closed.
retire_labeled_tab() {
  local panes="$1" tab="$2" label="$3"
  if tab_is_all_shells "$panes" "$tab"; then
    herdr tab close "$tab" > /dev/null || fail "herdr tab close failed for stale $label"
  else
    herdr tab rename "$tab" "${label}~old" > /dev/null ||
      fail "herdr tab rename failed for stale $label"
  fi
}

# retire_duplicate_labels <workspace_id> <canonical_tab_id> <label> — after the
# canonical tab is established, rename every OTHER tab carrying <label> to
# <label>~old; never close them.
retire_duplicate_labels() {
  local ws="$1" canonical="$2" label="$3"
  local listing panes tab_id
  listing="$(herdr tab list --workspace "$ws")" || fail "herdr tab list failed for $ws"
  panes="$(herdr pane list --workspace "$ws")" || fail "herdr pane list failed for $ws"
  for tab_id in $(print -rn -- "$listing" |
    jq -r --arg label "$label" '.result.tabs[] | select(.label == $label) | .tab_id'); do
    if [[ "$tab_id" != "$canonical" ]]; then
      retire_labeled_tab "$panes" "$tab_id" "$label"
    fi
  done
}

# create_tab <workspace_id> <label> <cwd> <main_cmd> — create the labeled tab:
# main_cmd pane at 0.75 (direction right) + shell pane at 0.25, both cwd'd.
create_tab() {
  local ws="$1" label="$2" cwd="$3" main_cmd="$4" created tab_id main_pane
  created="$(herdr tab create --workspace "$ws" --cwd "$cwd" --label "$label" --no-focus)" ||
    fail "herdr tab create failed for $label in $ws"
  tab_id="$(print -rn -- "$created" | jq -er '.result.tab.tab_id')" ||
    fail "tab create returned no tab_id for $label"
  main_pane="$(print -rn -- "$created" | jq -er '.result.root_pane.pane_id')" ||
    fail "tab create returned no root pane for $label"
  # right split at 0.75 leaves the original (main) pane holding 0.75
  herdr pane split --pane "$main_pane" --direction right --ratio 0.75 \
    --cwd "$cwd" --no-focus > /dev/null ||
    fail "herdr pane split failed for $label"
  herdr pane run "$main_pane" ${(z)main_cmd} > /dev/null ||
    fail "herdr pane run failed for $label"
}

# ensure_tab <workspace_id> <label> <cwd> <main_cmd> — adoption contract, in
# order: (1) a labeled plugin-shaped tab is left untouched; (2) any tab running
# the main process is adopted (renamed, split added when single-pane) and a
# second main process is never spawned — lowest tab number wins, the rest stay
# as they are; (3) a labeled all-shell tab is closed and recreated; (4) a
# labeled tab holding other work is renamed aside to <label>~old, never closed.
ensure_tab() {
  local ws="$1" label="$2" cwd="$3" main_cmd="$4"
  local main_word="${main_cmd%% *}"
  local listing panes labeled_tab adopt_tab tab_id count main_pane
  listing="$(herdr tab list --workspace "$ws")" || fail "herdr tab list failed for $ws"
  panes="$(herdr pane list --workspace "$ws")" || fail "herdr pane list failed for $ws"
  labeled_tab="$(print -rn -- "$listing" |
    jq -r --arg label "$label" \
      '[.result.tabs[] | select(.label == $label)] | first | .tab_id // empty')"

  # (1) the labeled tab already carries the plugin shape
  if [[ -n "$labeled_tab" ]]; then
    count="$(print -rn -- "$panes" | jq -r --arg tab "$labeled_tab" \
      '[.result.panes[] | select(.tab_id == $tab)] | length')"
    if [[ "$count" == 2 ]] && tab_main_pane "$panes" "$labeled_tab" "$main_word" > /dev/null; then
      retire_duplicate_labels "$ws" "$labeled_tab" "$label"
      return 0
    fi
  fi

  # (2) adopt the lowest-numbered tab running the main process
  adopt_tab=""
  for tab_id in $(print -rn -- "$listing" |
    jq -r '.result.tabs | sort_by(.number) | .[].tab_id'); do
    if tab_main_pane "$panes" "$tab_id" "$main_word" > /dev/null; then
      adopt_tab="$tab_id"
      break
    fi
  done
  if [[ -n "$adopt_tab" ]]; then
    if [[ -n "$labeled_tab" && "$labeled_tab" != "$adopt_tab" ]]; then
      retire_labeled_tab "$panes" "$labeled_tab" "$label"
    fi
    if [[ "$adopt_tab" != "$labeled_tab" ]]; then
      herdr tab rename "$adopt_tab" "$label" > /dev/null ||
        fail "herdr tab rename failed adopting $label"
    fi
    count="$(print -rn -- "$panes" | jq -r --arg tab "$adopt_tab" \
      '[.result.panes[] | select(.tab_id == $tab)] | length')"
    if [[ "$count" == 1 ]]; then
      main_pane="$(tab_main_pane "$panes" "$adopt_tab" "$main_word")" ||
        fail "adopted $label tab lost its $main_word pane"
      herdr pane split --pane "$main_pane" --direction right --ratio 0.75 \
        --cwd "$cwd" --no-focus > /dev/null ||
        fail "herdr pane split failed adopting $label"
    fi
    retire_duplicate_labels "$ws" "$adopt_tab" "$label"
    return 0
  fi

  # (3)/(4) no main process anywhere: clear a stale labeled tab, create fresh
  if [[ -n "$labeled_tab" ]]; then
    retire_labeled_tab "$panes" "$labeled_tab" "$label"
  fi
  create_tab "$ws" "$label" "$cwd" "$main_cmd"
  # After creation, the canonical tab is the newly labeled one; retire duplicates
  local canonical_tab
  canonical_tab="$(tab_id_by_label "$ws" "$label")"
  [[ -n "$canonical_tab" ]] && retire_duplicate_labels "$ws" "$canonical_tab" "$label"
}

# close_bare_tabs <workspace_id> <keep_tab_id>... — close every other tab in the
# workspace that is bare: exactly 1 pane whose foreground is a plain shell. A tab
# holding anything else is user work and is never closed; a pane whose
# process-info fails counts as not bare.
close_bare_tabs() {
  local ws="$1"; shift
  local keep=("$@") listing panes tab_id pane_id fg
  listing="$(herdr tab list --workspace "$ws")" || fail "herdr tab list failed for $ws"
  panes="$(herdr pane list --workspace "$ws")" || fail "herdr pane list failed for $ws"
  for tab_id in $(print -rn -- "$listing" | jq -r '.result.tabs[] | select(.pane_count == 1) | .tab_id'); do
    (( ${keep[(Ie)$tab_id]} )) && continue
    pane_id="$(print -rn -- "$panes" | jq -r --arg tab "$tab_id" \
      '[.result.panes[] | select(.tab_id == $tab)] | first | .pane_id // empty')"
    [[ -n "$pane_id" ]] || continue
    fg="$(pane_foreground "$pane_id")" || continue
    case "${fg%% *}" in
      zsh|-zsh|bash|-bash|sh|-sh|fish|-fish)
        herdr tab close "$tab_id" > /dev/null || fail "herdr tab close failed for bare $tab_id" ;;
    esac
  done
}

# focus_agent <workspace_id> — focus the agent tab and its pi pane.
focus_agent() {
  local ws="$1" tab_id panes shell_pane
  tab_id="$(tab_id_by_label "$ws" "agent")"
  [[ -n "$tab_id" ]] || fail "agent tab is missing after ensure_tab"
  herdr tab focus "$tab_id" > /dev/null || fail "herdr tab focus failed"
  # `pane focus` moves to a NEIGHBOR; the pi pane sits left of the shell pane,
  # so step left from the tab's rightmost (shell) pane. Focus is cosmetic:
  # never fail the run over it.
  panes="$(herdr pane list --workspace "$ws")" || fail "herdr pane list failed for $ws"
  shell_pane="$(print -rn -- "$panes" | jq -r --arg tab "$tab_id" \
    '[.result.panes[] | select(.tab_id == $tab) |
      select((.terminal_title_stripped // .terminal_title // "") | test("pi|\u03c0") | not)] |
      first | .pane_id // empty')"
  if [[ -n "$shell_pane" ]]; then
    herdr pane focus --pane "$shell_pane" --direction left > /dev/null 2>&1 || true
  fi
}

# main — apply both tabs under the workspace lock, then focus.
main() {
  local ws wt_path
  ws="$(event_workspace_id)"
  wt_path="$(event_worktree_path)"
  acquire_workspace_lock "$ws"
  trap '[[ -n "$LOCK_DIR" ]] && rmdir "$LOCK_DIR" 2>/dev/null || true' EXIT
  ensure_tab "$ws" "agent" "$wt_path" "pi"
  ensure_tab "$ws" "editor" "$wt_path" "nvim ."
  close_bare_tabs "$ws" "$(tab_id_by_label "$ws" "agent")" "$(tab_id_by_label "$ws" "editor")"
  focus_agent "$ws"
}

main "$@"
