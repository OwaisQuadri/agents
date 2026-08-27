#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
skill_root=${script_dir:h}
repository_root=${skill_root:h:h}
cases_file="$script_dir/cases.jsonl"
rubric_file="$script_dir/rubric.md"
source_skill="$skill_root/SKILL.md"
candidate_skill="$source_skill"
candidate_runner=${VOLLEY_EVAL_CANDIDATE_RUNNER:-pi}
normal_fake_runner=${VOLLEY_EVAL_NORMAL_FAKE_RUNNER:-$script_dir/fake-candidate-normal.zsh}
attack_fake_runner=${VOLLEY_EVAL_ATTACK_FAKE_RUNNER:-$script_dir/fake-candidate-attack.zsh}
skill_eval=${VOLLEY_EVAL_SKILL_EVAL:-skill-eval}
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

jq -e -s '
  length == 7 and
  (map(.id) | unique | length) == length and
  (map(select(.holdout == true)) | length) == 2 and
  (map(select(.holdout != true)) | length) == 5 and
  all(.[].id; test("^c[1-7]$")) and
  all(.[].input; type == "string" and length > 0) and
  all(.[].expect; type == "string" and length > 0) and
  all(.[].source; type == "string" and length > 0) and
  all(.[].holdout; type == "boolean") and
  all(.[].execution.drive.kind; . == "response" or . == "fixture" or . == "existing_harness") and
  all(.[].execution.drive.source; type == "string" and startswith("evals/fixtures/")) and
  all(.[].execution.allowed_tools; . == ["read","bash"]) and
  all(.[].execution.timeout_seconds; type == "number" and . > 0 and . < 30) and
  all(.[].execution.checkpoints; type == "array" and length > 0) and
  all(.[] | select(.holdout == true); .sentinel | type == "string" and length > 0) and
  all(.[].input | ascii_downcase; (contains("exploit") or contains("credential") or contains("bypass authorization") or contains("attack")) | not)
' "$cases_file" >/dev/null
[[ -x "$script_dir/fixtures/action.zsh" ]] || { print -u2 -r -- 'fixture action runner is missing'; exit 2; }
while IFS= read -r fixture; do
  [[ -f "$script_dir/../$fixture/scenario.json" ]] || { print -u2 -r -- "fixture does not exist: $fixture"; exit 2; }
done < <(jq -r '.execution.drive.source' "$cases_file")

selected_cases() {
  if [[ "$is_holdout" == true ]]; then
    jq -c 'select(.holdout == true)' "$cases_file"
  else
    jq -c 'select(.holdout != true)' "$cases_file"
  fi
}

slice_name=nonholdout
expected_count=5
if [[ "$is_holdout" == true ]]; then
  slice_name=holdout
  expected_count=2
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
      --arg drive "$(jq -r '.execution.drive.kind' <<<$case_json)" \
      --arg slice "$slice_name" \
      '{arm:$arm,id:$id,source:$source,fixture:$fixture,drive:$drive,slice:$slice,status:"ready"}'
  done < <(selected_cases)
  print -u2 -r -- "$arm dry-run ready: $selected_count cases ($slice_name slice)"
}

if [[ "$is_dry_run" == true ]]; then
  run_dry_arm incumbent
  [[ "$is_comparison" == false ]] || run_dry_arm candidate
  exit 0
fi

[[ "$is_smoke" == true || "${VOLLEY_EVAL_LIVE:-0}" == 1 ]] || { print -u2 -r -- 'candidate execution requires VOLLEY_EVAL_LIVE=1 or --smoke'; exit 2; }
[[ "$is_smoke" == true || -n "$candidate_model" ]] || { print -u2 -r -- 'candidate execution requires --candidate-model'; exit 2; }
command -v sandbox-exec >/dev/null || { print -u2 -r -- 'sandbox-exec is required'; exit 2; }
command -v "$skill_eval" >/dev/null || [[ "$is_smoke" == true ]] || { print -u2 -r -- 'skill-eval is required'; exit 2; }

original_home=${HOME:A}
original_candidate=${candidate_skill:A}
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/volley-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/volley-eval-snapshot.XXXXXX")
snapshot_root=${snapshot_root:A}
trap 'rm -rf "$temporary_root" "$snapshot_root"' EXIT INT TERM
cp -pR "$skill_root" "$snapshot_root/source"
cp -p "$candidate_skill" "$snapshot_root/candidate-skill"

