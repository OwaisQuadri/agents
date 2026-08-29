#!/bin/bash
# gepa-due trigger — fired daily at 3pm by
# workflows/gepa-due/launchd/com.owaisquadri.gepa-due.plist (and once manually to
# test). Accumulation-triggered, not time-triggered, despite the daily cadence: the
# clock only decides WHEN to check, never whether to act. tools/gepa-due (a zero-LLM
# Rust check — see its own doc comment) decides that, every single day, and this
# script only escalates to opening a Pi session when it prints a non-empty due list.
# On a day nothing crosses the threshold, this exits after the free check — no herdr
# call, no worktree, no Pi invocation, no cost.
#
# Never drives a GUI Terminal (open -a Terminal / osascript) — herdr runs as a
# persistent background daemon reachable over its own socket, so every herdr command
# below is a plain CLI call, mirroring workflows/scheduled-ideation/scripts/trigger.sh's
# own proven mechanism (its own comment documents why the launchd-vs-TCC(Transparency,
# Consent, and Control) unattended-Automation risk does not apply here).
set -euo pipefail

REPO="${GEPA_DUE_REPO:-/Users/owaisquadri/Documents/agents}"
HERDR="${HERDR_BIN:-/opt/homebrew/bin/herdr}"
GEPA_DUE_BIN="$REPO/tools/gepa-due/target/release/gepa-due"
PRUNE_AFTER_DAYS=7
POLL_TAB_TIMEOUT_S=30
POLL_TAB_INTERVAL_S=2
RUN_STAMP="$(date +%Y-%m-%d-%H%M%S)"
BRANCH="gepa-due/$RUN_STAMP"
PROMPT_TIMEOUT_MS=600000

log() { printf '[%s] %s\n' "$(date '+%Y-%m-%d %H:%M:%S%z')" "$1"; }

log "gepa-due trigger starting"

# --- step 0: the ONLY step that runs every single day. Zero LLM cost, zero herdr
#     call. Build the checker on first run (missing binary, fresh checkout); reuse it
#     on every later run since it doesn't change day to day. ---
if [ ! -x "$GEPA_DUE_BIN" ]; then
  log "gepa-due binary not built yet, building it once"
  ( cd "$REPO/tools/gepa-due" && cargo build --release ) 2>&1 | while read -r l; do log "  $l"; done
fi

DUE_JSON="$("$GEPA_DUE_BIN" "$REPO")"
DUE_COUNT="$(printf '%s' "$DUE_JSON" | jq 'length')"

if [ "$DUE_COUNT" -eq 0 ]; then
  log "nothing due — exiting, no Pi invocation"
  exit 0
fi

log "due: $DUE_COUNT artifact(s) — $(printf '%s' "$DUE_JSON" | jq -c '[.[].artifact]')"

# --- step 1: herdr daemon must be reachable; a harmless no-op call proves it ---
if ! "$HERDR" worktree list --cwd "$REPO" > /dev/null 2>&1; then
  log "ERROR: herdr daemon unreachable (herdr worktree list failed) — is 'herdr server' running?"
  exit 1
fi

# --- step 2: prune gepa-due/* worktrees older than PRUNE_AFTER_DAYS ---
CUTOFF_EPOCH=$(( $(date +%s) - PRUNE_AFTER_DAYS * 86400 ))
"$HERDR" worktree list --cwd "$REPO" 2>/dev/null |
  jq -r '.result.worktrees[] | select(.branch // "" | startswith("gepa-due/")) | [.branch, .path] | @tsv' |
  while IFS=$'\t' read -r branch path; do
    stamp="${branch#gepa-due/}"
    stamp_date="${stamp:0:10}"
    stamp_epoch="$(date -j -f '%Y-%m-%d' "$stamp_date" +%s 2>/dev/null || echo 0)"
    if [ "$stamp_epoch" -gt 0 ] && [ "$stamp_epoch" -lt "$CUTOFF_EPOCH" ]; then
      log "pruning stale worktree $branch at $path (older than ${PRUNE_AFTER_DAYS}d)"
      git -C "$REPO" worktree remove --force "$path" 2>&1 | while read -r l; do log "  $l"; done || \
        log "WARN: prune failed for $path, leaving it — next run retries"
    fi
  done

