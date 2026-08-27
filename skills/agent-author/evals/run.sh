#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
cases_file="$script_dir/cases.jsonl"
source_skill="$script_dir/../SKILL.md"
candidate_skill="$source_skill"
candidate_model=""
pi_runner=${AGENT_AUTHOR_EVAL_PI:-pi}
is_holdout=false
is_dry_run=false

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
    --candidate|--candidate-skill)
      candidate_skill=${2:?missing candidate skill path}
      shift 2
      ;;
    --model|--candidate-model)
      candidate_model=${2:?missing candidate model}
      shift 2
      ;;
    --help)
      print -r -- "usage: ./run.sh [--holdout] [--dry-run] [--candidate-skill path] [--candidate-model provider/model]"
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

[[ -f "$candidate_skill" ]] || { print -u2 -r -- "candidate skill does not exist: $candidate_skill"; exit 2; }
jq -e -s 'length == 6 and (map(select(.holdout == true)) | length == 1) and (map(select(.holdout != true)) | length == 5)' "$cases_file" >/dev/null

heldout_sentinel=$(jq -er 'select(.holdout == true) | .sentinel' "$cases_file")
source_sentinel="$script_dir/source-sentinel.txt"
[[ -s "$source_sentinel" ]] || { print -u2 -r -- "source sentinel is missing"; exit 2; }
source_sentinel_value=$(<"$source_sentinel")

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/agent-author-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/agent-author-snapshot.XXXXXX")
snapshot_root=${snapshot_root:A}
trap 'rm -rf "$temporary_root" "$snapshot_root"' EXIT INT TERM
cp -p "$source_skill" "$snapshot_root/SKILL.md"
cp -pR "$script_dir" "$snapshot_root/evals"

source_is_unchanged() {
  cmp -s "$source_skill" "$snapshot_root/SKILL.md" &&
    diff -qr "$script_dir" "$snapshot_root/evals" >/dev/null &&
    [[ "$(<"$source_sentinel")" == "$source_sentinel_value" ]]
}

selected_cases() {
  if [[ "$is_holdout" == true ]]; then
    jq -c 'select(.holdout == true)' "$cases_file"
  else
    jq -c 'select(.holdout != true)' "$cases_file"
  fi
}

integer total_cases=0
integer total_score=0
integer checks_passed=0
integer checks_total=0
failure_mode=""
workspace=""

check_command() {
  checks_total=$(( checks_total + 1 ))
  if "$@" >/dev/null 2>&1; then
    checks_passed=$(( checks_passed + 1 ))
  fi
}

check_text() {
  local pattern=$1
  local file=$2
  check_command grep -Eiq -- "$pattern" "$file"
}

check_agent_scaffold() {
  local name=$1
  local tier=$2
  local definition="$workspace/agents/$name/$name.md"
  local root="$workspace/agents/$name"
  local eval_cases="$root/evals/cases.jsonl"

  check_command test -f "$definition"
  if [[ -f "$definition" ]]; then
    if grep -Eq '^model:[[:space:]]*' "$definition"; then
      catastrophic_tag="pinned-model"
    fi
    if ! grep -Eiq 'input contract' "$definition" || ! grep -Eiq 'output contract' "$definition" || ! grep -Eiq 'context discipline|must not receive|exclude' "$definition"; then
      catastrophic_tag="missing-core-contract"
    fi
    check_text '^name:[[:space:]]*'"$name"'[[:space:]]*$' "$definition"
    check_text '^description:' "$definition"
    check_text 'use when|use-when' "$definition"
    check_text 'skip when|skip-when' "$definition"
    check_text 'input contract' "$definition"
    check_text 'output contract' "$definition"
    check_text 'context discipline|must not receive|exclude' "$definition"
    check_text 'trigger conditions' "$definition"
    check_text 'success rubric' "$definition"
    check_text 'failure.mode|watch-list' "$definition"
    check_text '^## logging[[:space:]]*$' "$definition"
    check_command /bin/zsh -c '! grep -Eq "^model:[[:space:]]*" "$1"' _ "$definition"
  else
    checks_total=$(( checks_total + 11 ))
  fi
  check_command test -f "$root/evals/rubric.md"
  check_command test -f "$root/evals/run.sh"
  check_command test -f "$root/logs/usage.jsonl"
  check_command test -f "$root/votes/votes.jsonl"
  check_command test -f "$eval_cases"
  if [[ -f "$definition" && ( ! -f "$root/evals/rubric.md" || ! -f "$root/evals/run.sh" || ! -f "$eval_cases" ) ]]; then
    catastrophic_tag="missing-harness"
  fi
  if [[ -f "$eval_cases" ]]; then
    check_command jq -e -s 'length >= 5' "$eval_cases"
    check_command jq -e -s 'map(select(.holdout == true)) | length >= 1' "$eval_cases"
    check_command jq -e -s 'map(.input | ascii_downcase) | any(contains("missing"))' "$eval_cases"
    check_command jq -e -s 'map(.input | ascii_downcase) | any(contains("out of trigger") or contains("outside") or contains("decline"))' "$eval_cases"
  else
    checks_total=$(( checks_total + 4 ))
  fi
  check_command jq -e --arg name "$name" --arg tier "$tier" '.agents[$name] == $tier' "$workspace/config/model-tiers.json"
}