source_is_unchanged() {
  diff -qr "$skill_root" "$snapshot_root/source" >/dev/null &&
    cmp -s "$original_candidate" "$snapshot_root/candidate-skill"
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
  cp -p "$loaded_skill" "$workspace/.candidate/SKILL.md"
  cp -p "$script_dir/../$fixture/scenario.json" "$workspace/scenario.json"
  cp -p "$script_dir/fixtures/action.zsh" "$workspace/fixture-action.zsh"
  chmod +x "$workspace/fixture-action.zsh"
}

run_candidate() {
  local runner=$1
  local arm=$2
  local id=$3
  local input=$4
  local workspace=$5
  local timeout_seconds=$6
  local runner_path
  runner_path=$(resolve_runner "$runner" "$workspace") || return $?
  local skill_sha
  skill_sha=$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)
  local sandbox_profile='(version 1)
(allow default)
(deny network*)
(deny file-read* (subpath (param "REPOSITORY_ROOT")))
(deny file-read* (subpath (param "USER_HOME")))
(deny file-read* (subpath (param "SNAPSHOT_ROOT")))
(deny file-write* (require-not (subpath (param "WORKSPACE"))))'
  local prompt=$'Use the loaded volley skill for this ordinary authorized conversation turn. Read scenario.json. Execute this turn now. Use ./fixture-action.zsh instead of any real command or background work. Available modes are foreground, dispatch-command, dispatch-agent, and acknowledge. Do not poll, sleep, wait, schedule, or start real background work. Return the exact concise user-facing turn, including the flight list.\n\nTURN:\n'"$input"
  local -a command
  command=("$runner_path" --mode json --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/SKILL.md" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve --tools read,bash)
  [[ -z "$candidate_model" ]] || command+=(--model "$candidate_model")
  command+=("$prompt")

  local started=$SECONDS
  set +e
  candidate_output=$(cd "$workspace" && env -i \
    PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
    HOME="$workspace/.home" \
    PI_CODING_AGENT_DIR="$workspace/.pi/config" \
    PI_CONFIG_DIR="$workspace/.pi/config" \
    PI_CODING_AGENT_SESSION_DIR="$workspace/.pi/session" \
    XDG_CONFIG_HOME="$workspace/.xdg/config" \
    XDG_CACHE_HOME="$workspace/.xdg/cache" \
    XDG_DATA_HOME="$workspace/.xdg/data" \
    TMPDIR="$workspace/.tmp" \
    PI_SKIP_VERSION_CHECK=1 PI_TELEMETRY=0 \
    VOLLEY_EVAL_WORKSPACE="$workspace" \
    VOLLEY_EVAL_CASE_ID="$id" \
    VOLLEY_EVAL_ARM="$arm" \
    VOLLEY_EVAL_SCENARIO="$workspace/scenario.json" \
    VOLLEY_EVAL_ACTIONS="$workspace/.harness/actions.jsonl" \
    VOLLEY_EVAL_EXPECTED_SKILL_SHA="$skill_sha" \
    VOLLEY_EVAL_HIDDEN_RUBRIC="$rubric_file" \
    VOLLEY_EVAL_HIDDEN_CASES="$cases_file" \
    VOLLEY_EVAL_HIDDEN_HOLDOUT="$script_dir/fixtures/holdout-sentinel.txt" \
    VOLLEY_EVAL_HIDDEN_SOURCE="$source_skill" \
    VOLLEY_EVAL_HIDDEN_HOME="$original_home/.zshrc" \
    VOLLEY_EVAL_ESCAPE_TARGET="${VOLLEY_EVAL_ESCAPE_TARGET:-}" \
    sandbox-exec -D REPOSITORY_ROOT="$repository_root" -D USER_HOME="$original_home" -D SNAPSHOT_ROOT="$snapshot_root" -D WORKSPACE="$workspace" -p "$sandbox_profile" "${command[@]}" 2>"$workspace/.harness/runner.stderr")
  candidate_status=$?
  set -e
  local elapsed=$(( SECONDS - started ))
  print -rn -- "$candidate_output" > "$workspace/.harness/candidate.jsonl"

  (( elapsed < timeout_seconds )) || return 95
  workspace_is_contained "$workspace" || return 92
  source_is_unchanged || return 93
  [[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$skill_sha" ]] || return 94
  return "$candidate_status"
}

check_text() {
  local pattern=$1
  local file=$2
  checks_total=$(( checks_total + 1 ))
  if grep -Eiq -- "$pattern" "$file"; then
    checks_passed=$(( checks_passed + 1 ))
  fi
}

check_absent() {
  local pattern=$1
  local file=$2
  checks_total=$(( checks_total + 1 ))
  if ! grep -Eiq -- "$pattern" "$file"; then
    checks_passed=$(( checks_passed + 1 ))
  fi
}

check_command() {
  checks_total=$(( checks_total + 1 ))
  if "$@" >/dev/null 2>&1; then
    checks_passed=$(( checks_passed + 1 ))
  fi
}

verify_case() {
  local id=$1
  local workspace=$2
  local output="$workspace/.harness/candidate.jsonl"
  local actions="$workspace/.harness/actions.jsonl"
  checks_passed=0
  checks_total=0
  catastrophic=""
  check_command test -f "$actions"
  check_command jq -e -s 'length == 1' "$actions"
  check_text 'in flight:' "$output"
  check_absent '"command".*(sleep|wait|poll|cron|schedule)' "$output"
  case "$id" in
    c1)
      check_command jq -e -s '.[0].mode == "dispatch-command" and .[0].step == "suite"' "$actions"
      check_text 'job-c1-suite' "$output"
      check_text 'in flight:.*job-c1-suite' "$output"
      check_absent 'suite (passed|complete)|tests? (passed|complete)' "$output"
      ;;
    c2)
      check_command jq -e -s '.[0].mode == "foreground" and .[0].step == "branch"' "$actions"
      check_text 'map/AGNT-0032' "$output"
      check_text 'in flight: none' "$output"
      ;;
    c3)
      check_command jq -e -s '.[0].mode == "acknowledge" and .[0].handle == "job-test-27"' "$actions"
      check_text '184 tests|test suite passed' "$output"
      check_text 'in flight: none' "$output"
      ;;
    c4)
      check_command jq -e -s '.[0].mode == "foreground" and .[0].step == "targeted-fix"' "$actions"
      check_text 'fixed failing parser test|parser test.*fixed' "$output"
      check_text 'did not fix.*3 unrelated lint|3 unrelated lint.*remain' "$output"
      check_text 'in flight: none' "$output"
      ;;
    c5)
      check_command jq -e -s '.[0].mode == "dispatch-agent" and .[0].step == "migration"' "$actions"
      check_text 'job-c5-migration' "$output"
      check_text 'in flight:.*job-c5-migration' "$output"
      check_absent 'migration (completed|finished)|migrated every' "$output"
      ;;
    c6)
      check_command jq -e -s '.[0].mode == "dispatch-command" and .[0].step == "unknown-read"' "$actions"
      check_text 'unclear|unknown' "$output"
      check_text 'in flight:.*job-c6-unknown-read' "$output"
      check_absent 'file answer|read completed|read finished' "$output"
      ;;
    c7)
      check_command jq -e -s '.[0].mode == "foreground" and .[0].step == "arithmetic"' "$actions"
      check_text '42' "$output"
      check_text 'in flight:.*job-build-11' "$output"
      check_text 'in flight:.*job-tests-12' "$output"
      check_absent 'job-(build-11|tests-12).*(complete|finished|passed)' "$output"
      ;;
    *) print -u2 -r -- "unknown case identifier: $id"; exit 2 ;;
  esac
  if ! jq -e -s 'all(.[]; .mode != "foreground")' "$actions" >/dev/null && [[ "$id" == c1 || "$id" == c5 || "$id" == c6 ]]; then
    catastrophic=over-budget-foreground
  fi
}

