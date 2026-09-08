#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/skill-eval-timing-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/bin" "$TMP/from-another-cwd" "$TMP/state"

cat > "$TMP/bin/pi" <<'PI'
#!/usr/bin/env bash
if [ "${FAIL_PI_DISPATCH:-0}" = 1 ]; then exit 42; fi
{
  printf 'CALL\n'
  for argument in "$@"; do printf '%s\n' "$argument"; done
} >> "${PI_ARGS_LOG:?}"
printf 'synthetic agent output\n'
PI
chmod +x "$TMP/bin/pi"

cat > "$TMP/bin/tier-dispatch" <<'DISPATCH'
#!/usr/bin/env bash
set -eu
if [ "${HANG_DISPATCH:-0}" = 1 ]; then exec sleep 30; fi
wrapper=
input=
system_prompt=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dispatch-bin) wrapper="$2"; shift 2 ;;
    --input) input="$2"; shift 2 ;;
    --system-prompt-file) system_prompt="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[ -n "$wrapper" ] || exit 2
[ -n "$system_prompt" ] || exit 2
cp "$system_prompt" "${FAKE_SYSTEM_PROMPT:?}"
"$wrapper" "$input"
printf '%s\n' 'attempt: {"model":"fake/model","thinking":"low","elapsed_ms":1,"result":"success"}' >&2
printf '%s\n' 'model_ran: fake/model' >&2
DISPATCH
chmod +x "$TMP/bin/tier-dispatch"
: > "$TMP/auth-extension.ts"
: > "$TMP/donsetch-extension.ts"
: > "$TMP/pi-args.log"
for command in maestro java; do
  printf '#!/usr/bin/env bash\nexit 0\n' > "$TMP/bin/$command"
  chmod +x "$TMP/bin/$command"
done
printf '#!/usr/bin/env bash\nprintf \"Booted\\n\"\n' > "$TMP/bin/xcrun"
chmod +x "$TMP/bin/xcrun"

export SKILL_EVAL_STATE_DIR="$TMP/state"
export TIER_DISPATCH_BIN="$TMP/bin/tier-dispatch"
export TIERS_FILE="$REPO_ROOT/config/model-tiers.json"
export PI_BIN="$TMP/bin/pi"
export PI_ANTHROPIC_AUTH_EXTENSION="$TMP/auth-extension.ts"
export PI_DONSETCH_EXTENSION="$TMP/donsetch-extension.ts"
export PI_ARGS_LOG="$TMP/pi-args.log"
export FAKE_SYSTEM_PROMPT="$TMP/fake-system-prompt"
export PATH="$TMP/bin:$PATH"

run_harness() {
  local name=$1 script=$2 status=0 stdout stderr file
  stdout="$TMP/$name.stdout"
  stderr="$TMP/$name.stderr"
  if (
    cd "$TMP/from-another-cwd"
    bash "$REPO_ROOT/$script"
  ) >"$stdout" 2>"$stderr"; then
    :
  else
    status=$?
  fi

  file="$(find "$TMP/state/runs" -type f -name "custom-$name-*.jsonl" -print)"
  [ "$(printf '%s\n' "$file" | sed '/^$/d' | wc -l | tr -d ' ')" -eq 1 ]
  [ "$(jq -s '[.[] | select(.type == "start")] | length' "$file")" -eq 1 ]
  [ "$(jq -s '[.[] | select(.type == "complete")] | length' "$file")" -eq 1 ]
  [ "$(jq -s '[.[] | select(.type == "case")] | length' "$file")" -gt 0 ]
  jq -e '
    select(.type == "case")
    | .total_ms >= 0
    and .generation_ms >= 0
    and .requested_tier != ""
    and .final_model == "fake/model"
    and (.attempts | length == 1)
    and .attempts[0].model == "fake/model"
  ' "$stdout" >/dev/null

  case "$name" in
    anchor-verifier|maestro-tester|spec-tester) [ "$status" -ne 0 ] ;;
    *) [ "$status" -eq 0 ] ;;
  esac
}

for harness in anchor-verifier code-reviewer debugger implementer log-summarizer maestro-tester spec-tester web-research-summarizer; do
  run_harness "$harness" "agents/$harness/evals/run.sh"
done