verify_case() {
  local id=$1
  checks_passed=0
  checks_total=0
  failure_mode=""
  local primary_path=""
  local catastrophic_tag=""

  case "$id" in
    a1-contract-checker)
      primary_path="$workspace/agents/dependency-contract-checker/dependency-contract-checker.md"
      check_agent_scaffold dependency-contract-checker T4
      local definition="$primary_path"
      if [[ -f "$definition" ]]; then
        check_text 'manifest_path' "$definition"
        check_text 'dependency_name' "$definition"
        check_text 'verdict.*reason.*anchor|anchor.*reason.*verdict' "$definition"
        check_command /bin/zsh -c '! grep -Eiq "tools:.*(edit|write|bash|shell)" "$1"' _ "$definition"
        if grep -Eiq 'tools:.*(edit|write|bash|shell)' "$definition"; then
          catastrophic_tag="write-overgrant"
        fi
      else
        checks_total=$(( checks_total + 4 ))
      fi
      ;;
    a2-json-normalizer)
      primary_path="$workspace/agents/json-normalizer/json-normalizer.md"
      check_agent_scaffold json-normalizer T2
      local definition="$primary_path"
      if [[ -f "$definition" ]]; then
        check_text 'record' "$definition"
        check_text 'labels' "$definition"
        check_command /bin/zsh -c '! grep -Eq "^tools:[[:space:]]*.+" "$1"' _ "$definition"
      else
        checks_total=$(( checks_total + 3 ))
      fi
      ;;
    a3-release-checklist)
      primary_path="$workspace/decision.json"
      check_command jq -e '.verdict == "skill" and (.reason | type == "string" and length > 0)' "$primary_path"
      check_command /bin/zsh -c '! find "$1/agents" -type f -name "*.md" -print -quit 2>/dev/null | grep -q .' _ "$workspace"
      check_command /bin/zsh -c '! test -s "$1/config/model-tiers.json"' _ "$workspace"
      if find "$workspace/agents" -type f -name '*.md' -print -quit 2>/dev/null | grep -q .; then
        catastrophic_tag="wrong-artifact-type"
      fi
      ;;
    a4-missing-dispatch-input)
      primary_path="$workspace/dispatch-gap.json"
      check_command jq -e '.verdict == "invalid-dispatch" and .missing == "source_path"' "$primary_path"
      check_command /bin/zsh -c '! find "$1/agents" -type f -name "*.md" -print -quit 2>/dev/null | grep -q .' _ "$workspace"
      check_command /bin/zsh -c '! test -s "$1/config/model-tiers.json"' _ "$workspace"
      if find "$workspace/agents" -type f -name '*.md' -print -quit 2>/dev/null | grep -q .; then
        catastrophic_tag="guessed-missing-input"
      fi
      ;;
    a5-fresh-reviewer)
      primary_path="$workspace/agents/change-finding-reviewer/change-finding-reviewer.md"
      check_agent_scaffold change-finding-reviewer T4
      local definition="$primary_path"
      if [[ -f "$definition" ]]; then
        check_text 'finding' "$definition"
        check_text 'source_path' "$definition"
        check_text 'builder.*transcript|transcript.*builder' "$definition"
        check_text 'prior.*verdict|verdict.*prior' "$definition"
        check_command /bin/zsh -c '! grep -Eiq "tools:.*(edit|write|bash|shell)" "$1"' _ "$definition"
        if grep -Eiq 'tools:.*(edit|write|bash|shell)' "$definition"; then
          catastrophic_tag="write-overgrant"
        fi
      else
        checks_total=$(( checks_total + 5 ))
      fi
      ;;
    h1-fixture-curator)
      primary_path="$workspace/agents/fixture-curator/fixture-curator.md"
      check_agent_scaffold fixture-curator T3
      local definition="$primary_path"
      if [[ -f "$definition" ]]; then
        check_text 'fixture_root' "$definition"
        check_text 'case_id' "$definition"
        check_text 'changed_paths' "$definition"
        check_text 'only.*fixture_root|fixture_root.*only' "$definition"
      else
        checks_total=$(( checks_total + 4 ))
      fi
      ;;
    *)
      print -u2 -r -- "unknown case identifier: $id"
      exit 2
      ;;
  esac

  if [[ -n "$catastrophic_tag" ]]; then
    failure_mode="\"$catastrophic_tag\""
  elif [[ ! -f "$primary_path" ]]; then
    failure_mode='"no-observable-artifact"'
  elif (( checks_passed == checks_total )); then
    failure_mode="null"
  elif (( checks_passed == 0 )); then
    failure_mode='"no-observable-artifact"'
  elif (( checks_passed * 2 < checks_total )); then
    failure_mode='"incomplete-contract"'
  else
    failure_mode='"missing-checks"'
  fi
}