# --- step 3: create a FRESH worktree for this run, same mechanism as
#     scheduled-ideation (never reuse; the branch stamp is unique to the second) ---
CREATE_JSON="$("$HERDR" worktree create --cwd "$REPO" --branch "$BRANCH" --base "${GEPA_DUE_BASE:-main}" --label gepa-due --focus 2>&1)"
WORKSPACE_ID="$(printf '%s' "$CREATE_JSON" | jq -r '.result.workspace.workspace_id // .result.worktree.open_workspace_id // empty' 2>/dev/null)"
if [ -z "$WORKSPACE_ID" ]; then
  log "ERROR: could not determine workspace id from herdr worktree create/open output: $CREATE_JSON"
  exit 1
fi
log "worktree ready: branch=$BRANCH workspace=$WORKSPACE_ID"

# --- step 4: find the agent pane, wait for pi to be its foreground process, then
#     seed the kickoff prompt with the actual due list — same proven mechanism as
#     scheduled-ideation's trigger.sh (see its own comments for the three pitfalls
#     this mirrors: capitalized Enter silently no-ops, agent prompt --wait's own Enter
#     can silently miss too, and the first enter can race a pane still registering
#     typed text) ---
AGENT_PANE=""
elapsed=0
while [ "$elapsed" -lt "$POLL_TAB_TIMEOUT_S" ]; do
  TABS_JSON="$("$HERDR" tab list --workspace "$WORKSPACE_ID" 2>/dev/null || echo '{}')"
  AGENT_TAB="$(printf '%s' "$TABS_JSON" | jq -r '.result.tabs[]? | select(.label == "agent") | .tab_id' | head -n1)"
  if [ -n "$AGENT_TAB" ]; then
    PANES_JSON="$("$HERDR" pane list --workspace "$WORKSPACE_ID" 2>/dev/null || echo '{}')"
    for pane_id in $(printf '%s' "$PANES_JSON" | jq -r --arg tab "$AGENT_TAB" '.result.panes[]? | select(.tab_id == $tab) | .pane_id'); do
      HAS_PI="$("$HERDR" pane process-info --pane "$pane_id" 2>/dev/null | jq -r '[.result.process_info.foreground_processes[]?.argv0] | any(. == "pi")' 2>/dev/null || echo 'false')"
      if [ "$HAS_PI" = "true" ]; then
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

KICKOFF="tools/gepa-due found $DUE_COUNT artifact(s) with real, unacted-on evidence since their last GEPA tune: $(printf '%s' "$DUE_JSON" | jq -c '.'). Read skills/ai-author/SKILL.md's GEPA loop (Reflect/Propose/Test/Decide/Record) and its 'applying frontier data' section, then decide what to do with this due list — run a real tuning pass on the highest-leverage artifact, or surface a short note on what's due for the owner to act on later. Never ship a mutation without going through the loop's own Decide gate; this prompt is a nudge, not an instruction to skip it. This is the scheduled gepa-due run started at $RUN_STAMP."
"$HERDR" pane send-text "$AGENT_PANE" "$KICKOFF"

log "confirming the agent actually started working"
SUBMIT_CONFIRMED=0
for attempt in 1 2 3; do
  "$HERDR" agent send-keys "$AGENT_PANE" enter
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
  log "WARN: agent wait did not settle cleanly within ${PROMPT_TIMEOUT_MS}ms — pane is still open and interactive, check it directly"
  exit 1
fi
log "SUCCESS: gepa-due session settled — branch=$BRANCH workspace=$WORKSPACE_ID"
