#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
cases_file="$script_dir/cases.jsonl"
source_skill="$script_dir/../SKILL.md"
candidate_skill="$source_skill"
candidate_model=""
pi_runner=${AI_AUTHOR_EVAL_PI:-pi}
skill_eval=${AI_AUTHOR_EVAL_SKILL_EVAL:-skill-eval}
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
      print -r -- "usage: ./run.sh [--holdout] [--dry-run] [--smoke] [--candidate-skill path] [--candidate-model provider/model]"
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

if [[ "$is_smoke" == true ]]; then
  pi_runner="$script_dir/fake-pi-normal.sh"
  skill_eval="$script_dir/fake-skill-eval.sh"
  candidate_model=fake/candidate
fi

[[ -f "$candidate_skill" ]] || { print -u2 -r -- "candidate skill does not exist: $candidate_skill"; exit 2; }
jq -e -s '
  length == 15 and
  (map(.id) | unique | length == 15) and
  all(.[]; (.input | type == "string" and length > 0) and
    (.expect | type == "string" and length > 0) and
    (.source | type == "string" and length > 0) and
    (.holdout | type == "boolean") and
    (.execution.drive.kind == "response" or .execution.drive.kind == "fixture" or .execution.drive.kind == "existing_harness")) and
  (map(select(.holdout == true)) | length == 5) and
  (map(select(.holdout != true)) | length == 10)
' "$cases_file" >/dev/null

source_sentinel="$script_dir/source-sentinel.txt"
[[ -s "$source_sentinel" ]] || { print -u2 -r -- "source sentinel is missing"; exit 2; }
source_sentinel_hash=$(shasum -a 256 "$source_sentinel" | cut -d ' ' -f 1)

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ai-author-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/ai-author-snapshot.XXXXXX")
snapshot_root=${snapshot_root:A}
trap 'rm -rf "$temporary_root" "$snapshot_root"' EXIT INT TERM
cp -p "$source_skill" "$snapshot_root/SKILL.md"
cp -pR "$script_dir" "$snapshot_root/evals"

source_is_unchanged() {
  cmp -s "$source_skill" "$snapshot_root/SKILL.md" &&
    diff -qr "$script_dir" "$snapshot_root/evals" >/dev/null &&
    [[ "$(shasum -a 256 "$source_sentinel" | cut -d ' ' -f 1)" == "$source_sentinel_hash" ]]
}

selected_cases() {
  if [[ "$is_holdout" == true ]]; then
    jq -c 'select(.holdout == true)' "$cases_file"
  else
    jq -c 'select(.holdout != true)' "$cases_file"
  fi
}

smoke_dir="$temporary_root/submit-vote"
mkdir -p "$smoke_dir/votes"
print -r -- '{"ts":"x","artifact":"fake","grade":"9","vote":"SENTINEL-PRIOR-VOTE"}' > "$smoke_dir/votes/votes.jsonl"
submit_out=$(print -r -- "second vote" | python3 "$script_dir/../scripts/submit_vote.py" --artifact "$smoke_dir" --grade 7)
[[ "$submit_out" == "vote recorded" ]] || { print -u2 -r -- "smoke: submit_vote did not confirm"; exit 1; }
[[ "$submit_out" != *SENTINEL-PRIOR-VOTE* ]] || { print -u2 -r -- "smoke: submit_vote leaked a prior vote"; exit 1; }
[[ "$(wc -l < "$smoke_dir/votes/votes.jsonl")" -eq 2 ]] || { print -u2 -r -- "smoke: votes are not append-only"; exit 1; }
head -1 "$smoke_dir/votes/votes.jsonl" | grep -q SENTINEL-PRIOR-VOTE || { print -u2 -r -- "smoke: prior vote changed"; exit 1; }
if print -rn -- "" | python3 "$script_dir/../scripts/submit_vote.py" --artifact "$smoke_dir" --grade 7 >/dev/null 2>&1; then
  print -u2 -r -- "smoke: empty vote was accepted"
  exit 1
fi
print -u2 -r -- "smoke: submit_vote checks passed"

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

check_file() {
  check_command test -f "$workspace/$1"
}

check_pattern() {
  local pattern=$1
  local file_path=$2
  check_command grep -Eiq -- "$pattern" "$workspace/$file_path"
}

check_contract() {
  local root=$1
  local definition=$2
  check_file "$definition"
  check_file "$root/evals/cases.jsonl"
  check_file "$root/evals/rubric.md"
  check_file "$root/evals/run.sh"
  check_file "$root/logs/usage.jsonl"
  check_file "$root/votes/votes.jsonl"
  if [[ -f "$workspace/$definition" ]]; then
    check_command /bin/zsh -c '[[ "$(grep -E "^## " "$1" | tail -1)" == "## logging" ]]' _ "$workspace/$definition"
  else
    checks_total=$(( checks_total + 1 ))
  fi
}

