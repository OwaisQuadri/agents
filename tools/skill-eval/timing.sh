set -u

_timing_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
TIMING_REPO_ROOT=$(CDPATH= cd -- "$_timing_dir/../.." && pwd -P)
: "${TIER_DISPATCH_BIN:=$TIMING_REPO_ROOT/tools/tier-dispatch/target/debug/tier-dispatch}"
: "${TIERS_FILE:=$TIMING_REPO_ROOT/config/model-tiers.json}"
: "${PI_BIN:=pi}"
: "${PI_ANTHROPIC_AUTH_EXTENSION:=$HOME/.pi/agent/extensions/pi-anthropic-auth}"
: "${PI_DONSETCH_EXTENSION:=$HOME/.pi/agent/extensions/npm/pi-extension.ts}"
: "${SKILL_EVAL_STATE_DIR:=$HOME/.local/state/skill-eval}"
TIMING_RUN_FILE=
TIMING_ARTIFACT=
TIMING_SLICE=
TIMING_CANDIDATE_FINGERPRINT=
TIMING_GENERATION_MS=null
TIMING_REQUESTED_TIER=
TIMING_FINAL_MODEL=
TIMING_ATTEMPTS='[]'

timing_now_ms() {
  python3 -c 'import time; print(time.monotonic_ns() // 1_000_000)'
}

timing_now_ns() {
  python3 -c 'import time; print(time.time_ns())'
}

timing_process_start() {
  ps -o lstart= -p "$1" 2>/dev/null
}

timing_owner_is_live() {
  local pid=$1 recorded_start=$2 current_start
  case "$pid" in ''|*[!0-9]*) return 1 ;; esac
  [ -n "$recorded_start" ] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  current_start=$(timing_process_start "$pid") || return 1
  [ -n "$current_start" ] && [ "$current_start" = "$recorded_start" ]
}

timing_timeout_bin() {
  command -v gtimeout 2>/dev/null || command -v timeout 2>/dev/null
}

