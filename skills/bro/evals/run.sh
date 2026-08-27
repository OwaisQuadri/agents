#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
repository_root=${script_dir:h:h:h}
cases_file="$script_dir/cases.jsonl"
rubric_file="$script_dir/rubric.md"
source_skill="$script_dir/../SKILL.md"
candidate_skill="$source_skill"
candidate_runner=${BRO_EVAL_CANDIDATE_RUNNER:-pi}
checker=${BRO_EVAL_CHECKER:-${commands[ste-check]:-$repository_root/tools/ste-check/target/release/ste-check}}
is_holdout=false
is_dry_run=false
is_smoke=false

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
      shift 2
      ;;
    --help)
      print -r -- "usage: ./run.sh [--holdout] [--dry-run|--smoke] [--candidate-skill path]"
      exit 0
      ;;
    --*)
      print -u2 -r -- "unknown option: $1"
      exit 2
      ;;
    *)
      candidate_skill=$1
      shift
      ;;
  esac
done

[[ "$is_dry_run" != true || "$is_smoke" != true ]] || { print -u2 -r -- "choose dry-run or smoke"; exit 2; }
[[ -f "$candidate_skill" ]] || { print -u2 -r -- "candidate skill does not exist: $candidate_skill"; exit 2; }
[[ -x "$checker" ]] || { print -u2 -r -- "ste-check does not exist: $checker"; exit 2; }

