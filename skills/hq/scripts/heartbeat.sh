#!/bin/bash
set -euo pipefail

HQ_STATE="${HQ_STATE:-$HOME/.claude/hq}"
CLAUDE_BIN="${HQ_CLAUDE_BIN:-/Users/owaisquadri/.local/bin/claude}"
NOTIFIER="/opt/homebrew/bin/terminal-notifier"
SCRIPTS="$(cd "$(dirname "$0")" && pwd)"

HQ_STATE="$HQ_STATE" /bin/bash "$SCRIPTS/scan.sh"

DELTA="$HQ_STATE/delta.json"
[ -f "$DELTA" ] || exit 0

IS_DUE="$(/usr/bin/python3 - "$HQ_STATE" <<'PY'
import json, os, sys
from datetime import datetime

state_dir = sys.argv[1]
delta_at = os.path.getmtime(os.path.join(state_dir, "delta.json"))
last = None
try:
    with open(os.path.join(state_dir, "state.json")) as f:
        last = json.load(f).get("lastTriageAt")
except (OSError, ValueError):
    pass
if last is None:
    print("due")
else:
    stamp = datetime.strptime(last, "%Y-%m-%dT%H:%M:%S%z").timestamp()
    print("due" if delta_at > stamp else "done")
PY
)"
[ "$IS_DUE" = "due" ] || exit 0

TRIAGE_OUT="$HQ_STATE/triage-out.txt"
TRIAGE_PROMPT="You are HQ triage, a headless monitoring pass woken because the stage-1 scan found anomalies. Work only from files.

Read $HQ_STATE/delta.json and $HQ_STATE/registry.json. For each anomaly, read the minimum evidence only: for a job, tail ~/.claude/jobs/<id>/timeline.jsonl; for a session or workspace, tail the transcript path from the registry entry.

Hard limits: never write outside $HQ_STATE. Never touch git or any remote, never merge, never resolve or approve a gate, never message or resume another session.

For each anomaly needing a human decision, write one gate file at $HQ_STATE/gates/<yyyymmdd-hhmm>-<slug>.json shaped exactly as: {\"id\":\"<same as filename stem>\",\"createdAt\":\"<local iso with offset>\",\"source\":\"heartbeat\",\"kind\":\"plan_approval|permission_prompt|merge|signoff|destructive_action|failure_needs_decision\",\"subject\":\"<name or label>\",\"summary\":\"<1-3 lines>\",\"evidence\":[\"<absolute paths>\"],\"urgency\":\"notify_now|next_talk\",\"isResolved\":false,\"resolvedAt\":null,\"resolution\":null}. Reserve urgency notify_now for anomalies the user would want interrupted for right now; default to next_talk. If a gate for the same subject and kind already sits unresolved in $HQ_STATE/gates/, refresh that file instead of adding a duplicate.

Before writing the digest or any NOTIFY line, invoke the mouthpiece skill and follow it: both are messages the user reads. Gate files stay plain JSON. Rewrite $HQ_STATE/digest.md: unresolved gates first, each with its evidence paths, then a short per-project rundown of the anomalies.

If and only if at least one gate has urgency notify_now, print as your final line exactly one line starting with NOTIFY: followed by one short lowercase sentence. Otherwise print no line starting with NOTIFY."

cd /Users/owaisquadri/Documents/agents
"$CLAUDE_BIN" -p "$TRIAGE_PROMPT" --allowedTools Read Glob Grep Write Edit Skill > "$TRIAGE_OUT" || true

NOTIFY_LINE="$(grep -m1 '^NOTIFY:' "$TRIAGE_OUT" || true)"
if [ -n "$NOTIFY_LINE" ] && [ -x "$NOTIFIER" ]; then
  "$NOTIFIER" -group hq -title HQ -message "${NOTIFY_LINE#NOTIFY: }" >/dev/null 2>&1 || true
fi

/usr/bin/python3 - "$HQ_STATE" <<'PY'
import json, os, sys
from datetime import datetime

state_dir = sys.argv[1]
path = os.path.join(state_dir, "state.json")
try:
    with open(path) as f:
        state = json.load(f)
except (OSError, ValueError):
    state = {}
state["lastTriageAt"] = datetime.now().astimezone().strftime("%Y-%m-%dT%H:%M:%S%z")
with open(path, "w") as f:
    json.dump(state, f, indent=1)
PY
