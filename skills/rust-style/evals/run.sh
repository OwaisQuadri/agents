#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
repository_root=${script_dir:h:h:h}
cases_file="$script_dir/cases.jsonl"
rubric_file="$script_dir/rubric.md"
source_skill="$script_dir/../SKILL.md"
candidate_skill="$source_skill"
candidate_runner=${RUST_STYLE_EVAL_CANDIDATE_RUNNER:-pi}
normal_fake_runner=${RUST_STYLE_EVAL_NORMAL_FAKE_RUNNER:-$script_dir/fake-candidate-normal.zsh}
attack_fake_runner=${RUST_STYLE_EVAL_ATTACK_FAKE_RUNNER:-$script_dir/fake-candidate-attack.zsh}
skill_eval=${RUST_STYLE_EVAL_SKILL_EVAL:-skill-eval}
candidate_model=""
is_holdout=false
is_dry_run=false
is_smoke=false
is_comparison=false

while (( $# > 0 )); do
  case "$1" in
    --holdout)
      is_holdout=true
      shift
      ;;
    --dry-run)
      is_dry_run=true
      shift
      ;;
    --smoke)
      is_smoke=true
      shift
      ;;
    --candidate|--candidate-skill)
      candidate_skill=${2:?missing candidate skill path}
      is_comparison=true
      shift 2
      ;;
    --model|--candidate-model)
      candidate_model=${2:?missing candidate model}
      shift 2
      ;;
    --help)
      print -r -- "usage: ./run.sh [--holdout] [--dry-run|--smoke] [--candidate-skill path] [--candidate-model provider/model]"
      exit 0
      ;;
    --*)
      print -u2 -r -- "unknown option: $1"
      exit 2
      ;;
    *)
      candidate_skill=$1
      is_comparison=true
      shift
      ;;
  esac
done

[[ "$is_dry_run" != true || "$is_smoke" != true ]] || { print -u2 -r -- "choose dry-run or smoke"; exit 2; }
[[ -f "$candidate_skill" ]] || { print -u2 -r -- "candidate skill does not exist: $candidate_skill"; exit 2; }

