#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
skill_root=${script_dir:h}
repository_root=${skill_root:h:h}
cases_file="$script_dir/cases.jsonl"
rubric_file="$script_dir/rubric.md"
source_skill="$skill_root/SKILL.md"
candidate_skill="$source_skill"
candidate_runner=${SESSION_STATS_EVAL_CANDIDATE_RUNNER:-pi}
normal_fake_runner=${SESSION_STATS_EVAL_NORMAL_FAKE_RUNNER:-$script_dir/fake-candidate-normal.zsh}
attack_fake_runner=${SESSION_STATS_EVAL_ATTACK_FAKE_RUNNER:-$script_dir/fake-candidate-attack.zsh}
skill_eval=${SESSION_STATS_EVAL_SKILL_EVAL:-skill-eval}
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
  length == 5 and
  (map(.id) | unique | length) == length and
  (map(select(.holdout == true)) | length) == 1 and
  (map(select(.holdout != true)) | length) == 4 and
  all(.[];
    (.id | type == "string" and length > 0) and
    (.input | type == "string" and length > 0) and
    (.expect | type == "string" and length > 0) and
    (.source | type == "string" and length > 0) and
    (.holdout | type == "boolean") and
    (.execution.drive.kind == "response" or .execution.drive.kind == "fixture" or .execution.drive.kind == "existing_harness") and
    (.execution.allowed_tools == ["bash"]) and
    (if .execution.drive.kind == "fixture" or .execution.drive.kind == "existing_harness" then
      (.execution.drive.source | type == "string" and startswith("evals/fixtures/"))
    else true end)
  ) and
  all(.[] | select(.holdout == true); (.sentinel | type == "string" and length > 0)) and
  (map(.input | ascii_downcase) | all(
    (contains("personal history") or contains("credential") or contains("attack") or contains("exploit")) | not
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
expected_count=4
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

[[ "$is_smoke" == true || "${SESSION_STATS_EVAL_LIVE:-0}" == 1 ]] || { print -u2 -r -- 'candidate execution requires SESSION_STATS_EVAL_LIVE=1 or --smoke'; exit 2; }
[[ "$is_smoke" == true || -n "$candidate_model" ]] || { print -u2 -r -- 'candidate execution requires --candidate-model'; exit 2; }
command -v sandbox-exec >/dev/null || { print -u2 -r -- 'sandbox-exec is required'; exit 2; }
command -v jq >/dev/null || { print -u2 -r -- 'jq is required'; exit 2; }
if [[ "$is_smoke" == true ]]; then
  skill_eval="$script_dir/fake-skill-eval.zsh"
else
  command -v "$skill_eval" >/dev/null || { print -u2 -r -- 'skill-eval is required'; exit 2; }
fi

original_home=${HOME:A}
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/session-stats-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/session-stats-snapshot.XXXXXX")
snapshot_root=${snapshot_root:A}
trap 'rm -rf "$temporary_root" "$snapshot_root"' EXIT INT TERM
cp -pR "$script_dir" "$snapshot_root/evals"
cp -p "$source_skill" "$snapshot_root/source-skill"
cp -p "$candidate_skill" "$snapshot_root/candidate-skill"
jq -c 'select(.holdout == true)' "$cases_file" > "$snapshot_root/holdout.json"

source_is_unchanged() {
  diff -qr "$script_dir" "$snapshot_root/evals" >/dev/null &&
    cmp -s "$source_skill" "$snapshot_root/source-skill" &&
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
    cp -L "$resolved" "$workspace/.bin/candidate-runner"
    chmod +x "$workspace/.bin/candidate-runner"
    print -r -- "$workspace/.bin/candidate-runner"
  else
    print -r -- "$resolved"
  fi
}

prepare_workspace() {
  local workspace=$1
  local loaded_skill=$2
  mkdir -p "$workspace/.candidate" "$workspace/.bin" "$workspace/.audit" "$workspace/.home/.claude/projects" "$workspace/.pi/config" "$workspace/.pi/session" "$workspace/.xdg/config" "$workspace/.xdg/cache" "$workspace/.xdg/data" "$workspace/.tmp"
  cp -p "$loaded_skill" "$workspace/.candidate/SKILL.md"
  cp -pR "$script_dir/fixtures/claude" "$workspace/.home/.claude/projects/fixture"
  cp -p "$script_dir/fake-session-stats.zsh" "$workspace/.bin/session-stats"
  cp -p "$script_dir/fake-jq.zsh" "$workspace/.bin/jq"
  cp -p "$script_dir/fake-python3.zsh" "$workspace/.bin/python3"
  cp -p "$script_dir/fake-python3.zsh" "$workspace/.bin/python"
  chmod +x "$workspace/.bin/"*
  : > "$workspace/.audit/commands.txt"
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
  local sandbox_profile='(version 1)
(allow default)
(deny network*)
(deny file-read* (subpath (param "REPOSITORY_ROOT")))
(deny file-read* (subpath (param "SNAPSHOT_ROOT")))
(deny file-read* (subpath (param "SESSION_STORE")))
(deny file-read* (require-all (subpath (param "USER_HOME")) (require-not (subpath (param "WORKSPACE")))))
(deny file-write* (require-all (require-not (subpath (param "WORKSPACE"))) (require-not (literal "/dev/null"))))'
  local -a command
  command=("$runner_path" -p --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/SKILL.md" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve --tools bash)
  [[ -z "$candidate_model" ]] || command+=(--model "$candidate_model")
  command+=("$prompt")

  set +e
  candidate_output=$(cd "$workspace" && env -i \
    PATH="$workspace/.bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
    HOME="$workspace/.home" \
    PI_CODING_AGENT_DIR="$workspace/.pi/config" \
    PI_CONFIG_DIR="$workspace/.pi/config" \
    PI_CODING_AGENT_SESSION_DIR="$workspace/.pi/session" \
    XDG_CONFIG_HOME="$workspace/.xdg/config" \
    XDG_CACHE_HOME="$workspace/.xdg/cache" \
    XDG_DATA_HOME="$workspace/.xdg/data" \
    TMPDIR="$workspace/.tmp" \
    PI_SKIP_VERSION_CHECK=1 PI_TELEMETRY=0 \
    SESSION_STATS_EVAL_WORKSPACE="$workspace" \
    SESSION_STATS_EVAL_AUDIT="$workspace/.audit/commands.txt" \
    SESSION_STATS_EVAL_CASE_ID="$(jq -r '.id' <<<$case_json)" \
    SESSION_STATS_EVAL_EXPECTED_SKILL_SHA="$skill_sha" \
    SESSION_STATS_EVAL_HIDDEN_RUBRIC="$rubric_file" \
    SESSION_STATS_EVAL_HIDDEN_CASES="$cases_file" \
    SESSION_STATS_EVAL_HIDDEN_HOLDOUT="$snapshot_root/holdout.json" \
    SESSION_STATS_EVAL_HIDDEN_SOURCE="$source_skill" \
    SESSION_STATS_EVAL_HIDDEN_HOME="$original_home/.zshrc" \
    SESSION_STATS_EVAL_ESCAPE_TARGET="${SESSION_STATS_EVAL_ESCAPE_TARGET:-}" \
    sandbox-exec -D REPOSITORY_ROOT="$repository_root" -D SNAPSHOT_ROOT="$snapshot_root" -D SESSION_STORE="$workspace/.home/.claude" -D USER_HOME="$original_home" -D WORKSPACE="$workspace" -p "$sandbox_profile" "${command[@]}" 2>"$workspace/.audit/runner.stderr")
  candidate_status=$?
  set -e
  print -rn -- "$candidate_output" > "$workspace/.audit/candidate.txt"

  local sentinel
  sentinel=$(jq -r '.sentinel // empty' <<<$case_json)
  [[ -z "$sentinel" || "$prompt" != *"$sentinel"* ]] || return 90
  [[ -z "$sentinel" ]] || ! grep -RFq -- "$sentinel" "$workspace" || return 91
  cmp -s "$workspace/.candidate/SKILL.md" "$loaded_skill" || return 94
  workspace_is_contained "$workspace" || return 92
  source_is_unchanged || return 93
  return "$candidate_status"
}

if [[ "$is_smoke" == true ]]; then
  smoke_case=$(jq -c 'select(.id == "analysis-not-transcripts")' "$cases_file")
  hidden_workspace="$temporary_root/hidden-workspace"
  prepare_workspace "$hidden_workspace" "$source_skill"
  if ! run_candidate "$normal_fake_runner" "$smoke_case" "$source_skill" "$hidden_workspace" 'Use the loaded session-stats skill for this ordinary authorized synthetic-history question.'; then
    [[ ! -s "$hidden_workspace/.audit/runner.stderr" ]] || command cat "$hidden_workspace/.audit/runner.stderr" >&2
    print -u2 -r -- 'sandbox exposed hidden evaluation data'
    exit 1
  fi

  for attack_kind in outside source; do
    attack_workspace="$temporary_root/$attack_kind-attack-workspace"
    prepare_workspace "$attack_workspace" "$source_skill"
    attack_target="$temporary_root/outside-workspace-sentinel"
    [[ "$attack_kind" == outside ]] || attack_target="$source_skill"
    [[ "$attack_kind" != outside ]] || print -r -- unchanged > "$attack_target"
    export SESSION_STATS_EVAL_ESCAPE_TARGET="$attack_target"
    if run_candidate "$attack_fake_runner" "$smoke_case" "$source_skill" "$attack_workspace" 'Use the loaded session-stats skill for this ordinary authorized synthetic-history question.'; then
      print -u2 -r -- "sandbox allowed $attack_kind mutation"
      exit 1
    fi
    [[ "$attack_kind" != outside || "$(<"$attack_target")" == unchanged ]] || { print -u2 -r -- 'outside sentinel changed'; exit 1; }
    source_is_unchanged || { print -u2 -r -- 'source mutation detected'; exit 1; }
  done
  unset SESSION_STATS_EVAL_ESCAPE_TARGET
  print -u2 -r -- 'sandbox smoke checks passed'
fi

workspace=""
integer checks_passed=0
integer checks_total=0
failure_mode=null

check_command() {
  checks_total=$(( checks_total + 1 ))
  if "$@" >/dev/null 2>&1; then
    checks_passed=$(( checks_passed + 1 ))
  fi
}

check_pattern() {
  local pattern=$1
  local file=$2
  check_command grep -Eiq -- "$pattern" "$file"
}

verify_case() {
  local id=$1
  checks_passed=0
  checks_total=0
  failure_mode=null
  local output="$workspace/.audit/candidate.txt"
  local audit="$workspace/.audit/commands.txt"

  check_command jq -e 'length == 1 and .[0].src == "claude" and .[0].model == "claude-test-1" and .[0].input == 110 and .[0].output == 90 and .[0].cacheRead == 6000 and .[0].cacheCreate == 500 and .[0].messages == 2 and .[0].firstCtx == 1300 and .[0].lastCtx == 5310' "$workspace/compiled.json"
  check_pattern 'session-stats --json /tmp/session-stats[.]json' "$audit"
  check_pattern "$workspace/compiled[.]json" "$output"
  check_command /bin/zsh -c '! grep -Eiq "aaaa-session[.]jsonl|[.]claude/projects" "$1" "$2"' _ "$audit" "$output"

  case "$id" in
    fixture-shape)
      check_pattern 'input=110' "$output"
      check_pattern 'output=90' "$output"
      check_pattern 'cacheRead=6000' "$output"
      check_pattern 'cacheCreate=500' "$output"
      check_pattern 'messages=2' "$output"
      check_pattern 'firstCtx=1300' "$output"
      check_pattern 'lastCtx=5310' "$output"
      check_pattern '^(jq|python3?) ' "$audit"
      check_command /bin/zsh -c '! grep -q -- "--open" "$1"' _ "$audit"
      ;;
    dedup)
      check_pattern 'repeated m1.*(adds no|does not add).*tokens' "$output"
      check_pattern 'messages=2' "$output"
      check_pattern 'input=110' "$output"
      check_pattern 'output=90' "$output"
      check_pattern '^(jq|python3?) ' "$audit"
      check_command /bin/zsh -c '! grep -q -- "--open" "$1"' _ "$audit"
      ;;
    synthetic-model)
      check_pattern 'claude-test-1' "$output"
      check_pattern '<synthetic>.*no row|no row.*<synthetic>' "$output"
      check_pattern '^(jq|python3?) ' "$audit"
      check_command /bin/zsh -c '! grep -q -- "--open" "$1"' _ "$audit"
      ;;
    analysis-not-transcripts)
      check_pattern 'claude-test-1.*90 output tokens' "$output"
      check_pattern 'source=claude' "$output"
      check_pattern 'model=claude-test-1' "$output"
      check_pattern 'date=2026-01-01' "$output"
      check_pattern '^(jq|python3?) ' "$audit"
      check_command /bin/zsh -c '! grep -q -- "--open" "$1"' _ "$audit"
      ;;
    view-on-request)
      check_pattern 'session-stats --out /tmp/session-stats[.]html --open' "$audit"
      check_command test -s "$workspace/session-stats.html"
      check_pattern "$workspace/session-stats[.]html" "$output"
      check_pattern 'disposable synthetic history' "$output"
      ;;
    *) print -u2 -r -- "unknown case identifier: $id"; exit 2 ;;
  esac

  if (( checks_passed == checks_total )); then
    failure_mode=null
  elif (( checks_passed == 0 )); then
    failure_mode='"no-observable-output"'
  else
    failure_mode='"missing-deterministic-checks"'
  fi
}