verify_fixture() {
  local id=$1
  checks_passed=0
  checks_total=0
  failure_mode=null
  case "$id" in
    a4)
      check_file research-results.json
      check_command jq -e '.task == "mobile-testing-research" and (.tools | length == 3) and (.summary | type == "string" and length > 0)' "$workspace/research-results.json"
      check_command /bin/zsh -c '! find "$1" -path "*/evals/cases.jsonl" -o -path "*/logs/usage.jsonl" -o -path "*/votes/votes.jsonl" | grep -q .' _ "$workspace"
      ;;
    b1)
      check_contract skills/create-pr skills/create-pr/SKILL.md
      check_pattern 'rule 3|linear recipe' skills/create-pr/SKILL.md
      check_command /bin/zsh -c '! find "$1/agents" "$1/workflows" -type f 2>/dev/null | grep -q .' _ "$workspace"
      ;;
    b2)
      check_contract agents/diff-reviewer agents/diff-reviewer/diff-reviewer.md
      check_pattern 'fresh context|fresh-context' agents/diff-reviewer/diff-reviewer.md
      check_pattern 'read-only|tools:[[:space:]]*read' agents/diff-reviewer/diff-reviewer.md
      check_pattern 'ranked findings|severity' agents/diff-reviewer/diff-reviewer.md
      ;;
    b3)
      check_contract workflows/research-sweep workflows/research-sweep/research-sweep.workflow.js
      check_pattern 'fan.?out|parallel|five' workflows/research-sweep/research-sweep.workflow.js
      check_pattern 'skeptic|judge' workflows/research-sweep/research-sweep.workflow.js
      check_pattern 'synthesi' workflows/research-sweep/research-sweep.workflow.js
      ;;
    c1)
      check_file RESULT.md
      check_command /bin/zsh -c '[[ "$(wc -l < "$1")" -eq 2 ]] && [[ "$(head -1 "$1")" == nonholdout ]] && [[ "$(tail -1 "$1")" == holdout ]]' _ "$workspace/skills/create-pr/.eval-runs"
      check_pattern 'per-case|n1.*10|score.*10' RESULT.md
      check_pattern 'non-holdout.*1.*1|1.*1.*non-holdout' RESULT.md
      check_pattern 'holdout.*pass|holdout.*10' RESULT.md
      check_pattern 'votes/votes.jsonl|vote file' RESULT.md
      check_pattern 'logging' RESULT.md
      ;;
    *)
      print -u2 -r -- "unknown fixture case: $id"
      exit 2
      ;;
  esac
  if (( checks_passed == checks_total )); then
    failure_mode=null
  elif (( checks_passed == 0 )); then
    failure_mode='"no-observable-artifact"'
  else
    failure_mode='"missing-checks"'
  fi
}

