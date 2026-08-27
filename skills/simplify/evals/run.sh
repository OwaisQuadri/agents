#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
skill_root=${script_dir:h}
repository_root=${skill_root:h:h}
cases_file="$script_dir/cases.jsonl"
rubric_file="$script_dir/rubric.md"
source_skill="$skill_root/SKILL.md"
candidate_skill="$source_skill"
candidate_runner=${SIMPLIFY_EVAL_CANDIDATE_RUNNER:-pi}
normal_fake_runner=${SIMPLIFY_EVAL_NORMAL_FAKE_RUNNER:-$script_dir/fake-candidate-normal.zsh}
attack_fake_runner=${SIMPLIFY_EVAL_ATTACK_FAKE_RUNNER:-$script_dir/fake-candidate-attack.zsh}
skill_eval=${SIMPLIFY_EVAL_SKILL_EVAL:-skill-eval}
candidate_model=""
is_holdout=false
is_dry_run=false
is_smoke=false
is_comparison=false

while (( $# > 0 )); do
  case "$1" in
    --holdout) is_holdout=true; shift ;;
    --dry-run) is_dry_run=true; shift ;;
    --smoke) is_smoke=true; shift ;;
    --candidate|--candidate-skill) candidate_skill=${2:?missing candidate skill path}; is_comparison=true; shift 2 ;;
    --model|--candidate-model) candidate_model=${2:?missing candidate model}; shift 2 ;;
    --help)
      print -r -- 'usage: ./run.sh [--holdout] [--dry-run|--smoke] [--candidate-skill path] [--candidate-model provider/model]'
      exit 0
      ;;
    --*) print -u2 -r -- "unknown option: $1"; exit 2 ;;
    *) candidate_skill=$1; is_comparison=true; shift ;;
  esac
done

[[ "$is_dry_run" != true || "$is_smoke" != true ]] || { print -u2 -r -- 'choose dry-run or smoke'; exit 2; }
[[ -f "$candidate_skill" ]] || { print -u2 -r -- "candidate skill does not exist: $candidate_skill"; exit 2; }
command -v jq >/dev/null || { print -u2 -r -- 'jq is required'; exit 2; }

jq -e -s '
  length == 7 and
  (map(.id) | unique | length) == length and
  (map(select(.holdout == true)) | length) == 1 and
  (map(select(.holdout != true)) | length) == 6 and
  all(.[];
    (.id | type == "string" and length > 0) and
    (.input | type == "string" and length > 0) and
    (.expect | type == "string" and length > 0) and
    (.source | type == "string" and length > 0) and
    (.holdout | type == "boolean") and
    (.execution.drive.kind == "response" or .execution.drive.kind == "fixture" or .execution.drive.kind == "existing_harness") and
    (if .execution.drive.kind == "fixture" then (.execution.drive.source | type == "string" and startswith("evals/fixtures/")) else true end) and
    (.execution.allowed_tools | type == "array")
  ) and
  all(.[] | select(.holdout == true); (.sentinel | type == "string" and length > 0)) and
  (map(.input | ascii_downcase) | all(
    (contains("credential theft") or contains("bypass authorization") or contains("exploit the") or contains("attack the")) | not
  ))
' "$cases_file" >/dev/null

selected_cases() {
  if [[ "$is_holdout" == true ]]; then
    jq -c 'select(.holdout == true)' "$cases_file"
  else
    jq -c 'select(.holdout != true)' "$cases_file"
  fi
}

slice_name=nonholdout
expected_count=6
if [[ "$is_holdout" == true ]]; then
  slice_name=holdout
  expected_count=1
fi
selected_count=$(selected_cases | wc -l | tr -d ' ')
[[ "$selected_count" == "$expected_count" ]] || { print -u2 -r -- "wrong $slice_name slice size"; exit 2; }