jq -e -s '
  length == 6 and
  (map(.id) | unique | length) == length and
  (map(select(.holdout == true)) | length) == 1 and
  (map(select(.holdout != true)) | length) == 5 and
  all(
    (.id | type == "string" and length > 0) and
    (.input | type == "string" and length > 0) and
    (.expect | type == "string" and length > 0) and
    (.source | type == "string" and length > 0) and
    (.holdout | type == "boolean") and
    (.execution.drive.kind == "response" or .execution.drive.kind == "fixture" or .execution.drive.kind == "existing_harness") and
    (.execution.allowed_tools | type == "array") and
    (if .execution.drive.kind == "fixture" then (.execution.drive.source | type == "string" and length > 0) else true end)
  ) and
  (map(.input | ascii_downcase) | all(
    (contains("exploit") or contains("bypass authorization") or contains("credential") or contains("live-system attack")) | not
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
expected_count=5
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
      --arg drive "$(jq -r '.execution.drive.kind' <<<$case_json)" \
      --arg slice "$slice_name" \
      '{arm:$arm,id:$id,source:$source,drive:$drive,slice:$slice,status:"ready"}'
  done < <(selected_cases)
  print -u2 -r -- "$arm dry-run ready: $selected_count cases ($slice_name slice)"
}

if [[ "$is_dry_run" == true ]]; then
  run_dry_arm incumbent
  [[ "$is_comparison" == false ]] || run_dry_arm candidate
  exit 0
fi

[[ "$is_smoke" == true || "${RUST_STYLE_EVAL_LIVE:-0}" == 1 ]] || { print -u2 -r -- "candidate execution requires RUST_STYLE_EVAL_LIVE=1 or --smoke"; exit 2; }
[[ "$is_smoke" == true || -n "$candidate_model" ]] || { print -u2 -r -- "candidate execution requires --candidate-model"; exit 2; }
command -v sandbox-exec >/dev/null || { print -u2 -r -- "sandbox-exec is required"; exit 2; }
command -v jq >/dev/null || { print -u2 -r -- "jq is required"; exit 2; }
command -v cargo >/dev/null || { print -u2 -r -- "cargo is required"; exit 2; }
if [[ "$is_smoke" == true ]]; then
  candidate_runner="$normal_fake_runner"
  skill_eval="$script_dir/fake-skill-eval.zsh"
else
  command -v "$skill_eval" >/dev/null || { print -u2 -r -- "skill-eval is required"; exit 2; }
fi

original_home=${HOME:A}
rust_cargo_bin=${commands[cargo]:h:A}
rust_toolchain="$original_home/.rustup"
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/rust-style-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/rust-style-snapshot.XXXXXX")
snapshot_root=${snapshot_root:A}
trap 'rm -rf "$temporary_root" "$snapshot_root"' EXIT INT TERM
cp -pR "$script_dir/.." "$snapshot_root/source"
cp -p "$candidate_skill" "$snapshot_root/candidate-skill"
jq -c 'select(.holdout == true)' "$cases_file" > "$snapshot_root/holdout.json"

source_is_unchanged() {
  diff -qr "$script_dir/.." "$snapshot_root/source" >/dev/null &&
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
  mkdir -p "$workspace/.candidate" "$workspace/.harness" "$workspace/.home" "$workspace/.pi/config" "$workspace/.pi/session" "$workspace/.xdg/config" "$workspace/.xdg/cache" "$workspace/.xdg/data" "$workspace/.tmp" "$workspace/.cargo"
  cp -p "$loaded_skill" "$workspace/.candidate/SKILL.md"
}

prepare_references() {
  local case_json=$1
  local workspace=$2
  local extra
  while IFS= read -r extra; do
    [[ -f "$script_dir/../$extra" ]] || { print -u2 -r -- "reference file does not exist: $extra"; exit 2; }
    cp -p "$script_dir/../$extra" "$workspace/.candidate/$extra"
  done < <(jq -r '.files[]?' <<<$case_json)
}

loaded_references_are_unchanged() {
  local case_json=$1
  local workspace=$2
  local extra
  while IFS= read -r extra; do
    cmp -s "$workspace/.candidate/$extra" "$script_dir/../$extra" || return 1
  done < <(jq -r '.files[]?' <<<$case_json)
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
  command=("$runner_path" -p --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/SKILL.md" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve)
  if [[ -n "$tools" ]]; then
    command+=(--tools "$tools")
  else
    command+=(--no-tools)
  fi
  [[ -z "$candidate_model" ]] || command+=(--model "$candidate_model")
  command+=("$prompt")

  set +e
  candidate_output=$(cd "$workspace" && env -i \
    PATH="$rust_cargo_bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
    HOME="$workspace/.home" \
    CARGO_HOME="$workspace/.cargo" \
    RUSTUP_HOME="$rust_toolchain" \
    PI_CODING_AGENT_DIR="$workspace/.pi/config" \
    PI_CONFIG_DIR="$workspace/.pi/config" \
    PI_CODING_AGENT_SESSION_DIR="$workspace/.pi/session" \
    XDG_CONFIG_HOME="$workspace/.xdg/config" \
    XDG_CACHE_HOME="$workspace/.xdg/cache" \
    XDG_DATA_HOME="$workspace/.xdg/data" \
    TMPDIR="$workspace/.tmp" \
    PI_SKIP_VERSION_CHECK=1 \
    PI_TELEMETRY=0 \
    RUST_STYLE_EVAL_WORKSPACE="$workspace" \
    RUST_STYLE_EVAL_CASE_ID="$(jq -r '.id' <<<$case_json)" \
    RUST_STYLE_EVAL_EXPECTED_SKILL_SHA="$skill_sha" \
    RUST_STYLE_EVAL_HIDDEN_RUBRIC="$rubric_file" \
    RUST_STYLE_EVAL_HIDDEN_CASES="$cases_file" \
    RUST_STYLE_EVAL_HIDDEN_HOLDOUT="$snapshot_root/holdout.json" \
    RUST_STYLE_EVAL_HIDDEN_SOURCE="$source_skill" \
    RUST_STYLE_EVAL_HIDDEN_HOME="$original_home/.zshrc" \
    RUST_STYLE_EVAL_HIDDEN_SNAPSHOT="$snapshot_root/source" \
    RUST_STYLE_EVAL_ESCAPE_TARGET="${RUST_STYLE_EVAL_ESCAPE_TARGET:-}" \
    sandbox-exec -D REPOSITORY_ROOT="$repository_root" -D SNAPSHOT_ROOT="$snapshot_root" -D USER_HOME="$original_home" -D WORKSPACE="$workspace" -D RUST_CARGO_BIN="$rust_cargo_bin" -D RUST_TOOLCHAIN="$rust_toolchain" -p "$sandbox_profile" "${command[@]}" 2>"$workspace/.harness/runner.stderr")
  candidate_status=$?
  set -e
  print -rn -- "$candidate_output" > "$workspace/.harness/candidate.txt"

  local sentinel
  sentinel=$(jq -r '.sentinel // empty' <<<$case_json)
  [[ -z "$sentinel" || "$prompt" != *"$sentinel"* ]] || return 90
  [[ -z "$sentinel" ]] || ! grep -RFq -- "$sentinel" "$workspace" || return 91
  cmp -s "$workspace/.candidate/SKILL.md" "$loaded_skill" || return 94
  loaded_references_are_unchanged "$case_json" "$workspace" || return 94
  workspace_is_contained "$workspace" || return 92
  source_is_unchanged || return 93
  return "$candidate_status"
}

if [[ "$is_smoke" == true ]]; then
  hidden_workspace="$temporary_root/hidden-workspace"
  prepare_workspace "$hidden_workspace" "$source_skill"
  hidden_case=$(jq -c 'select(.id == "c4")' "$cases_file")
  run_candidate "$normal_fake_runner" "$hidden_case" "$source_skill" "$hidden_workspace" "ordinary authorized local Rust task" || { print -u2 -r -- "sandbox exposed hidden evaluation data"; exit 1; }

  for attack_kind in outside source; do
    attack_workspace="$temporary_root/$attack_kind-attack-workspace"
    prepare_workspace "$attack_workspace" "$source_skill"
    attack_target="$temporary_root/outside-workspace-sentinel"
    [[ "$attack_kind" == outside ]] || attack_target="$source_skill"
    [[ "$attack_kind" != outside ]] || print -r -- unchanged > "$attack_target"
    export RUST_STYLE_EVAL_ESCAPE_TARGET="$attack_target"
    if run_candidate "$attack_fake_runner" "$hidden_case" "$source_skill" "$attack_workspace" "ordinary authorized local Rust task"; then
      print -u2 -r -- "sandbox allowed $attack_kind mutation"
      exit 1
    fi
    [[ "$attack_kind" != outside || "$(<"$attack_target")" == unchanged ]] || { print -u2 -r -- "outside sentinel changed"; exit 1; }
    source_is_unchanged || { print -u2 -r -- "source mutation detected"; exit 1; }
  done
  unset RUST_STYLE_EVAL_ESCAPE_TARGET
  print -u2 -r -- "sandbox smoke checks passed"
fi

workspace=""
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

check_rust_commands() {
  check_command /bin/zsh -c 'cd "$1" && cargo fmt --check' _ "$workspace"
  check_command /bin/zsh -c 'cd "$1" && cargo check --quiet' _ "$workspace"
  check_command /bin/zsh -c 'cd "$1" && cargo clippy --quiet -- -D warnings' _ "$workspace"
}

verify_fixture() {
  local id=$1
  local fixture=$2
  checks_passed=0
  checks_total=0
  failure_mode=null
  local baseline_dir="$snapshot_root/source/evals/fixtures/${fixture:t}"
  local report=""

  case "$id" in
    c1)
      report="$workspace/REPORT.md"
      check_command test -f "$report"
      check_pattern 'try_parse_port.*Result<u16, ParseIntError>' "$workspace/src/lib.rs"
      check_command /bin/zsh -c 'cd "$1" && cargo test --quiet' _ "$workspace"
      check_rust_commands
      ;;
    c2)
      report="$workspace/REVIEW.md"
      check_command test -f "$report"
      check_command cmp -s "$workspace/src/lib.rs" "$baseline_dir/src/lib.rs"
      check_pattern 'unsafe.*(necessary reason|safety requirements)|safety requirements.*unsafe' "$report"
      check_pattern '#\[expect.*reason' "$report"
      check_rust_commands
      ;;
    c6)
      report="$workspace/REPORT.md"
      check_command test -f "$report"
      check_command cmp -s "$workspace/src/lib.rs" "$baseline_dir/src/lib.rs"
      check_pattern 'reject|unsupported|not justified' "$report"
      check_pattern 'profile|evidence|requirement' "$report"
      check_command /bin/zsh -c 'cd "$1" && cargo test --quiet' _ "$workspace"
      check_rust_commands
      ;;
    *)
      print -u2 -r -- "unknown fixture case: $id"
      exit 2
      ;;
  esac

  check_pattern '^Rust baseline:[[:space:]]*applied' "$report"
  check_pattern '^Checks:.*cargo fmt --check.*pass' "$report"
  check_pattern '^Checks:.*cargo check.*pass' "$report"
  check_pattern '^Checks:.*cargo clippy.*pass' "$report"
  check_pattern '^Exceptions:' "$report"

  if (( checks_passed == checks_total )); then
    failure_mode=null
  elif (( checks_passed == 0 )); then
    failure_mode='"no-observable-artifact"'
  else
    failure_mode='"missing-deterministic-checks"'
  fi
}

