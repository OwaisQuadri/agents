set -euo pipefail

HQ_STATE=${HQ_STATE:-$HOME/.pi/agent/state/hq}
HQ_REPO=${HQ_REPO:-/Users/owaisquadri/Documents/agents}
HQ_PI_BIN=${HQ_PI_BIN:-pi}
NOTIFIER=${HQ_NOTIFIER:-/opt/homebrew/bin/terminal-notifier}
scripts_dir=${0:A:h}
triage_output=$HQ_STATE/triage-out.json

log() { print -r -- "[$(/bin/date '+%Y-%m-%d %H:%M:%S%z')] $1"; }

HQ_STATE=$HQ_STATE "$scripts_dir/scan.sh"
[[ -f $HQ_STATE/delta.json ]] || exit 0
[[ $(HQ_STATE=$HQ_STATE "$scripts_dir/scan.sh" --triage-due) == due ]] || exit 0

triage_prompt="You are HQ triage. Read only $HQ_STATE/delta.json, $HQ_STATE/registry.json, and the minimum bounded evidence those files identify. Herdr supplies active Pi panes and transcript paths. Never read more than the last 200 lines of a Pi transcript. Do not use git, remotes, session resume, or any write tool. Return exactly one JSON object with keys digest, gates, and notify. digest is markdown with unresolved gates first and a short per-project anomaly rundown. gates is an array of objects with id, createdAt, source, kind, subject, summary, evidence, urgency, isResolved, resolvedAt, and resolution. New gates must use source heartbeat, isResolved false, resolvedAt null, and resolution null. notify is null unless a gate needs immediate notification; then it is one short lowercase sentence."

dispatch_triage() {
  if [[ -n ${HQ_TIER_DISPATCH_BIN:-} ]]; then
    "$HQ_TIER_DISPATCH_BIN" "$@"
  else
    cargo run --quiet --manifest-path "$HQ_REPO/tools/tier-dispatch/Cargo.toml" -- "$@"
  fi
}

if ! dispatch_triage \
  --tiers-file "$HQ_REPO/config/model-tiers.json" \
  --tier T2 \
  --system-prompt-file "$HQ_REPO/skills/hq/SKILL.md" \
  --dispatch-bin "$HQ_PI_BIN" \
  --input "$triage_prompt" > "$triage_output"; then
  log "ERROR: bounded Pi triage through tier-dispatch failed"
  exit 1
fi

if ! notification=$(HQ_STATE=$HQ_STATE "$scripts_dir/scan.sh" --apply-triage "$triage_output"); then
  log "ERROR: Pi triage output was invalid"
  exit 1
fi
HQ_STATE=$HQ_STATE "$scripts_dir/scan.sh" --mark-triaged
if [[ -n $notification && -x $NOTIFIER ]]; then
  "$NOTIFIER" -group hq -title HQ -message "${notification#NOTIFY:}" >/dev/null 2>&1 || true
fi