timing_preflight() {
  local web_tools=${1:-false} resolved_pi
  command -v jq >/dev/null 2>&1 || { printf 'missing dependency: jq\n' >&2; return 2; }
  command -v python3 >/dev/null 2>&1 || { printf 'missing dependency: python3\n' >&2; return 2; }
  command -v shasum >/dev/null 2>&1 || { printf 'missing dependency: shasum\n' >&2; return 2; }
  [ -x "$TIER_DISPATCH_BIN" ] || { printf 'missing dependency: tier-dispatch (%s)\n' "$TIER_DISPATCH_BIN" >&2; return 2; }
  [ -f "$TIERS_FILE" ] || { printf 'missing dependency: TIERS_FILE (%s)\n' "$TIERS_FILE" >&2; return 2; }
  jq -e '.agents | type == "object"' "$TIERS_FILE" >/dev/null 2>&1 || { printf 'invalid TIERS_FILE (%s)\n' "$TIERS_FILE" >&2; return 2; }
  case "$PI_BIN" in
    */*) [ -x "$PI_BIN" ] || { printf 'missing dependency: PI_BIN (%s)\n' "$PI_BIN" >&2; return 2; } ;;
    *) resolved_pi=$(command -v "$PI_BIN" 2>/dev/null) || { printf 'missing dependency: PI_BIN (%s)\n' "$PI_BIN" >&2; return 2; }; [ -x "$resolved_pi" ] || { printf 'missing dependency: PI_BIN (%s)\n' "$PI_BIN" >&2; return 2; } ;;
  esac
  [ -e "$PI_ANTHROPIC_AUTH_EXTENSION" ] || { printf 'missing dependency: PI_ANTHROPIC_AUTH_EXTENSION (%s)\n' "$PI_ANTHROPIC_AUTH_EXTENSION" >&2; return 2; }
  if [ "$web_tools" = true ]; then
    [ -f "$PI_DONSETCH_EXTENSION" ] || { printf 'missing dependency: PI_DONSETCH_EXTENSION (%s)\n' "$PI_DONSETCH_EXTENSION" >&2; return 2; }
  fi
}

timing_tools() {
  local definition=$1 declared tool mapped tools= in_frontmatter=false
  while IFS= read -r line; do
    if [ "$line" = '---' ]; then
      if [ "$in_frontmatter" = false ]; then in_frontmatter=true; continue; fi
      break
    fi
    [ "$in_frontmatter" = true ] || continue
    case "$line" in tools:*) declared=${line#tools:}; break ;; esac
  done < "$definition"
  [ -n "${declared:-}" ] || { printf 'missing tools frontmatter: %s\n' "$definition" >&2; return 2; }
  declared=${declared//,/ }
  for tool in $declared; do
    case "$tool" in
      Read) mapped=read ;; Grep) mapped=grep ;; Glob) mapped=find ;; Bash) mapped=bash ;;
      Edit) mapped=edit ;; Write) mapped=write ;; WebSearch) mapped=web_search ;; WebFetch) mapped=web_fetch ;;
      *) printf 'unsupported frontmatter tool %s in %s\n' "$tool" "$definition" >&2; return 2 ;;
    esac
    tools=${tools:+$tools,}$mapped
  done
  printf '%s\n' "$tools"
}

timing_append() {
  python3 - "$TIMING_RUN_FILE" "$1" <<'PY'
import os
import sys

with open(sys.argv[1], "ab", buffering=0) as output:
    output.write(sys.argv[2].encode() + b"\n")
    output.flush()
    os.fsync(output.fileno())
PY
}

timing_records() {
  python3 - "$1" <<'PY'
import json
import os
import sys

records = []
for root, _, names in os.walk(sys.argv[1]):
    for name in names:
        if not name.startswith("custom-") or not name.endswith(".jsonl"):
            continue
        path = os.path.join(root, name)
        try:
            with open(path, encoding="utf-8") as source:
                lines = [line for line in source if line.strip()]
            start = json.loads(lines[0])
            complete = json.loads(lines[-1])
        except (IndexError, OSError, ValueError, json.JSONDecodeError):
            continue
        completed_at = str(complete.get("completed_at_ns", 0))
        if not completed_at.isascii() or not completed_at.isdigit():
            completed_at = "0"
        started_at = str(start.get("started_at_ns", 0))
        if not started_at.isascii() or not started_at.isdigit():
            started_at = "0"
        fields = [
            path,
            str(start.get("artifact", "")),
            str(start.get("candidate_fingerprint", "")),
            "true" if complete.get("type") == "complete" else "false",
            str(start.get("owner_pid", "")),
            str(start.get("owner_start", "")),
            completed_at,
            started_at,
        ]
        if not any("\0" in field for field in fields):
            records.append(fields)

for fields in sorted(records, key=lambda fields: (int(fields[6]), int(fields[7]), fields[0])):
    for field in fields:
        sys.stdout.buffer.write(field.encode() + b"\0")
PY
}

timing_prune() {
  local runs="$SKILL_EVAL_STATE_DIR/runs" file artifact fingerprint completed pid owner_start completed_at started_at
  local -a completed_files inactive_files inactive_started
  local completed_count=0 inactive_count=0 newest_inactive=-1 i
  mkdir -p "$runs" || return 2
  while IFS= read -r -d '' file && IFS= read -r -d '' artifact && IFS= read -r -d '' fingerprint && IFS= read -r -d '' completed && IFS= read -r -d '' pid && IFS= read -r -d '' owner_start && IFS= read -r -d '' completed_at && IFS= read -r -d '' started_at; do
    [ "$artifact" = "$TIMING_ARTIFACT" ] || continue
    if [ "$completed" = true ]; then
      completed_files[$completed_count]=$file
      completed_count=$((completed_count + 1))
      continue
    fi
    if timing_owner_is_live "$pid" "$owner_start"; then
      continue
    fi
    if [ "$fingerprint" != "$TIMING_CANDIDATE_FINGERPRINT" ]; then
      rm -f -- "$file" || return 2
      continue
    fi
    inactive_files[$inactive_count]=$file
    inactive_started[$inactive_count]=$started_at
    if [ "$newest_inactive" -lt 0 ] || [ "$started_at" -gt "${inactive_started[$newest_inactive]}" ]; then
      newest_inactive=$inactive_count
    fi
    inactive_count=$((inactive_count + 1))
  done < <(timing_records "$runs")

  for ((i = 0; i < inactive_count; i++)); do
    [ "$i" -eq "$newest_inactive" ] || rm -f -- "${inactive_files[$i]}" || return 2
  done
  for ((i = 0; i < completed_count - 100; i++)); do
    rm -f -- "${completed_files[$i]}" || return 2
  done
}

timing_case_begin() {
  TIMING_GENERATION_MS=null
  TIMING_REQUESTED_TIER=
  TIMING_FINAL_MODEL=
  TIMING_ATTEMPTS='[]'
}

timing_begin() {
  TIMING_ARTIFACT=$1
  TIMING_SLICE=$2
  local candidate=$3 runs safe_artifact owner_pid owner_start started_at_ns
  TIMING_CANDIDATE_FINGERPRINT=$(shasum -a 1 "$candidate" | awk '{print $1}')
  runs="$SKILL_EVAL_STATE_DIR/runs"
  mkdir -p "$runs" || return 2
  safe_artifact=$(printf '%s' "$TIMING_ARTIFACT" | tr -c '[:alnum:]_.-' '_')
  owner_pid=${BASHPID:-$$}
  owner_start=$(timing_process_start "$owner_pid") || return 2
  [ -n "$owner_start" ] || return 2
  started_at_ns=$(timing_now_ns)
  TIMING_RUN_FILE="$runs/custom-${safe_artifact}-${started_at_ns}-${owner_pid}-${RANDOM}.jsonl"
  timing_append "$(jq -cn --arg artifact "$TIMING_ARTIFACT" --arg slice "$TIMING_SLICE" --arg fingerprint "$TIMING_CANDIDATE_FINGERPRINT" --argjson owner_pid "$owner_pid" --arg owner_start "$owner_start" --argjson started_at_ns "$started_at_ns" '{type:"start",artifact:$artifact,slice:$slice,candidate_fingerprint:$fingerprint,owner_pid:$owner_pid,owner_start:$owner_start,started_at_ns:$started_at_ns}')" || return 2
  timing_prune || return 2
}

timing_dispatch() {
  local agent=$1 definition=$2 workdir=$3 input=$4 timeout_seconds=${5:-} timeout_bin=${6:-}
  local tier wrapper stderr body started status tools web_extension=
  tools=$(timing_tools "$definition") || return 2
  body=$(mktemp "${TMPDIR:-/tmp}/skill-eval-prompt.XXXXXX") || return 2
  awk '
    $0 == "---" { delimiters += 1; next }
    delimiters >= 2 { print }
    END { exit delimiters < 2 }
  ' "$definition" > "$body" || { rm -f "$body"; printf 'invalid frontmatter: %s\n' "$definition" >&2; return 2; }
  case ",$tools," in
    *,web_search,*|*,web_fetch,*)
      [ -f "$PI_DONSETCH_EXTENSION" ] || { rm -f "$body"; printf 'missing dependency: PI_DONSETCH_EXTENSION (%s)\n' "$PI_DONSETCH_EXTENSION" >&2; return 2; }
      web_extension=$PI_DONSETCH_EXTENSION
      ;;
  esac
  tier=$(jq -r --arg agent "$agent" '.agents[$agent] // empty' "$TIERS_FILE")
  [ -n "$tier" ] && [ "$tier" != null ] || { rm -f "$body"; printf 'missing tier for %s\n' "$agent" >&2; return 2; }
  wrapper=$(mktemp "${TMPDIR:-/tmp}/skill-eval-pi.XXXXXX") || { rm -f "$body"; return 2; }
  stderr=$(mktemp "${TMPDIR:-/tmp}/skill-eval-dispatch.XXXXXX") || { rm -f "$body" "$wrapper"; return 2; }
  printf '%s\n' '#!/usr/bin/env bash' > "$wrapper"
  printf 'cd %q || exit 2\n' "$workdir" >> "$wrapper"
  printf 'exec %q --no-extensions -e %q ' "$PI_BIN" "$PI_ANTHROPIC_AUTH_EXTENSION" >> "$wrapper"
  if [ -n "$web_extension" ]; then printf -- '-e %q ' "$web_extension" >> "$wrapper"; fi
  printf -- '--no-session --no-context-files --no-skills --no-prompt-templates --tools %q "$@"\n' "$tools" >> "$wrapper"
  chmod 700 "$wrapper" || { rm -f "$body" "$wrapper" "$stderr"; return 2; }
  started=$(timing_now_ms)
  if [ -n "$timeout_seconds" ]; then
    [ -n "$timeout_bin" ] || timeout_bin=$(timing_timeout_bin) || { rm -f "$body" "$wrapper" "$stderr"; printf 'no timeout command available\n' >&2; return 2; }
    "$timeout_bin" "$timeout_seconds" "$TIER_DISPATCH_BIN" --tiers-file "$TIERS_FILE" --tier "$tier" --system-prompt-file "$body" --input "$input" --dispatch-bin "$wrapper" 2>"$stderr"
  else
    "$TIER_DISPATCH_BIN" --tiers-file "$TIERS_FILE" --tier "$tier" --system-prompt-file "$body" --input "$input" --dispatch-bin "$wrapper" 2>"$stderr"
  fi
  status=$?
  TIMING_GENERATION_MS=$(( $(timing_now_ms) - started ))
  [ "$TIMING_GENERATION_MS" -ge 0 ] || TIMING_GENERATION_MS=0
  TIMING_REQUESTED_TIER=$tier
  TIMING_FINAL_MODEL=$(sed -n 's/^model_ran: //p' "$stderr" | tail -1)
  TIMING_ATTEMPTS=$(sed -n 's/^attempt: //p' "$stderr" | jq -sc . 2>/dev/null || printf '[]')
  cat "$stderr" >&2
  rm -f "$body" "$wrapper" "$stderr"
  return "$status"
}

timing_case() {
  local id=$1 score=$2 failure_mode=$3 started_ms=$4
  local total_ms=$(( $(timing_now_ms) - started_ms )) event
  [ "$total_ms" -ge 0 ] || total_ms=0
  event=$(jq -cn --arg artifact "$TIMING_ARTIFACT" --arg slice "$TIMING_SLICE" --arg fingerprint "$TIMING_CANDIDATE_FINGERPRINT" --arg id "$id" --argjson score "$score" --argjson failure_mode "$failure_mode" --argjson total_ms "$total_ms" --argjson generation_ms "$TIMING_GENERATION_MS" --arg tier "$TIMING_REQUESTED_TIER" --arg model "$TIMING_FINAL_MODEL" --argjson attempts "$TIMING_ATTEMPTS" '{type:"case",artifact:$artifact,slice:$slice,candidate_fingerprint:$fingerprint,id:$id,score:$score,failure_mode:$failure_mode,total_ms:$total_ms,generation_ms:$generation_ms,requested_tier:($tier|if length == 0 then null else . end),final_model:($model|if length == 0 then null else . end),attempts:$attempts,judge_ms:null,judge_model:null}')
  timing_append "$event" || return 2
  printf '%s\n' "$event"
}

timing_complete() {
  timing_append "$(jq -cn --arg artifact "$TIMING_ARTIFACT" --arg slice "$TIMING_SLICE" --arg fingerprint "$TIMING_CANDIDATE_FINGERPRINT" --argjson completed_at_ns "$(timing_now_ns)" '{type:"complete",artifact:$artifact,slice:$slice,candidate_fingerprint:$fingerprint,completed_at_ns:$completed_at_ns}')" || return 2
  timing_prune
}