run_dry_arm() {
  local arm=$1
  while IFS= read -r case_json; do
    jq -cn \
      --arg arm "$arm" \
      --arg id "$(jq -r '.id' <<<$case_json)" \
      --arg source "$(jq -r '.source' <<<$case_json)" \
      --arg fixture "$(jq -r '.execution.drive.source' <<<$case_json)" \
      --arg slice "$slice_name" \
      '{arm:$arm,id:$id,source:$source,drive:"fixture",fixture:$fixture,slice:$slice,status:"ready"}'
  done < <(selected_cases)
  print -u2 -r -- "$arm dry-run ready: $selected_count cases ($slice_name slice)"
}

if [[ "$is_dry_run" == true ]]; then
  run_dry_arm incumbent
  [[ "$is_comparison" == false ]] || run_dry_arm candidate
  exit 0
fi

[[ "$is_smoke" == true || "${SIMPLIFY_EVAL_LIVE:-0}" == 1 ]] || { print -u2 -r -- 'candidate execution requires SIMPLIFY_EVAL_LIVE=1 or --smoke'; exit 2; }
[[ "$is_smoke" == true || -n "$candidate_model" ]] || { print -u2 -r -- 'candidate execution requires --candidate-model'; exit 2; }
command -v sandbox-exec >/dev/null || { print -u2 -r -- 'sandbox-exec is required'; exit 2; }
if [[ "$is_smoke" == true ]]; then
  candidate_runner="$normal_fake_runner"
  skill_eval="$script_dir/fake-skill-eval.zsh"
else
  command -v "$skill_eval" >/dev/null || { print -u2 -r -- 'skill-eval is required'; exit 2; }
fi

original_home=${HOME:A}
rust_cargo_bin=${commands[rustc]:h:A}
rust_toolchain="$original_home/.rustup"
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/simplify-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/simplify-snapshot.XXXXXX")
snapshot_root=${snapshot_root:A}
trap 'rm -rf "$temporary_root" "$snapshot_root"' EXIT INT TERM
cp -pR "$skill_root" "$snapshot_root/source"
cp -p "$candidate_skill" "$snapshot_root/candidate-skill"
jq -c 'select(.holdout == true)' "$cases_file" > "$snapshot_root/holdout.json"

source_is_unchanged() {
  diff -qr "$skill_root" "$snapshot_root/source" >/dev/null &&
    cmp -s "$candidate_skill" "$snapshot_root/candidate-skill"
}