missing_pi_status=0
if (
  cd "$TMP/from-another-cwd"
  SKILL_EVAL_STATE_DIR="$TMP/missing-pi-state" PI_BIN="$TMP/bin/absent-pi" bash "$REPO_ROOT/agents/implementer/evals/run.sh"
) >"$TMP/missing-pi.stdout" 2>"$TMP/missing-pi.stderr"; then :; else missing_pi_status=$?; fi
[ "$missing_pi_status" -eq 2 ]
grep -q 'missing dependency: PI_BIN' "$TMP/missing-pi.stderr"
[ ! -d "$TMP/missing-pi-state/runs" ]

printf '{}\n' > "$TMP/invalid-tiers.json"
invalid_tiers_status=0
if (
  cd "$TMP/from-another-cwd"
  SKILL_EVAL_STATE_DIR="$TMP/invalid-tiers-state" TIERS_FILE="$TMP/invalid-tiers.json" bash "$REPO_ROOT/agents/implementer/evals/run.sh"
) >"$TMP/invalid-tiers.stdout" 2>"$TMP/invalid-tiers.stderr"; then :; else invalid_tiers_status=$?; fi
[ "$invalid_tiers_status" -eq 2 ]
grep -q 'invalid TIERS_FILE' "$TMP/invalid-tiers.stderr"
[ ! -d "$TMP/invalid-tiers-state/runs" ]

: > "$TMP/state-creation-failure"
state_creation_status=0
if (
  cd "$TMP/from-another-cwd"
  SKILL_EVAL_STATE_DIR="$TMP/state-creation-failure" bash "$REPO_ROOT/agents/implementer/evals/run.sh"
) >"$TMP/state-creation.stdout" 2>"$TMP/state-creation.stderr"; then :; else state_creation_status=$?; fi
[ "$state_creation_status" -eq 2 ]
! grep -q 'slice=' "$TMP/state-creation.stdout" "$TMP/state-creation.stderr"

missing_donsetch_status=0
if (
  cd "$TMP/from-another-cwd"
  SKILL_EVAL_STATE_DIR="$TMP/missing-donsetch-state" PI_DONSETCH_EXTENSION="$TMP/absent-donsetch-extension.ts" bash "$REPO_ROOT/agents/web-research-summarizer/evals/run.sh"
) >"$TMP/missing-donsetch.stdout" 2>"$TMP/missing-donsetch.stderr"; then :; else missing_donsetch_status=$?; fi
[ "$missing_donsetch_status" -eq 2 ]
grep -q 'missing dependency: PI_DONSETCH_EXTENSION' "$TMP/missing-donsetch.stderr"
[ ! -d "$TMP/missing-donsetch-state/runs" ]

for harness in code-reviewer implementer log-summarizer web-research-summarizer; do
  ungraded_status=0
  if (
    cd "$TMP/from-another-cwd"
    FAIL_PI_DISPATCH=1 SKILL_EVAL_STATE_DIR="$TMP/ungraded-$harness-state" bash "$REPO_ROOT/agents/$harness/evals/run.sh"
  ) >"$TMP/ungraded-$harness.stdout" 2>"$TMP/ungraded-$harness.stderr"; then :; else ungraded_status=$?; fi
  [ "$ungraded_status" -eq 2 ]
  grep -q 'ungraded=[1-9]' "$TMP/ungraded-$harness.stderr"
done

holdout_status=0
if (
  cd "$TMP/from-another-cwd"
  SKILL_EVAL_STATE_DIR="$TMP/implementer-holdout-state" bash "$REPO_ROOT/agents/implementer/evals/run.sh" --holdout
) >"$TMP/implementer-holdout.stdout" 2>"$TMP/implementer-holdout.stderr"; then :; else holdout_status=$?; fi
[ "$holdout_status" -eq 0 ]
holdout_file="$(find "$TMP/implementer-holdout-state/runs" -type f -name 'custom-implementer-*.jsonl' -print)"
jq -e 'select(.type == "start") | .slice == "holdout"' "$holdout_file" >/dev/null
[ "$(jq -r 'select(.type == "case") | .id' "$holdout_file")" = i5 ]
[ "$(jq -s '[.[] | select(.type == "case")] | length' "$holdout_file")" -eq 1 ]