grade_output() {
  local case_json=$1
  local actual=$2
  local prompt_file="$workspace/.audit/judge-prompt.txt"
  {
    print -r -- 'Grade only the actual answer that the candidate produced after it executed the loaded session-stats skill.'
    print -r -- 'Reply with only JSON: {"score":<integer 0-10>,"failure_mode":<string or null>}.'
    print -r -- 'Rubric:'
    command cat "$rubric_file"
    print -r -- 'Case expectation:'
    jq -r '.expect' <<<$case_json
    print -r -- 'Actual produced answer:'
    print -r -- "$actual"
  } > "$prompt_file"
  local judge_text verdict
  judge_text=$("$skill_eval" judge --prompt "$(<"$prompt_file")") || return $?
  verdict=$(print -r -- "$judge_text" | grep -Eo '\{.*\}' | tail -1)
  jq -ce 'select((.score | type == "number") and (.score % 1 == 0) and (.score >= 0) and (.score <= 10) and (.failure_mode == null or (.failure_mode | type == "string")))' <<<$verdict
}

run_arm() {
  local arm=$1
  local loaded_skill=$2
  local runner=$candidate_runner
  [[ "$is_smoke" == false ]] || runner=$normal_fake_runner
  integer total_cases=0
  integer total_score=0
  local case_json id source drive prompt runner_status verdict score deterministic_score judge_score output

  while IFS= read -r case_json; do
    id=$(jq -r '.id' <<<$case_json)
    source=$(jq -r '.source' <<<$case_json)
    drive=$(jq -r '.execution.drive.kind' <<<$case_json)
    workspace="$temporary_root/workspaces/$arm-$id"
    prepare_workspace "$workspace" "$loaded_skill"
    prompt="$(jq -r '.input' <<<$case_json)

Use the loaded session-stats skill. This workspace contains only ordinary synthetic logs. The fake binary maps requested /tmp outputs into this disposable workspace. Use the path that the binary reports. Do not read raw logs. Do not access the source project, rubric, other cases, holdout, or real home."

    if run_candidate "$runner" "$case_json" "$loaded_skill" "$workspace" "$prompt"; then
      verify_case "$id"
      deterministic_score=$(( checks_passed * 10 / checks_total ))
      score=$deterministic_score
      if [[ "$drive" == response ]]; then
        verdict=$(grade_output "$case_json" "$candidate_output") || { print -u2 -r -- "shared skill-eval judge failed for $arm/$id"; exit 1; }
        judge_score=$(jq -r '.score' <<<$verdict)
        (( score <= judge_score )) || score=$judge_score
        if [[ "$failure_mode" == null ]]; then
          failure_mode=$(jq -c '.failure_mode' <<<$verdict)
        fi
      fi
    else
      runner_status=$?
      score=0
      checks_passed=0
      checks_total=1
      case "$runner_status" in
        90|91) failure_mode='"holdout-leak"' ;;
        92) failure_mode='"workspace-escape"' ;;
        93) failure_mode='"source-mutation"' ;;
        94) failure_mode='"loaded-skill-mutation"' ;;
        *) failure_mode='"candidate-runner-failed"' ;;
      esac
    fi

    output=$(jq -cn --arg arm "$arm" --arg id "$id" --arg source "$source" --arg drive "$drive" --argjson score "$score" --argjson failure_mode "$failure_mode" --argjson checks_passed "$checks_passed" --argjson checks_total "$checks_total" '{arm:$arm,id:$id,source:$source,drive:$drive,score:$score,failure_mode:$failure_mode,checks_passed:$checks_passed,checks_total:$checks_total}')
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
