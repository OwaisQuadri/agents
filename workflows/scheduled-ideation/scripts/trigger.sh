#!/bin/bash
# scheduled-ideation trigger — fired daily at 3pm by
# workflows/scheduled-ideation/launchd/com.owaisquadri.scheduled-ideation.plist (and
# once manually to test). Mechanizable-only: no LLM judgment happens in this script.
#
# Never drives a GUI Terminal (open -a Terminal / osascript) — herdr runs as a
# persistent background daemon reachable over its own socket, so every herdr command
# below is a plain CLI call. This is why the launchd-vs-TCC(Transparency, Consent, and
# Control) unattended-Automation risk that blocks GUI-scripting launchd jobs does not
# apply here; see .context/scheduled-ideation/research.md in the worktree that
# authored this for the full research trail.
set -euo pipefail

REPO="${SCHEDULED_IDEATION_REPO:-/Users/owaisquadri/Documents/agents}"
HERDR="${HERDR_BIN:-/opt/homebrew/bin/herdr}"
PRUNE_AFTER_DAYS=7
POLL_TAB_TIMEOUT_S=30
POLL_TAB_INTERVAL_S=2
POLL_DIGEST_TIMEOUT_S=90
POLL_DIGEST_INTERVAL_S=10
TODAY="$(date +%Y-%m-%d)"
RUN_STAMP="$(date +%Y-%m-%d-%H%M%S)"
BRANCH="ideation/$RUN_STAMP"
DIGEST_PATH=".context/scheduled-ideation-digest.md"
PROMPT_TIMEOUT_MS=600000

log() { printf '[%s] %s\n' "$(date '+%Y-%m-%d %H:%M:%S%z')" "$1"; }

log "scheduled-ideation trigger starting for $TODAY"

# --- step 1: herdr daemon must be reachable; a harmless no-op call proves it ---
if ! "$HERDR" worktree list --cwd "$REPO" > /dev/null 2>&1; then
  log "ERROR: herdr daemon unreachable (herdr worktree list failed) — is 'herdr server' running?"
  exit 1
fi

# --- step 2: prune ideation/* worktrees older than PRUNE_AFTER_DAYS ---
CUTOFF_EPOCH=$(( $(date +%s) - PRUNE_AFTER_DAYS * 86400 ))
"$HERDR" worktree list --cwd "$REPO" 2>/dev/null |
  jq -r '.result.worktrees[] | select(.branch // "" | startswith("ideation/")) | [.branch, .path] | @tsv' |
  while IFS=$'\t' read -r branch path; do
    stamp="${branch#ideation/}"
    stamp_date="${stamp:0:10}"
    stamp_epoch="$(date -j -f '%Y-%m-%d' "$stamp_date" +%s 2>/dev/null || echo 0)"
    if [ "$stamp_epoch" -gt 0 ] && [ "$stamp_epoch" -lt "$CUTOFF_EPOCH" ]; then
      log "pruning stale worktree $branch at $path (older than ${PRUNE_AFTER_DAYS}d)"
      git -C "$REPO" worktree remove --force "$path" 2>&1 | while read -r l; do log "  $l"; done || \
        log "WARN: prune failed for $path, leaving it — next run retries"
    fi
  done

# --- step 3: create a FRESH worktree for this run (never reuse — every invocation,
#     scheduled or manual, gets its own; the branch stamp is unique to the second so
#     even two runs in the same minute never collide). Fires the existing
#     worktree.created hook, which lays out the standard agent+editor tabs with bare
#     `pi` in the agent pane. ---
CREATE_JSON="$("$HERDR" worktree create --cwd "$REPO" --branch "$BRANCH" --base main --label ideation --focus 2>&1)"
WORKSPACE_ID="$(printf '%s' "$CREATE_JSON" | jq -r '.result.workspace.workspace_id // .result.worktree.open_workspace_id // empty' 2>/dev/null)"
if [ -z "$WORKSPACE_ID" ]; then
  log "ERROR: could not determine workspace id from herdr worktree create/open output: $CREATE_JSON"
  exit 1
fi
log "worktree ready: branch=$BRANCH workspace=$WORKSPACE_ID"

# --- step 4: find the agent pane the worktree.created hook already created, wait
#     for pi to be its foreground process, then seed the kickoff prompt ---
AGENT_PANE=""
elapsed=0
while [ "$elapsed" -lt "$POLL_TAB_TIMEOUT_S" ]; do
  TABS_JSON="$("$HERDR" tab list --workspace "$WORKSPACE_ID" 2>/dev/null || echo '{}')"
  AGENT_TAB="$(printf '%s' "$TABS_JSON" | jq -r '.result.tabs[]? | select(.label == "agent") | .tab_id' | head -n1)"
  if [ -n "$AGENT_TAB" ]; then
    PANES_JSON="$("$HERDR" pane list --workspace "$WORKSPACE_ID" 2>/dev/null || echo '{}')"
    for pane_id in $(printf '%s' "$PANES_JSON" | jq -r --arg tab "$AGENT_TAB" '.result.panes[]? | select(.tab_id == $tab) | .pane_id'); do
      FG="$("$HERDR" pane process-info --pane "$pane_id" 2>/dev/null | jq -r '.result.process_info.foreground_processes[0].argv0 // empty' 2>/dev/null || echo '')"
      if [ "$FG" = "pi" ]; then
        AGENT_PANE="$pane_id"
        break 2
      fi
    done
  fi
  sleep "$POLL_TAB_INTERVAL_S"
  elapsed=$(( elapsed + POLL_TAB_INTERVAL_S ))