source "$REPO_ROOT/tools/skill-eval/timing.sh"
cat > "$TMP/read-only-debugger.md" <<'EOF'
---
name: debugger
tools: Read
---
EOF
cat > "$TMP/body-only-debugger.md" <<'EOF'
---
name: debugger
tools: Read
---
Body-only dispatcher sentinel.
EOF
timing_preflight
timing_begin tool-allowlist nonholdout "$REPO_ROOT/agents/debugger/debugger.md"
timing_dispatch debugger "$REPO_ROOT/agents/debugger/debugger.md" "$TMP/from-another-cwd" synthetic >/dev/null 2>/dev/null
timing_dispatch debugger "$TMP/read-only-debugger.md" "$TMP/from-another-cwd" synthetic >/dev/null 2>/dev/null
timing_dispatch debugger "$TMP/body-only-debugger.md" "$TMP/from-another-cwd" synthetic >/dev/null 2>/dev/null
timing_complete
python3 - "$TMP/pi-args.log" "$TMP/auth-extension.ts" "$TMP/donsetch-extension.ts" "$TMP/fake-system-prompt" <<'PY'
import sys

lines = open(sys.argv[1], encoding="utf-8").read().splitlines()
calls = []
for line in lines:
    if line == "CALL":
        calls.append([])
    else:
        calls[-1].append(line)
assert calls, "fake Pi received no calls"

def tools(call):
    index = call.index("--tools")
    return call[index + 1]

assert any(tools(call) == "read,edit,bash,grep,find" for call in calls), calls
assert any(tools(call) == "read" for call in calls), calls
web = next(call for call in calls if tools(call) == "web_search,web_fetch,read")
expected = [
    "--no-extensions", "-e", sys.argv[2], "-e", sys.argv[3],
    "--no-session", "--no-context-files", "--no-skills", "--no-prompt-templates",
    "--tools", "web_search,web_fetch,read",
]
assert web[:len(expected)] == expected, web
prompt = open(sys.argv[4], encoding="utf-8").read()
assert prompt == "Body-only dispatcher sentinel.\n", prompt
assert "---" not in prompt and "tools:" not in prompt, prompt
PY

mkdir "$TMP/append-target"
TIMING_RUN_FILE="$TMP/append-target"
if timing_append '{"type":"append-failure"}' 2>/dev/null; then
  echo 'timing_append unexpectedly accepted a directory' >&2
  exit 1
fi

parallel_run() (
  cd "$TMP/from-another-cwd"
  source "$REPO_ROOT/tools/skill-eval/timing.sh"
  timing_begin parallel-test nonholdout "$REPO_ROOT/agents/debugger/debugger.md"
  : > "$TMP/parallel-ready"
  while [ ! -f "$TMP/parallel-release" ]; do sleep 0.01; done
  timing_case_begin
  started="$(timing_now_ms)"
  timing_case parallel-first 1 null "$started" >/dev/null
  timing_complete
)

parallel_run &
first=$!
for _ in $(seq 1 100); do [ -f "$TMP/parallel-ready" ] && break; sleep 0.01; done
[ -f "$TMP/parallel-ready" ]
(
  cd "$TMP/from-another-cwd"
  source "$REPO_ROOT/tools/skill-eval/timing.sh"
  timing_begin parallel-test nonholdout "$REPO_ROOT/agents/implementer/implementer.md"
  timing_case_begin
  started="$(timing_now_ms)"
  timing_case parallel-second 1 null "$started" >/dev/null
  timing_complete
)
parallel_files="$(find "$TMP/state/runs" -type f -name 'custom-parallel-test-*.jsonl' -print)"
[ "$(printf '%s\n' "$parallel_files" | sed '/^$/d' | wc -l | tr -d ' ')" -eq 2 ]
: > "$TMP/parallel-release"
wait "$first"

(
  cd "$TMP/from-another-cwd"
  source "$REPO_ROOT/tools/skill-eval/timing.sh"
  timing_begin attribution-test nonholdout "$REPO_ROOT/agents/debugger/debugger.md"
  timing_case_begin
  started="$(timing_now_ms)"
  timing_dispatch debugger "$REPO_ROOT/agents/debugger/debugger.md" "$TMP/from-another-cwd" synthetic >/dev/null 2>/dev/null
  timing_case dispatched 1 null "$started" >/dev/null
  timing_case_begin
  started="$(timing_now_ms)"
  timing_case nondispatch 1 null "$started" >/dev/null
  timing_complete
)
attribution_file="$(find "$TMP/state/runs" -type f -name 'custom-attribution-test-*.jsonl' -print)"
jq -e 'select(.type == "case" and .id == "dispatched") | .generation_ms >= 0 and .requested_tier != null and .final_model == "fake/model" and (.attempts | length == 1)' "$attribution_file" >/dev/null
jq -e 'select(.type == "case" and .id == "nondispatch") | .generation_ms == null and .requested_tier == null and .final_model == null and .attempts == []' "$attribution_file" >/dev/null

