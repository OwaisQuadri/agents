#!/bin/zsh
set -euo pipefail

HQ_STATE=${HQ_STATE:-$HOME/.pi/agent/state/hq}
HQ_REPO=${HQ_REPO:-/Users/owaisquadri/Documents/agents}
HQ_PI_BIN=${HQ_PI_BIN:-pi}
NOTIFIER=${HQ_NOTIFIER:-/opt/homebrew/bin/terminal-notifier}
scripts_dir=${0:A:h}
triage_output=$HQ_STATE/triage-out.json
triage_failure=$HQ_STATE/triage-failed-at

log() { print -r -- "[$(/bin/date '+%Y-%m-%dT%H:%M:%S%z')] $1"; }
mark_triaged() { HQ_STATE=$HQ_STATE "$scripts_dir/scan.sh" --mark-triaged; }
mark_failed() { /bin/date -u '+%Y-%m-%dT%H:%M:%SZ' > "$triage_failure"; }

HQ_STATE=$HQ_STATE "$scripts_dir/scan.sh"
[[ -f $HQ_STATE/delta.json ]] || exit 0
[[ $(HQ_STATE=$HQ_STATE "$scripts_dir/scan.sh" --triage-due) == due ]] || exit 0
if [[ -f $triage_failure && -n $(/usr/bin/find "$triage_failure" -mmin -360 -print) ]]; then
  log "triage cooldown active after the last failure"
  exit 0
fi

triage_model=$(jq -er '.tiers.T2.pi.model' "$HQ_REPO/config/model-tiers.json")
triage_thinking=$(jq -er '.tiers.T2.pi.thinking' "$HQ_REPO/config/model-tiers.json")
triage_system_prompt='You are an unattended anomaly triage pass. Use only the read tool. Read only the two state files named in the user prompt. Return one fenced JSON object with digest, gates, and notify. gates is an array. notify is null or one short lowercase sentence. Do not write, run commands, use a session, contact a remote, or make decisions for the user.'
triage_prompt="Read $HQ_STATE/delta.json and $HQ_STATE/registry.json. Summarize only the anomalies in delta.json. For a human decision, add a gate with id, createdAt, source, kind, subject, summary, evidence, urgency, isResolved, resolvedAt, and resolution. A gate id uses only letters, numbers, dots, underscores, and hyphens. evidence is always an array of strings. New gates use source heartbeat, isResolved false, resolvedAt null, and resolution null. digest lists unresolved gates first. Do not read any other file."

if ! RAG_RECALL=0 "$HQ_PI_BIN" -p \
  --model "$triage_model" \
  --thinking "$triage_thinking" \
  --system-prompt "$triage_system_prompt" \
  --no-session \
  --no-skills \
  --no-extensions \
  --no-context-files \
  --tools read \
  "$triage_prompt" > "$triage_output"; then
  log "ERROR: bounded Pi triage failed"
  mark_failed
  exit 1
fi

if ! notification=$(HQ_STATE=$HQ_STATE "$scripts_dir/scan.sh" --apply-triage "$triage_output"); then
  log "ERROR: Pi triage output was invalid"
  mark_failed
  exit 1
fi
mark_triaged
if [[ -n $notification && -x $NOTIFIER ]]; then
  "$NOTIFIER" -group hq -title HQ -message "${notification#NOTIFY:}" >/dev/null 2>&1 || true
fi
