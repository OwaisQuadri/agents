#!/bin/zsh
# worktree-layout — herdr worktree.created/worktree.opened hook: apply the
# two-tab agent/editor layout to the event's workspace.
# Input:  HERDR_PLUGIN_EVENT_JSON — herdr EventEnvelope; workspace id and the
#         worktree path come from its data.
# Exit:   0 when the layout is applied or already correct; non-zero when herdr
#         or the event is unusable.
set -euo pipefail

# event_workspace_id — parse HERDR_PLUGIN_EVENT_JSON, print the workspace id.
event_workspace_id() { print "unimplemented" >&2; exit 42 }

# event_worktree_path — parse HERDR_PLUGIN_EVENT_JSON, print the worktree path.
event_worktree_path() { print "unimplemented" >&2; exit 42 }

# tab_id_by_label <workspace_id> <label> — print the tab id carrying <label> in
# the workspace, or nothing when absent.
tab_id_by_label() { print "unimplemented" >&2; exit 42 }

# ensure_tab <workspace_id> <label> <cwd> <main_cmd> — make the labeled tab
# exist with main_cmd at 0.75 and a shell at 0.25; replace a stale one.
ensure_tab() { print "unimplemented" >&2; exit 42 }

# focus_agent <workspace_id> — focus the agent tab and its main pane.
focus_agent() { print "unimplemented" >&2; exit 42 }

# main — apply both tabs, then focus.
main() { print "unimplemented" >&2; exit 42 }

main "$@"