jq -e -s '
  length == 13 and
  (map(.id) | unique | length) == length and
  (map(select(.holdout == true)) | length) == 4 and
  (map(select(.holdout != true)) | length) == 9 and
  all(
    (.id | type == "string" and length > 0) and
    (.input | type == "string" and length > 0) and
    (.expect | type == "string" and length > 0) and
    (.source | type == "string" and length > 0) and
    (.execution.drive.kind == "response") and
    (.execution.allowed_tools == []) and
    (.execution.timeout_seconds | type == "number" and . > 0)
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
[[ "$is_holdout" == false ]] || slice_name=holdout
selected_count=$(selected_cases | wc -l | tr -d ' ')
expected_count=9
[[ "$is_holdout" == false ]] || expected_count=4
[[ "$selected_count" == "$expected_count" ]] || { print -u2 -r -- "wrong $slice_name slice size"; exit 2; }

if [[ "$is_dry_run" == true ]]; then
  while IFS= read -r case_json; do
    jq -cn --arg id "$(jq -r '.id' <<<"$case_json")" --arg source "$(jq -r '.source' <<<"$case_json")" --arg slice "$slice_name" '{id:$id,source:$source,slice:$slice,drive:"response",status:"ready"}'
  done < <(selected_cases)
  print -u2 -r -- "dry-run ready: $selected_count cases ($slice_name slice)"
  exit 0
fi

[[ "$is_smoke" == true || "${BRO_EVAL_LIVE:-0}" == 1 ]] || { print -u2 -r -- "candidate execution requires BRO_EVAL_LIVE=1 or --smoke"; exit 2; }
command -v sandbox-exec >/dev/null || { print -u2 -r -- "sandbox-exec is required"; exit 2; }
command -v skill-eval >/dev/null || [[ "$is_smoke" == true ]] || { print -u2 -r -- "skill-eval is required"; exit 2; }

original_home=${HOME:A}
original_candidate=${candidate_skill:A}
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/bro-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/bro-eval-snapshot.XXXXXX")
snapshot_root=${snapshot_root:A}
trap 'rm -rf "$temporary_root" "$snapshot_root"' EXIT INT TERM
cp -pR "$script_dir/.." "$snapshot_root/source"
cp -p "$candidate_skill" "$snapshot_root/candidate-skill"

if [[ "$is_smoke" == true ]]; then
  candidate_skill="$temporary_root/smoke-candidate-SKILL.md"
  cp -p "$source_skill" "$candidate_skill"
  print -r -- "smoke-candidate-marker: BRO-EVAL-LOADED-SKILL" >> "$candidate_skill"
  candidate_runner=${BRO_EVAL_CANDIDATE_RUNNER:-$script_dir/fake-candidate-runner.zsh}
fi

source_is_unchanged() {
  diff -qr "$script_dir/.." "$snapshot_root/source" >/dev/null &&
    cmp -s "$snapshot_root/candidate-skill" "$original_candidate"
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

run_candidate() {
  local runner=$1
  local id=$2
  local input=$3
  local workspace=$4
  local runner_path
  runner_path=$(resolve_runner "$runner" "$workspace") || return $?
  local skill_sha input_sha
  skill_sha=$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)
  input_sha=$(print -rn -- "$input" | shasum -a 256 | cut -d ' ' -f 1)
  local sandbox_profile='(version 1)
(allow default)
(deny file-read* (subpath (param "REPOSITORY_ROOT")))
(deny file-read* (subpath (param "USER_HOME")))
(deny file-read* (subpath (param "SNAPSHOT_ROOT")))
(deny file-write* (require-not (subpath (param "WORKSPACE"))))'
  local prompt=$'Use the loaded bro skill. Treat the MESSAGE in this task as your own most recent assistant message. Return only the plain-words version. Do not add a preamble or commentary.\n\nTASK:\n'"$input"
  local -a command
  command=("$runner_path" -p --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/SKILL.md" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve --no-tools "$prompt")

  set +e
  candidate_output=$(cd "$workspace" && env \
    HOME="$workspace/.home" \
    PI_CODING_AGENT_DIR="$workspace/.pi/config" \
    PI_CONFIG_DIR="$workspace/.pi/config" \
    PI_CODING_AGENT_SESSION_DIR="$workspace/.pi/session" \
    XDG_CONFIG_HOME="$workspace/.xdg/config" \
    XDG_CACHE_HOME="$workspace/.xdg/cache" \
    XDG_DATA_HOME="$workspace/.xdg/data" \
    TMPDIR="$workspace/.tmp" \
    PI_SKIP_VERSION_CHECK=1 \
    PI_TELEMETRY=0 \
    BRO_EVAL_CASE_ID="$id" \
    BRO_EVAL_CASE_INPUT_SHA="$input_sha" \
    BRO_EVAL_EXPECTED_SKILL_SHA="$skill_sha" \
    BRO_EVAL_WORKSPACE="$workspace" \
    BRO_EVAL_ESCAPE_TARGET="${BRO_EVAL_ESCAPE_TARGET:-}" \
    sandbox-exec -D REPOSITORY_ROOT="$repository_root" -D USER_HOME="$original_home" -D SNAPSHOT_ROOT="$snapshot_root" -D WORKSPACE="$workspace" -p "$sandbox_profile" "${command[@]}" 2>"$workspace/.harness/runner.stderr")
  candidate_status=$?
  set -e
  print -rn -- "$candidate_output" > "$workspace/.harness/candidate.txt"
  workspace_is_contained "$workspace" || return 92
  source_is_unchanged || return 93
  return "$candidate_status"
}

prepare_workspace() {
  local workspace=$1
  mkdir -p "$workspace/.candidate" "$workspace/.harness" "$workspace/.home" "$workspace/.pi/config" "$workspace/.pi/session" "$workspace/.xdg/config" "$workspace/.xdg/cache" "$workspace/.xdg/data" "$workspace/.tmp"
  cp -p "$candidate_skill" "$workspace/.candidate/SKILL.md"
}

if [[ "$is_smoke" == true ]]; then
  hidden_workspace="$temporary_root/hidden-workspace"
  prepare_workspace "$hidden_workspace"
  export BRO_EVAL_HIDDEN_RUBRIC="$rubric_file"
  export BRO_EVAL_HIDDEN_CASES="$cases_file"
  export BRO_EVAL_HIDDEN_SOURCE="$source_skill"
  export BRO_EVAL_HIDDEN_HOME="$original_home/.zshrc"
  export BRO_EVAL_HIDDEN_SNAPSHOT="$snapshot_root/candidate-skill"
  if run_candidate "$script_dir/fake-candidate-hidden.zsh" hidden "ordinary authorized response" "$hidden_workspace"; then
    :
  else
    print -u2 -r -- "sandbox exposed a hidden path"
    exit 1
  fi
  unset BRO_EVAL_HIDDEN_RUBRIC BRO_EVAL_HIDDEN_CASES BRO_EVAL_HIDDEN_SOURCE BRO_EVAL_HIDDEN_HOME BRO_EVAL_HIDDEN_SNAPSHOT

  escape_workspace="$temporary_root/escape-workspace"
  prepare_workspace "$escape_workspace"
  escape_target="$temporary_root/outside-workspace-sentinel"
  print -r -- unchanged > "$escape_target"
  BRO_EVAL_ESCAPE_TARGET="$escape_target"
  if run_candidate "$script_dir/fake-candidate-escape.zsh" escape "ordinary authorized response" "$escape_workspace"; then
    print -u2 -r -- "sandbox allowed an outside-workspace mutation"
    exit 1
  fi
  [[ "$(<"$escape_target")" == unchanged ]] || { print -u2 -r -- "outside-workspace sentinel changed"; exit 1; }
  unset BRO_EVAL_ESCAPE_TARGET
fi

integer total_cases=0
integer total_score=0
while IFS= read -r case_json; do
  id=$(jq -r '.id' <<<"$case_json")
  input=$(jq -r '.input' <<<"$case_json")
  expect=$(jq -r '.expect' <<<"$case_json")
  source=$(jq -r '.source' <<<"$case_json")
  workspace="$temporary_root/workspaces/$id"
  prepare_workspace "$workspace"

  if run_candidate "$candidate_runner" "$id" "$input" "$workspace"; then
    :
  else
    runner_status=$?
    output=$(jq -cn --arg id "$id" --arg source "$source" --argjson score 0 --arg failure_mode "candidate-runner-failed-$runner_status" '{id:$id,source:$source,score:$score,failure_mode:$failure_mode}')
    print -r -- "$output"
    exit 1
  fi
  candidate=${candidate_output%$'\n'}
  [[ -n "$candidate" ]] || { print -u2 -r -- "candidate returned no output for $id"; exit 1; }

  set +e
  checker_output=$(print -rn -- "$candidate" | "$checker" --register bro 2>&1)
  checker_status=$?
  set -e
  mechanical_lines=$(print -r -- "$checker_output" | grep '^FAIL' || true)

  if [[ "$is_smoke" == true ]]; then
    [[ -z "$mechanical_lines" && "$checker_status" == 0 ]] || { print -u2 -r -- "fake response failed ste-check for $id\n$checker_output"; exit 1; }
    output=$(jq -cn --arg id "$id" --arg source "$source" '{id:$id,source:$source,drive:"response",candidate:"executed",checker:"pass",status:"smoke-pass"}')
    print -r -- "$output"
    total_cases=$(( total_cases + 1 ))
    continue
  fi

  judge_prompt=$(printf '%s\n\n%s\n\n%s\n%s\n\n%s\n%s\n\n%s\n%s\n\n%s\n' \
    'Grade one produced bro response. Reply with only a JSON object: {"score": <integer 0-10>, "failure_mode": "<short tag>" or null}.' \
    'RUBRIC:' "$(<"$rubric_file")" \
    'CASE INPUT:' "$input" \
    'EXPECT:' "$expect" \
    'PRODUCED RESPONSE:' "$candidate")
  judge_output=$(skill-eval judge --prompt "$judge_prompt")
  verdict=$(print -r -- "$judge_output" | jq -ce 'select((.score | type == "number") and (.score % 1 == 0) and (.score >= 0) and (.score <= 10) and (.failure_mode == null or (.failure_mode | type == "string")))')
  score=$(jq -r '.score' <<<"$verdict")
  failure_mode=$(jq -c '.failure_mode' <<<"$verdict")

  if [[ -n "$mechanical_lines" || "$checker_status" != 0 ]]; then
    (( score <= 4 )) || score=4
    mechanical_tag=${${(j:; :)${(f)mechanical_lines}}:-ste-check failed}
    failure_mode=$(jq -cn --arg tag "mechanical: $mechanical_tag" '$tag')
  fi

  output=$(jq -cn --arg id "$id" --arg source "$source" --argjson score "$score" --argjson failure_mode "$failure_mode" '{id:$id,source:$source,score:$score,failure_mode:$failure_mode}')
  print -r -- "$output"
  total_cases=$(( total_cases + 1 ))
  total_score=$(( total_score + score ))
done < <(selected_cases)

[[ "$total_cases" == "$selected_count" ]] || { print -u2 -r -- "not every selected case ran"; exit 1; }
source_is_unchanged || { print -u2 -r -- "source mutation detected"; exit 1; }

if [[ "$is_smoke" == true ]]; then
  print -u2 -r -- "smoke pass: $total_cases candidates executed ($slice_name slice)"
else
  mean_hundredths=$(( total_score * 100 / total_cases ))
  printf 'mean %d.%02d over %d cases (%s slice)\n' "$(( mean_hundredths / 100 ))" "$(( mean_hundredths % 100 ))" "$total_cases" "$slice_name" >&2
fi