done

if [ -z "$AGENT_PANE" ]; then
  log "ERROR: agent tab never came up with a live pi foreground process within ${POLL_TAB_TIMEOUT_S}s"
  exit 1
fi
log "typing kickoff prompt into pane $AGENT_PANE"

# Three proven-live pitfalls to avoid here, all confirmed by hand against a real pane:
# 1. `pane send-keys <pane> Enter` (capitalized) silently no-ops — it neither submits
#    nor errors, leaving the text sitting prefilled forever. The working key name is
#    lowercase `enter`, sent through `agent send-keys`, not `pane send-keys`.
# 2. `agent prompt --wait`'s own built-in Enter hit the same silent-no-submit failure
#    in this environment, yet still returned a clean (non-error) `idle` status instead
#    of `agent_prompt_stalled` — its 5s observed-state-change guard did not catch it.
#    Trusting that return value would have made the trigger call this run
#    "successful" while it never actually started. Never do that again: submit and
#    verify as two independently-checked steps instead of one trusted call.
# 3. Even the fixed `send-text` + `agent send-keys enter` pair above is a race: on a
#    real scheduled run (2026-08-28, first live launchd fire) the very first `enter`
#    landed before the pane finished registering the typed text and was silently
#    dropped — same symptom as (1), text left sitting prefilled. A second manual
#    `agent send-keys enter` right after submitted it immediately. So the send is
#    retried, not just sent once: each attempt re-sends `enter` (harmless no-op if
#    the previous one actually landed and a turn is now running — see below) and
#    waits a short beat for a real transition to "working" before giving up on it.
KICKOFF="Run workflows/scheduled-ideation/ (the Workflow tool, no args needed) and write its returned digest verbatim to $DIGEST_PATH in this worktree. This is the scheduled ideation run started at $RUN_STAMP."
"$HERDR" pane send-text "$AGENT_PANE" "$KICKOFF"

log "confirming the agent actually started working"
SUBMIT_CONFIRMED=0
for attempt in 1 2 3; do
  "$HERDR" agent send-keys "$AGENT_PANE" enter
  # a resend while a turn is already running just replays into the chat input, which
  # Pi ignores mid-turn — harmless. "agent wait --until working" is still the correct
  # check either way: it is already true if attempt 1 landed, so it returns at once.
  if "$HERDR" agent wait "$AGENT_PANE" --until working --timeout 8000 2>&1 | while read -r l; do log "  $l"; done; then
    SUBMIT_CONFIRMED=1
    break
  fi
  log "submit attempt $attempt did not reach working within 8s, retrying the enter key"
done
if [ "$SUBMIT_CONFIRMED" -ne 1 ]; then
  log "ERROR: agent never transitioned to working after 3 submit attempts — giving up"
  exit 1
fi
log "working confirmed; waiting for it to settle (idle/done/blocked)"
if ! "$HERDR" agent wait "$AGENT_PANE" --timeout "$PROMPT_TIMEOUT_MS" 2>&1 | while read -r l; do log "  $l"; done; then
  log "WARN: agent wait did not settle cleanly within ${PROMPT_TIMEOUT_MS}ms — checking for a digest anyway"
fi

# --- step 5: verify the digest actually lands before calling this run succeeded —
#     a scheduled AI agent job exiting 0 while silently producing nothing is a
#     documented 2026 failure mode this guard exists to catch ---
WORKTREE_PATH="$(printf '%s' "$CREATE_JSON" | jq -r '.result.worktree.path // .result.path // empty' 2>/dev/null)"
if [ -z "$WORKTREE_PATH" ]; then
  WORKTREE_PATH="$(git -C "$REPO" worktree list --porcelain | awk -v b="refs/heads/$BRANCH" '$1=="worktree"{p=$2} $1=="branch" && $2==b{print p}')"
fi
DIGEST_FILE="$WORKTREE_PATH/$DIGEST_PATH"

elapsed=0
while [ "$elapsed" -lt "$POLL_DIGEST_TIMEOUT_S" ]; do
  if [ -s "$DIGEST_FILE" ]; then
    log "SUCCESS: digest written at $DIGEST_FILE ($(wc -l < "$DIGEST_FILE") lines)"
    exit 0
  fi
  sleep "$POLL_DIGEST_INTERVAL_S"
  elapsed=$(( elapsed + POLL_DIGEST_INTERVAL_S ))
done

log "WARN: digest not found or empty at $DIGEST_FILE after agent prompt settled + ${POLL_DIGEST_TIMEOUT_S}s extra — the pane is still open and interactive, check it directly"
exit 1