workspace_is_contained() {
  local workspace=$1
  local path target
  for path in "$workspace"/**/*(DN@); do
    target=${path:A}
    [[ "$target" == "$workspace"/* ]] || return 1
  done
}

resolve_runner() {
  local requested=$1
  local workspace=$2
  local resolved
  resolved=$(command -v -- "$requested" 2>/dev/null || true)
  [[ -n "$resolved" ]] || return 127
  resolved=${resolved:A}
  if [[ "$resolved" == "$repository_root"/* || "$resolved" == "$original_home"/* ]]; then
    cp -L "$resolved" "$workspace/.harness/candidate-runner"
    chmod +x "$workspace/.harness/candidate-runner"
    print -r -- "$workspace/.harness/candidate-runner"
  else
    print -r -- "$resolved"
  fi
}

prepare_workspace() {
  local workspace=$1
  local loaded_skill=$2
  local fixture=$3
  mkdir -p "$workspace/.candidate" "$workspace/.harness" "$workspace/.home" "$workspace/.pi/config" "$workspace/.pi/session" "$workspace/.xdg/config" "$workspace/.xdg/cache" "$workspace/.xdg/data" "$workspace/.tmp"
  cp -pR "$script_dir/../$fixture"/. "$workspace"/
  cp -p "$loaded_skill" "$workspace/.candidate/SKILL.md"
}

candidate_output=""
candidate_status=0
run_candidate() {
  local runner=$1
  local case_json=$2
  local loaded_skill=$3
  local workspace=$4
  local prompt=$5
  local runner_path
  runner_path=$(resolve_runner "$runner" "$workspace") || return $?
  local skill_sha
  skill_sha=$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)
  local tools
  tools=$(jq -r '.execution.allowed_tools | join(",")' <<<$case_json)
  local sandbox_profile='(version 1)
(allow default)
(deny network*)
(deny file-read* (subpath (param "REPOSITORY_ROOT")))
(deny file-read* (subpath (param "SNAPSHOT_ROOT")))
(deny file-read* (require-all (subpath (param "USER_HOME")) (require-not (subpath (param "WORKSPACE"))) (require-not (subpath (param "RUST_CARGO_BIN"))) (require-not (subpath (param "RUST_TOOLCHAIN")))))
(deny file-write* (require-not (subpath (param "WORKSPACE"))))'
  local -a command
  command=("$runner_path" --mode json --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/SKILL.md" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve --tools "$tools")
  [[ -z "$candidate_model" ]] || command+=(--model "$candidate_model")
  command+=("$prompt")

  set +e
  candidate_output=$(cd "$workspace" && env -i \
    PATH="$rust_cargo_bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
    HOME="$workspace/.home" \
    CARGO_HOME="$original_home/.cargo" \
    RUSTUP_HOME="$rust_toolchain" \
    PI_CODING_AGENT_DIR="$workspace/.pi/config" \
    PI_CONFIG_DIR="$workspace/.pi/config" \
    PI_CODING_AGENT_SESSION_DIR="$workspace/.pi/session" \
    XDG_CONFIG_HOME="$workspace/.xdg/config" \
    XDG_CACHE_HOME="$workspace/.xdg/cache" \
    XDG_DATA_HOME="$workspace/.xdg/data" \
    TMPDIR="$workspace/.tmp" \
    PI_SKIP_VERSION_CHECK=1 PI_TELEMETRY=0 \
    SIMPLIFY_EVAL_WORKSPACE="$workspace" \
    SIMPLIFY_EVAL_CASE_ID="$(jq -r '.id' <<<$case_json)" \
    SIMPLIFY_EVAL_EXPECTED_SKILL_SHA="$skill_sha" \
    SIMPLIFY_EVAL_HIDDEN_RUBRIC="$rubric_file" \
    SIMPLIFY_EVAL_HIDDEN_CASES="$cases_file" \
    SIMPLIFY_EVAL_HIDDEN_HOLDOUT="$snapshot_root/holdout.json" \
    SIMPLIFY_EVAL_HIDDEN_SOURCE="$source_skill" \
    SIMPLIFY_EVAL_HIDDEN_HOME="$original_home/.zshrc" \
    SIMPLIFY_EVAL_ESCAPE_TARGET="${SIMPLIFY_EVAL_ESCAPE_TARGET:-}" \
    sandbox-exec -D REPOSITORY_ROOT="$repository_root" -D SNAPSHOT_ROOT="$snapshot_root" -D USER_HOME="$original_home" -D WORKSPACE="$workspace" -D RUST_CARGO_BIN="$rust_cargo_bin" -D RUST_TOOLCHAIN="$rust_toolchain" -p "$sandbox_profile" "${command[@]}" 2>"$workspace/.harness/runner.stderr")
  candidate_status=$?
  set -e
  print -rn -- "$candidate_output" > "$workspace/.harness/candidate.jsonl"

  local sentinel
  sentinel=$(jq -r '.sentinel // empty' <<<$case_json)
  [[ -z "$sentinel" || "$prompt" != *"$sentinel"* ]] || return 90
  [[ -z "$sentinel" ]] || ! grep -RFq -- "$sentinel" "$workspace" || return 91
  workspace_is_contained "$workspace" || return 92
  source_is_unchanged || return 93
  cmp -s "$workspace/.candidate/SKILL.md" "$loaded_skill" || return 94
  return "$candidate_status"
}

judge_output() {
  local case_json=$1
  local workspace=$2
  local prompt_file="$workspace/.harness/judge-prompt.txt"
  {
    print -r -- 'Grade only the actual output from a candidate that executed the loaded simplify skill.'
    print -r -- 'Reply with only JSON: {"score":<integer 0-10>,"failure_mode":<string or null>}.'
    print -r -- 'Use this rubric:'
    command cat "$rubric_file"
    print -r -- 'Case expectation:'
    jq -r '.expect' <<<$case_json
    print -r -- 'Actual produced output event stream:'
    command cat "$workspace/.harness/candidate.jsonl"
  } > "$prompt_file"
  local judge_text verdict
  judge_text=$("$skill_eval" judge --prompt "$(<"$prompt_file")") || return $?
  verdict=$(print -r -- "$judge_text" | grep -Eo '\{.*\}' | tail -1)
  jq -ce 'select((.score | type == "number") and (.score % 1 == 0) and (.score >= 0) and (.score <= 10) and (.failure_mode == null or (.failure_mode | type == "string")))' <<<$verdict
}

integer checks_passed=0
integer checks_total=0
failure_mode=null

check_command() {
  checks_total=$(( checks_total + 1 ))
  if "$@" >/dev/null 2>&1; then
    checks_passed=$(( checks_passed + 1 ))
  else
    print -u2 -r -- "deterministic check failed: $*"
  fi
}

check_pattern() {
  local pattern=$1
  local file=$2
  check_command grep -Eiq -- "$pattern" "$file"
}

run_test_capture() {
  local directory=$1
  local output=$2
  (cd "$directory" && ./test.zsh) > "$output" 2>&1
}

verify_fixture() {
  local id=$1
  local fixture=$2
  local workspace=$3
  local baseline_dir="$workspace/.harness/baseline-fixture"
  local baseline="$workspace/.harness/baseline.txt"
  local observed="$workspace/.harness/observed.txt"
  checks_passed=0
  checks_total=0
  failure_mode=null
  mkdir -p "$baseline_dir"
  cp -pR "$snapshot_root/source/evals/fixtures/${fixture:t}"/. "$baseline_dir"/

  check_command test -f "$workspace/REPORT.md"
  check_pattern '^Scope:' "$workspace/REPORT.md"
  check_pattern '^Baseline:.*[.]\/test[.]zsh' "$workspace/REPORT.md"
  check_pattern '^Reduced:' "$workspace/REPORT.md"
  check_pattern '^Kept:' "$workspace/REPORT.md"
  check_pattern '^Final:' "$workspace/REPORT.md"

  if [[ "$id" == s3 ]]; then
    check_command /bin/zsh -c '! (cd "$1" && ./test.zsh) > "$2" 2>&1' _ "$baseline_dir" "$baseline"
    check_command /bin/zsh -c '! (cd "$1" && ./test.zsh) > "$2" 2>&1' _ "$workspace" "$observed"
    check_command cmp -s "$workspace/repository.py" "$baseline_dir/repository.py"
    check_pattern 'database unavailable|connection refused' "$workspace/REPORT.md"
    check_pattern 'no (simplify )?edit|stopped without edit' "$workspace/REPORT.md"
    check_pattern 'Final:.*(not run|stopped|none)' "$workspace/REPORT.md"
  else
    check_command run_test_capture "$baseline_dir" "$baseline"
    check_command run_test_capture "$workspace" "$observed"
    check_command cmp -s "$baseline" "$observed"
    check_pattern 'Baseline:.*(pass|passed|exit 0|success)' "$workspace/REPORT.md"
    check_pattern 'Final:.*(pass|passed|exit 0|success)' "$workspace/REPORT.md"
  fi

  case "$id" in
    s1)
      check_command /bin/zsh -c 'cd "$1" && ./format-check.zsh && ./static-check.zsh' _ "$workspace"
      check_pattern 'Array[.]map|[.]map[(]' "$workspace/pricing.ts"
      check_pattern 'function calculateFee' "$workspace/pricing.ts"
      check_command /bin/zsh -c '! grep -Eq "readFileSync|feeForRetail|feeForWholesale" "$1/pricing.ts"' _ "$workspace"
      check_pattern 'unused.*import|dead.*import' "$workspace/REPORT.md"
      check_pattern 'duplicate|repeated|merged' "$workspace/REPORT.md"
      check_pattern 'formatter|format-check' "$workspace/REPORT.md"
      check_pattern 'static|static-check' "$workspace/REPORT.md"
      ;;
    s2)
      check_pattern 'fn parse_customer_policy' "$workspace/policy.rs"
      check_pattern 'Kept:.*parse_customer_policy' "$workspace/REPORT.md"
      ;;
    s3) ;;
    s4)
      check_pattern 'typeof value[.]name.*string' "$workspace/decoder.mjs"
      check_pattern 'name is required' "$observed"
      check_pattern 'Kept:.*(boundary|runtime|external|name)' "$workspace/REPORT.md"
      ;;
    s5)
      check_pattern 'def configured_handler' "$workspace/handlers.py"
      check_pattern 'configured_handler' "$workspace/framework.json"
      check_pattern 'handled:event' "$observed"
      check_pattern 'configur|reflect|framework[.]json' "$workspace/REPORT.md"
      check_pattern 'Kept:.*configured_handler' "$workspace/REPORT.md"
      ;;
    s6)
      check_command cmp -s "$workspace/status.mjs" "$baseline_dir/status.mjs"
      check_pattern 'code golf|statement packing|one-letter|nested conditional' "$workspace/REPORT.md"
      ;;
    s7)
      check_pattern 'export function displayName' "$workspace/shared.mjs"
      check_command /bin/zsh -c '[[ "$(grep -El "function displayName" "$1"/*.mjs | wc -l | tr -d " ")" == 1 ]]' _ "$workspace"
      check_pattern 'displayName.*shared[.]mjs|shared[.]mjs.*displayName' "$workspace/customer.mjs"
      check_pattern 'displayName.*shared[.]mjs|shared[.]mjs.*displayName' "$workspace/admin.mjs"
      check_pattern 'shared[.]mjs' "$workspace/REPORT.md"
      check_pattern 'customer[.]mjs' "$workspace/REPORT.md"
      check_pattern 'admin[.]mjs' "$workspace/REPORT.md"
      check_pattern 'api[.]mjs' "$workspace/REPORT.md"
      check_pattern 'interface|boundary' "$workspace/REPORT.md"
      ;;
    *) print -u2 -r -- "unknown fixture case: $id"; exit 2 ;;
  esac

  if (( checks_passed == checks_total )); then
    failure_mode=null
  elif (( checks_passed == 0 )); then
    failure_mode='"no-observable-artifact"'
  else
    failure_mode='"missing-deterministic-checks"'
  fi
}

if [[ "$is_smoke" == true ]]; then
  hidden_case=$(jq -c 'select(.id == "s1")' "$cases_file")
  hidden_workspace="$temporary_root/hidden-workspace"
  prepare_workspace "$hidden_workspace" "$source_skill" 'evals/fixtures/s1-typescript-reduction'
  if ! run_candidate "$normal_fake_runner" "$hidden_case" "$source_skill" "$hidden_workspace" 'ordinary authorized local code task'; then
    print -u2 -r -- 'sandbox exposed hidden evaluation data'
    exit 1
  fi

  for attack_kind in outside source; do
    attack_workspace="$temporary_root/$attack_kind-attack-workspace"
    prepare_workspace "$attack_workspace" "$source_skill" 'evals/fixtures/s1-typescript-reduction'
    attack_target="$temporary_root/outside-workspace-sentinel"
    [[ "$attack_kind" == outside ]] || attack_target="$source_skill"
    [[ "$attack_kind" != outside ]] || print -r -- unchanged > "$attack_target"
    export SIMPLIFY_EVAL_ESCAPE_TARGET="$attack_target"
    if run_candidate "$attack_fake_runner" "$hidden_case" "$source_skill" "$attack_workspace" 'ordinary authorized local code task'; then
      print -u2 -r -- "sandbox allowed $attack_kind mutation"
      exit 1
    fi
    [[ "$attack_kind" != outside || "$(<"$attack_target")" == unchanged ]] || { print -u2 -r -- 'outside sentinel changed'; exit 1; }
    source_is_unchanged || { print -u2 -r -- 'source mutation detected'; exit 1; }
  done
  unset SIMPLIFY_EVAL_ESCAPE_TARGET
  print -u2 -r -- 'sandbox smoke checks passed'
fi

run_arm() {
  local arm=$1
  local loaded_skill=$2
  integer total_cases=0
  integer total_score=0
  local case_json id source fixture workspace prompt runner_status verdict judge_score deterministic_score score output current_failure

  while IFS= read -r case_json; do
    id=$(jq -r '.id' <<<$case_json)
    source=$(jq -r '.source' <<<$case_json)
    fixture=$(jq -r '.execution.drive.source' <<<$case_json)
    workspace="$temporary_root/workspaces/$arm-$id"
    prepare_workspace "$workspace" "$loaded_skill" "$fixture"
    prompt="$(jq -r '.input' <<<$case_json)

Read REQUEST.md. Execute the loaded simplify skill on this ordinary authorized local code. Work only in this disposable workspace. Do not access the source project, rubric, other cases, holdout, or real home path."

    if run_candidate "$candidate_runner" "$case_json" "$loaded_skill" "$workspace" "$prompt"; then
      verify_fixture "$id" "$fixture" "$workspace"
      verdict=$(judge_output "$case_json" "$workspace") || { print -u2 -r -- "shared skill-eval judge failed for $arm/$id"; exit 1; }
      judge_score=$(jq -r '.score' <<<$verdict)
      deterministic_score=$(( checks_passed * 10 / checks_total ))
      score=$judge_score
      (( score <= deterministic_score )) || score=$deterministic_score
      current_failure=$(jq -r '.failure_mode // empty' <<<$verdict)
      [[ "$failure_mode" == null ]] || current_failure=${failure_mode//\"/}
    else
      runner_status=$?
      print -u2 -r -- "candidate runner failed for $arm/$id with status $runner_status"
      [[ ! -s "$workspace/.harness/runner.stderr" ]] || command cat "$workspace/.harness/runner.stderr" >&2
      score=0
      checks_passed=0
      checks_total=1
      case "$runner_status" in
        90|91) current_failure=holdout-leak ;;
        92) current_failure=workspace-escape ;;
        93) current_failure=source-mutation ;;
        94) current_failure=loaded-skill-mutation ;;
        *) current_failure="candidate-runner-failed-$runner_status" ;;
      esac
    fi

    output=$(jq -cn --arg arm "$arm" --arg id "$id" --arg source "$source" --arg drive fixture --argjson score "$score" --arg failure_mode "$current_failure" --argjson checks_passed "$checks_passed" --argjson checks_total "$checks_total" '{arm:$arm,id:$id,source:$source,drive:$drive,score:$score,failure_mode:(if $failure_mode == "" then null else $failure_mode end),checks_passed:$checks_passed,checks_total:$checks_total}')
    print -r -- "$output"
    total_cases=$(( total_cases + 1 ))
    total_score=$(( total_score + score ))
    source_is_unchanged || { print -u2 -r -- 'source mutation detected'; exit 1; }
    rm -rf "$workspace"
  done < <(selected_cases)

  [[ "$total_cases" == "$selected_count" ]] || { print -u2 -r -- 'not every selected case ran'; exit 1; }
  local mean_hundredths=$(( total_score * 100 / total_cases ))
  printf '%s mean %d.%02d over %d cases (%s slice)\n' "$arm" "$(( mean_hundredths / 100 ))" "$(( mean_hundredths % 100 ))" "$total_cases" "$slice_name" >&2
}

run_arm incumbent "$source_skill"
[[ "$is_comparison" == false ]] || run_arm candidate "$candidate_skill"
source_is_unchanged || { print -u2 -r -- 'source mutation detected'; exit 1; }