grade_response() {
  local case_json=$1
  local actual=$2
  local prompt_file="$workspace/.harness/judge-prompt.txt"
  {
    print -r -- 'Grade only the actual response that the candidate produced while the loaded rust-style skill was available.'
    print -r -- 'Reply with only a JSON object: {"score":<integer 0-10>,"failure_mode":<string or null>}.'
    print -r -- 'Use this rubric:'
    print -r -- "$(<"$rubric_file")"
    print -r -- 'Case expectation:'
    jq -r '.expect' <<<$case_json
    print -r -- 'Actual produced response:'
    print -r -- "$actual"
  } > "$prompt_file"
  local judge_output verdict
  judge_output=$("$skill_eval" judge --prompt "$(<"$prompt_file")") || return $?
  verdict=$(print -r -- "$judge_output" | grep -Eo '\{.*\}' | tail -1)
  jq -ce 'select((.score | type == "number") and (.score % 1 == 0) and (.score >= 0) and (.score <= 10) and (.failure_mode == null or (.failure_mode | type == "string")))' <<<$verdict
}

run_arm() {
  local arm=$1
  local loaded_skill=$2
  integer total_cases=0
  integer total_score=0
  local case_json id source drive fixture source_dir prompt runner_status verdict score output

  while IFS= read -r case_json; do
    id=$(jq -r '.id' <<<$case_json)
    source=$(jq -r '.source' <<<$case_json)
    drive=$(jq -r '.execution.drive.kind' <<<$case_json)
    workspace="$temporary_root/workspaces/$arm-$id"
    prepare_workspace "$workspace" "$loaded_skill"
    prepare_references "$case_json" "$workspace"

    if [[ "$drive" == fixture ]]; then
      fixture=$(jq -r '.execution.drive.source' <<<$case_json)
      source_dir="$script_dir/../$fixture"
      [[ -d "$source_dir" ]] || { print -u2 -r -- "fixture does not exist for $id: $fixture"; exit 2; }
      cp -pR "$source_dir"/. "$workspace"/
      prompt="$(jq -r '.input' <<<$case_json)

Read REQUEST.md and the loaded rust-style skill. Work only in this disposable Rust fixture. Do not access any source project, other case, rubric, holdout, or real home path."
    else
      prompt=$(jq -r '.input' <<<$case_json)
    fi

    if run_candidate "$candidate_runner" "$case_json" "$loaded_skill" "$workspace" "$prompt"; then
      if [[ "$drive" == response ]]; then
        [[ -n "$candidate_output" ]] || { print -u2 -r -- "candidate returned no response for $arm/$id"; exit 1; }
        verdict=$(grade_response "$case_json" "$candidate_output") || { print -u2 -r -- "shared judge failed for $arm/$id"; exit 1; }
        score=$(jq -r '.score' <<<$verdict)
      else
        verify_fixture "$id" "$fixture"
        score=$(( checks_passed * 10 / checks_total ))
        verdict=$(jq -cn --argjson score "$score" --argjson failure_mode "$failure_mode" --argjson checks_passed "$checks_passed" --argjson checks_total "$checks_total" '{score:$score,failure_mode:$failure_mode,checks_passed:$checks_passed,checks_total:$checks_total}')
      fi
    else
      runner_status=$?
      print -u2 -r -- "candidate runner failed for $arm/$id with status $runner_status"
      [[ ! -s "$workspace/.harness/runner.stderr" ]] || print -u2 -r -- "$(<"$workspace/.harness/runner.stderr")"
      score=0
      case "$runner_status" in
        90|91) failure_mode='"holdout-leak"' ;;
        92) failure_mode='"workspace-escape"' ;;
        93) failure_mode='"source-mutation"' ;;
        94) failure_mode='"loaded-skill-mutation"' ;;
        *) failure_mode='"candidate-runner-failed"' ;;
      esac
      verdict=$(jq -cn --argjson score "$score" --argjson failure_mode "$failure_mode" '{score:$score,failure_mode:$failure_mode}')
    fi

    output=$(jq -cn --arg arm "$arm" --arg id "$id" --arg source "$source" --arg drive "$drive" --argjson verdict "$verdict" '{arm:$arm,id:$id,source:$source,drive:$drive} + $verdict')
    print -r -- "$output"
    total_cases=$(( total_cases + 1 ))
    total_score=$(( total_score + score ))
    source_is_unchanged || { print -u2 -r -- "source mutation detected"; exit 1; }
    rm -rf "$workspace"
  done < <(selected_cases)

  [[ "$total_cases" == "$selected_count" ]] || { print -u2 -r -- "not every selected case ran"; exit 1; }
  local mean_hundredths=$(( total_score * 100 / total_cases ))
  printf '%s mean %d.%02d over %d cases (%s slice)\n' "$arm" "$(( mean_hundredths / 100 ))" "$(( mean_hundredths % 100 ))" "$total_cases" "$slice_name" >&2
}

run_arm incumbent "$source_skill"
[[ "$is_comparison" == false ]] || run_arm candidate "$candidate_skill"
source_is_unchanged || { print -u2 -r -- "source mutation detected"; exit 1; }