path_is_below_workspace() {
  local path=${1:A}
  [[ "$path" == "$workspace" || "$path" == "$workspace"/* ]]
}

workspace_is_contained() {
  local candidate_path target
  while IFS= read -r candidate_path; do
    target=${candidate_path:A}
    path_is_below_workspace "$target" || return 1
  done < <(find "$workspace" -type l -print)
}

run_candidate() {
  local case_json=$1
  local loaded_skill=$2
  local prompt=$3
  local runner_path=$pi_runner
  local stderr_file="$workspace/.harness/runner.stderr"
  local resolved_runner copied_runner=""
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
  local tools
  tools=$(jq -r '.execution.allowed_tools | join(",")' <<<"$case_json")
  local -a command
  command=("$runner_path" --mode json --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/SKILL.md" --model "$candidate_model" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve)
  if [[ -n "$tools" ]]; then
    command+=(--tools "$tools")
  else
    command+=(--tools "")
  fi
  command+=("$prompt")

  set +e
  (cd "$workspace" && env -i \
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
    AI_AUTHOR_EVAL_WORKSPACE="$workspace" \
    AI_AUTHOR_EVAL_SOURCE_SENTINEL="$source_sentinel" \
    sandbox-exec -D USER_HOME="${HOME:A}" -D SNAPSHOT_ROOT="$snapshot_root" -D WORKSPACE="$workspace" -p "$sandbox_profile" "${command[@]}" > "$workspace/.harness/transcript.jsonl" 2> "$stderr_file")
  local runner_status=$?
  set -e

  local sentinel
  sentinel=$(jq -r '.sentinel // empty' <<<"$case_json")
  [[ -z "$sentinel" || "$prompt" != *"$sentinel"* ]] || return 90
  [[ -z "$sentinel" ]] || ! grep -RFq -- "$sentinel" "$workspace" || return 91
  cmp -s "$workspace/.candidate/SKILL.md" "$loaded_skill" || return 94
  workspace_is_contained || return 92
  source_is_unchanged || return 93
  return "$runner_status"
}

grade_response() {
  local case_json=$1
  local transcript="$workspace/.harness/transcript.jsonl"
  local prompt_file="$workspace/.harness/judge-prompt.txt"
  {
    print -r -- 'Grade the actual response that a candidate produced while executing the loaded skill.'
    print -r -- 'Reply with only a JSON object: {"score":<integer 0-10>,"failure_mode":<string or null>}.'
    print -r -- 'Use this rubric:'
    cat "$script_dir/rubric.md"
    print -r -- 'Case expectation:'
    jq -r '.expect' <<<"$case_json"
    print -r -- 'Actual produced response event stream:'
    cat "$transcript"
  } > "$prompt_file"
  local judge_out
  judge_out=$("$skill_eval" judge --prompt "$(<"$prompt_file")") || return $?
  local verdict
  verdict=$(print -r -- "$judge_out" | grep -Eo '\{.*\}' | tail -1)
  jq -e '(.score | type == "number") and .score >= 0 and .score <= 10 and ((.failure_mode == null) or (.failure_mode | type == "string"))' <<<"$verdict" >/dev/null || return 65
  print -r -- "$verdict"
}

run_arm() {
  local arm=$1
  local loaded_skill=$2
  integer total_cases=0
  integer total_score=0
  local case_json id drive fixture source_dir prompt verdict score

  while IFS= read -r case_json; do
    id=$(jq -r '.id' <<<"$case_json")
    drive=$(jq -r '.execution.drive.kind' <<<"$case_json")
    workspace="$temporary_root/workspaces/$arm-$id"
    mkdir -p "$workspace"
    if [[ "$drive" == fixture ]]; then
      fixture=$(jq -r '.execution.drive.source' <<<"$case_json")
      source_dir="$script_dir/../$fixture"
      [[ -d "$source_dir" ]] || { print -u2 -r -- "fixture does not exist for $id: $fixture"; exit 2; }
      cp -pR "$source_dir"/. "$workspace"/
      prompt="$(jq -r '.input' <<<"$case_json")\n\nRead REQUEST.md. Work only in this disposable workspace."
    else
      prompt=$(jq -r '.input' <<<"$case_json")
    fi

    if [[ "$is_dry_run" == true ]]; then
      print -r -- "$(jq -cn --arg arm "$arm" --arg id "$id" --arg drive "$drive" --argjson holdout "$(jq '.holdout' <<<"$case_json")" '{arm:$arm,id:$id,holdout:$holdout,drive:$drive,status:"ready"}')"
      total_cases=$(( total_cases + 1 ))
      rm -rf "$workspace"
      continue
    fi

    [[ "${AI_AUTHOR_EVAL_LIVE:-0}" == 1 || "$is_smoke" == true ]] || { print -u2 -r -- "candidate execution requires AI_AUTHOR_EVAL_LIVE=1"; exit 2; }
    [[ -n "$candidate_model" ]] || { print -u2 -r -- "candidate execution requires --candidate-model"; exit 2; }
    mkdir -p "$workspace/.candidate" "$workspace/.harness" "$workspace/.home" "$workspace/.pi/config" "$workspace/.pi/session" "$workspace/.xdg/config" "$workspace/.xdg/cache" "$workspace/.xdg/data" "$workspace/.tmp"
    cp -p "$loaded_skill" "$workspace/.candidate/SKILL.md"

    if run_candidate "$case_json" "$loaded_skill" "$prompt"; then
      if [[ "$drive" == fixture ]]; then
        verify_fixture "$id"
        score=$(( checks_passed * 10 / checks_total ))
        verdict=$(jq -cn --argjson score "$score" --argjson failure_mode "$failure_mode" --argjson checks_passed "$checks_passed" --argjson checks_total "$checks_total" '{score:$score,failure_mode:$failure_mode,checks_passed:$checks_passed,checks_total:$checks_total}')
      else
        verdict=$(grade_response "$case_json") || { print -u2 -r -- "judge failed for case $id"; exit 1; }
        score=$(jq -r '.score' <<<"$verdict")
      fi
    else
      local runner_status=$?
      print -u2 -r -- "candidate runner failed for $arm/$id with status $runner_status"
      if [[ -s "$workspace/.harness/runner.stderr" ]]; then
        command cat "$workspace/.harness/runner.stderr" >&2
      fi
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

    output=$(jq -cn --arg arm "$arm" --arg id "$id" --argjson verdict "$verdict" '{arm:$arm,id:$id} + $verdict')
    print -r -- "$output"
    total_cases=$(( total_cases + 1 ))
    total_score=$(( total_score + score ))
    source_is_unchanged || { print -u2 -r -- "catastrophic source mutation"; exit 1; }
    rm -rf "$workspace"
  done < <(selected_cases)

  (( total_cases > 0 )) || { print -u2 -r -- "no cases selected"; exit 2; }
  if [[ "$is_dry_run" == true ]]; then
    print -u2 -r -- "$arm dry-run ready: $total_cases cases"
  else
    local -F 2 mean_score
    mean_score=$(( 1.0 * total_score / total_cases ))
    printf '%s mean %.2f over %d cases (%s slice)\n' "$arm" "$mean_score" "$total_cases" "$([[ "$is_holdout" == true ]] && print holdout || print nonholdout)" >&2
  fi
}

run_arm incumbent "$source_skill"
if [[ "$is_comparison" == true ]]; then
  run_arm candidate "$candidate_skill"
fi
source_is_unchanged || { print -u2 -r -- "catastrophic source mutation"; exit 1; }