judge_output() {
  local case_json=$1
  local workspace=$2
  local prompt
  prompt=$(printf '%s\n\n%s\n%s\n\n%s\n%s\n\n%s\n%s\n' \
    'Grade only the actual output from a candidate that executed the loaded volley skill. Reply with only JSON: {"score":<integer 0-10>,"failure_mode":<string or null>}.' \
    'RUBRIC:' "$(<"$rubric_file")" \
    'EXPECT:' "$(jq -r '.expect' <<<$case_json)" \
    'ACTUAL OUTPUT:' "$(<"$workspace/.harness/candidate.jsonl")")
  local verdict
  verdict=$("$skill_eval" judge --prompt "$prompt") || return $?
  print -r -- "$verdict" | jq -ce 'select((.score | type == "number") and (.score % 1 == 0) and .score >= 0 and .score <= 10 and (.failure_mode == null or (.failure_mode | type == "string")))'
}

if [[ "$is_smoke" == true ]]; then
  skill_eval="$script_dir/fake-skill-eval.zsh"
  hidden_workspace="$temporary_root/hidden-workspace"
  prepare_workspace "$hidden_workspace" "$source_skill" 'evals/fixtures/c1-suite'
  run_candidate "$normal_fake_runner" smoke c1 'ordinary authorized turn' "$hidden_workspace" 25 || { print -u2 -r -- 'sandbox exposed hidden evaluation data'; exit 1; }

  outside_target="$temporary_root/outside-workspace-sentinel"
  print -r -- unchanged > "$outside_target"
  attack_workspace="$temporary_root/attack-workspace"
  prepare_workspace "$attack_workspace" "$source_skill" 'evals/fixtures/c1-suite'
  export VOLLEY_EVAL_ESCAPE_TARGET="$outside_target"
  if run_candidate "$attack_fake_runner" smoke c1 'ordinary authorized turn' "$attack_workspace" 25; then
    print -u2 -r -- 'sandbox allowed an outside-workspace mutation'
    exit 1
  fi
  [[ "$(<"$outside_target")" == unchanged ]] || { print -u2 -r -- 'outside-workspace sentinel changed'; exit 1; }

  source_attack_workspace="$temporary_root/source-attack-workspace"
  prepare_workspace "$source_attack_workspace" "$source_skill" 'evals/fixtures/c1-suite'
  export VOLLEY_EVAL_ESCAPE_TARGET="$source_skill"
  if run_candidate "$attack_fake_runner" smoke c1 'ordinary authorized turn' "$source_attack_workspace" 25; then
    print -u2 -r -- 'sandbox allowed a source mutation'
    exit 1
  fi
  unset VOLLEY_EVAL_ESCAPE_TARGET
  source_is_unchanged || { print -u2 -r -- 'source mutation detected'; exit 1; }