retention_runs="$TMP/state/runs"
for index in $(seq 1 101); do
  file="$retention_runs/custom-retention-test-$index.jsonl"
  printf '{"type":"start","artifact":"retention-test","slice":"nonholdout","candidate_fingerprint":"retention","owner_pid":999999,"owner_start":"dead","started_at_ns":%s}\n' "$index" > "$file"
  printf '{"type":"complete","artifact":"retention-test","slice":"nonholdout","candidate_fingerprint":"retention","completed_at_ns":%s}\n' "$index" >> "$file"
done
(
  source "$REPO_ROOT/tools/skill-eval/timing.sh"
  TIMING_ARTIFACT=retention-test
  TIMING_CANDIDATE_FINGERPRINT=retention
  timing_prune
)
[ "$(find "$retention_runs" -type f -name 'custom-retention-test-*.jsonl' -print | wc -l | tr -d ' ')" -eq 100 ]
[ ! -f "$retention_runs/custom-retention-test-1.jsonl" ]
[ -f "$retention_runs/custom-retention-test-101.jsonl" ]

for index in $(seq 1 150); do
  file="$retention_runs/custom-incomplete-test-same-$index.jsonl"
  printf '{"type":"start","artifact":"incomplete-test","slice":"nonholdout","candidate_fingerprint":"same","owner_pid":999999,"owner_start":"dead","started_at_ns":%s}\n' "$index" > "$file"
done
printf '{"type":"start","artifact":"incomplete-test","slice":"nonholdout","candidate_fingerprint":"stale","owner_pid":999999,"owner_start":"dead","started_at_ns":151}\n' > "$retention_runs/custom-incomplete-test-stale.jsonl"
active_pid=${BASHPID:-$$}
active_start=$(timing_process_start "$active_pid")
printf '{"type":"start","artifact":"incomplete-test","slice":"nonholdout","candidate_fingerprint":"stale","owner_pid":%s,"owner_start":%s,"started_at_ns":152}\n' \
  "$active_pid" "$(jq -Rn --arg value "$active_start" '$value')" > "$retention_runs/custom-incomplete-test-active.jsonl"
(
  source "$REPO_ROOT/tools/skill-eval/timing.sh"
  TIMING_ARTIFACT=incomplete-test
  TIMING_CANDIDATE_FINGERPRINT=same
  timing_prune
)
incomplete_files="$(find "$retention_runs" -type f -name 'custom-incomplete-test-*.jsonl' -print)"
[ "$(printf '%s\n' "$incomplete_files" | sed '/^$/d' | wc -l | tr -d ' ')" -eq 2 ]
[ -f "$retention_runs/custom-incomplete-test-same-150.jsonl" ]
[ ! -f "$retention_runs/custom-incomplete-test-same-149.jsonl" ]
[ ! -f "$retention_runs/custom-incomplete-test-stale.jsonl" ]
[ -f "$retention_runs/custom-incomplete-test-active.jsonl" ]

timeout_status=0
timeout_started="$(python3 -c 'import time; print(time.monotonic())')"
if (
  cd "$TMP/from-another-cwd"
  HANG_DISPATCH=1 DEBUGGER_DISPATCH_TIMEOUT_SECONDS=0.1 SKILL_EVAL_STATE_DIR="$TMP/timeout-state" TIER_DISPATCH_BIN="$TMP/bin/tier-dispatch" TIERS_FILE="$REPO_ROOT/config/model-tiers.json" PI_BIN="$TMP/bin/pi" bash "$REPO_ROOT/agents/debugger/evals/run.sh"
) >"$TMP/timeout.stdout" 2>"$TMP/timeout.stderr"; then :; else timeout_status=$?; fi
timeout_elapsed="$(python3 - "$timeout_started" <<'PY2'
import sys, time
print(time.monotonic() - float(sys.argv[1]))
PY2
)"
[ "$timeout_status" -eq 2 ]
python3 - "$timeout_elapsed" <<'PY2'
import sys
raise SystemExit(not float(sys.argv[1]) < 5)
PY2
timeout_file="$(find "$TMP/timeout-state/runs" -type f -name 'custom-debugger-*.jsonl' -print)"
[ "$(jq -s '[.[] | select(.type == "case")] | length' "$timeout_file")" -gt 0 ]
jq -e 'select(.type == "case") | .generation_ms >= 0 and .requested_tier != null and .final_model == null and .attempts == []' "$timeout_file" >/dev/null

printf 'timing helper shell test passed\n'