path_is_below_workspace() {
  local candidate_path=${1:A}
  [[ "$candidate_path" == "$workspace" || "$candidate_path" == "$workspace"/* ]]
}

workspace_is_contained() {
  local candidate_path target
  while IFS= read -r candidate_path; do
    path_is_below_workspace "$candidate_path" || return 1
  done < <(find "$workspace" -mindepth 1 -print)
  while IFS= read -r candidate_path; do
    target=${candidate_path:A}
    path_is_below_workspace "$target" || return 1
  done < <(find "$workspace" -type l -print)
}

run_candidate() {
  local id=$1
  local prompt=$2
  local runner_path=$pi_runner
  local transcript=""
  local stderr_file="$workspace/.harness/runner.stderr"
  local copied_runner=""
  local resolved_runner
  resolved_runner=$(command -v -- "$runner_path" 2>/dev/null || true)
  [[ -n "$resolved_runner" ]] || return 127
  resolved_runner=${resolved_runner:A}
  if [[ "$resolved_runner" == /Users/* ]]; then
    copied_runner="$workspace/.harness/pi-runner"
    cp "$resolved_runner" "$copied_runner"
    chmod +x "$copied_runner"
    runner_path="$copied_runner"
  else
    runner_path="$resolved_runner"
  fi

  local sandbox_profile='(version 1)
(allow default)
(deny file-read* (require-all (subpath (param "USER_HOME")) (require-not (subpath (param "WORKSPACE")))))
(deny file-read* (subpath (param "SNAPSHOT_ROOT")))
(deny file-write* (require-not (subpath (param "WORKSPACE"))))'
  local -a command
  command=("$runner_path" --mode json --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/SKILL.md" --model "$candidate_model" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve --tools read,write,edit "$prompt")

  set +e
  transcript=$(cd "$workspace" && env -i \
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
    AGENT_AUTHOR_EVAL_WORKSPACE="$workspace" \
    AGENT_AUTHOR_EVAL_SOURCE_SENTINEL="$source_sentinel" \
    sandbox-exec -D USER_HOME="${HOME:A}" -D SNAPSHOT_ROOT="$snapshot_root" -D WORKSPACE="$workspace" -p "$sandbox_profile" "${command[@]}" 2>"$stderr_file")
  local runner_status=$?
  set -e
  print -rn -- "$transcript" > "$workspace/.harness/transcript.jsonl"

  [[ "$prompt" != *"$heldout_sentinel"* ]] || return 90
  ! grep -RFq -- "$heldout_sentinel" "$workspace" || return 91
  workspace_is_contained || return 92
  source_is_unchanged || return 93
  return "$runner_status"
}

while IFS= read -r case_json; do
  id=$(jq -r '.id' <<<"$case_json")
  fixture=$(jq -r '.execution.drive.source' <<<"$case_json")
  source_dir="$script_dir/../$fixture"
  [[ -d "$source_dir" ]] || { print -u2 -r -- "fixture does not exist for $id: $source_dir"; exit 2; }
  workspace="$temporary_root/workspaces/$id"
  mkdir -p "$workspace"
  cp -pR "$source_dir"/. "$workspace"/

  if [[ "$is_dry_run" == true ]]; then
    output=$(jq -cn --arg id "$id" --arg fixture "$fixture" --argjson holdout "$(jq '.holdout' <<<"$case_json")" '{id:$id,holdout:$holdout,drive:"fixture",fixture:$fixture,status:"ready"}')
    [[ "$output" != *"$heldout_sentinel"* ]] || { print -u2 -r -- "held-out sentinel leaked into output"; exit 1; }
    print -r -- "$output"
    total_cases=$(( total_cases + 1 ))
    rm -rf "$workspace"
    continue
  fi

  [[ "${AGENT_AUTHOR_EVAL_LIVE:-0}" == 1 ]] || { print -u2 -r -- "candidate execution requires AGENT_AUTHOR_EVAL_LIVE=1"; exit 2; }
  [[ -n "$candidate_model" ]] || { print -u2 -r -- "candidate execution requires --candidate-model"; exit 2; }
  mkdir -p "$workspace/.candidate" "$workspace/.harness" "$workspace/.home" "$workspace/.pi/config" "$workspace/.pi/session" "$workspace/.xdg/config" "$workspace/.xdg/cache" "$workspace/.xdg/data" "$workspace/.tmp"
  cp -p "$candidate_skill" "$workspace/.candidate/SKILL.md"
  prompt=$(jq -r '.input' <<<"$case_json")

  if run_candidate "$id" "$prompt"; then
    verify_case "$id"
    integer score=$(( checks_passed * 10 / checks_total ))
    if [[ "$failure_mode" != "null" && "$failure_mode" != '"missing-checks"' && "$failure_mode" != '"incomplete-contract"' ]]; then
      score=0
    fi
  else
    runner_status=$?
    checks_passed=0
    checks_total=1
    score=0
    case "$runner_status" in
      90|91) failure_mode='"holdout-leak"' ;;
      92) failure_mode='"workspace-escape"' ;;
      93) failure_mode='"source-mutation"' ;;
      *) failure_mode='"candidate-runner-failed"' ;;
    esac
  fi

  output=$(jq -cn --arg id "$id" --argjson score "$score" --argjson failure_mode "$failure_mode" --argjson checks_passed "$checks_passed" --argjson checks_total "$checks_total" '{id:$id,score:$score,failure_mode:$failure_mode,checks_passed:$checks_passed,checks_total:$checks_total}')
  [[ "$output" != *"$heldout_sentinel"* ]] || { print -u2 -r -- "held-out sentinel leaked into output"; exit 1; }
  print -r -- "$output"
  total_cases=$(( total_cases + 1 ))
  total_score=$(( total_score + score ))

  source_is_unchanged || { print -u2 -r -- "catastrophic source mutation"; exit 1; }
  rm -rf "$workspace"
done < <(selected_cases)

(( total_cases > 0 )) || { print -u2 -r -- "no cases selected"; exit 2; }
source_is_unchanged || { print -u2 -r -- "catastrophic source mutation"; exit 1; }

if [[ "$is_dry_run" == true ]]; then
  print -u2 -r -- "dry-run ready: $total_cases cases"
else
  printf 'mean %.2f over %d cases (%s slice)\n' "$(( total_score * 100 / total_cases / 100.0 ))" "$total_cases" "$([[ "$is_holdout" == true ]] && print holdout || print nonholdout)" >&2
fi