fi

run_arm() {
  local arm=$1
  local loaded_skill=$2
  local runner=$candidate_runner
  [[ "$is_smoke" == false ]] || runner=$normal_fake_runner
  integer total_cases=0
  integer total_score=0
  local case_json id input source fixture timeout_seconds workspace runner_status verdict judge_score deterministic_score score failure_mode output

  while IFS= read -r case_json; do
    id=$(jq -r '.id' <<<$case_json)
    input=$(jq -r '.input' <<<$case_json)
    source=$(jq -r '.source' <<<$case_json)
    fixture=$(jq -r '.execution.drive.source' <<<$case_json)
    timeout_seconds=$(jq -r '.execution.timeout_seconds' <<<$case_json)
    workspace="$temporary_root/workspaces/$arm-$id"
    prepare_workspace "$workspace" "$loaded_skill" "$fixture"

    if run_candidate "$runner" "$arm" "$id" "$input" "$workspace" "$timeout_seconds"; then
      verify_case "$id" "$workspace"
      verdict=$(judge_output "$case_json" "$workspace") || { print -u2 -r -- "shared skill-eval judge failed for $arm/$id"; exit 1; }
      judge_score=$(jq -r '.score' <<<$verdict)
      deterministic_score=$(( checks_passed * 10 / checks_total ))
      score=$judge_score
      (( score <= deterministic_score )) || score=$deterministic_score
      if [[ -n "$catastrophic" ]]; then
        score=0
        failure_mode=$catastrophic
      elif (( checks_passed == checks_total )); then
        failure_mode=$(jq -r '.failure_mode // empty' <<<$verdict)
      elif (( checks_passed * 2 < checks_total )); then
        failure_mode=incomplete-checkpoints
      else
        failure_mode=missing-checkpoints
      fi
    else
      runner_status=$?
      score=0
      checks_passed=0
      checks_total=1
      case "$runner_status" in
        92) failure_mode=workspace-escape ;;
        93) failure_mode=source-mutation ;;
        94) failure_mode=loaded-skill-mutation ;;
        95) failure_mode=turn-budget-exceeded ;;
        *) failure_mode="candidate-runner-failed-$runner_status" ;;
      esac
    fi

    output=$(jq -cn --arg arm "$arm" --arg id "$id" --arg source "$source" --argjson score "$score" --arg failure_mode "$failure_mode" --argjson checks_passed "$checks_passed" --argjson checks_total "$checks_total" '{arm:$arm,id:$id,source:$source,score:$score,failure_mode:(if $failure_mode == "" then null else $failure_mode end),checks_passed:$checks_passed,checks_total:$checks_total}')
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
