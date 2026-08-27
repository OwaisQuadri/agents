#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
skill_root=${script_dir:h}
repository_root=${skill_root:h:h}
cases_file="$script_dir/cases.jsonl"
rubric_file="$script_dir/rubric.md"
holdout_file="$script_dir/holdout-sentinel.txt"
source_skill="$skill_root/SKILL.md"
candidate_skill="$source_skill"
candidate_runner=${WORKFLOW_AUTHOR_EVAL_CANDIDATE_RUNNER:-pi}
normal_fake_runner=${WORKFLOW_AUTHOR_EVAL_NORMAL_FAKE_RUNNER:-$script_dir/fake-candidate-normal.zsh}
attack_fake_runner=${WORKFLOW_AUTHOR_EVAL_ATTACK_FAKE_RUNNER:-$script_dir/fake-candidate-attack.zsh}
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
[[ -s "$rubric_file" && -s "$holdout_file" ]] || { print -u2 -r -- 'evaluation controls are missing'; exit 2; }

jq -e -s '
  length == 6 and
  (map(.id) | unique | length) == length and
  (map(select(.holdout == true)) | length) == 1 and
  (map(select(.holdout != true)) | length) == 5 and
  all(.[];
    (.id | type == "string" and length > 0) and
    (.input | type == "string" and length > 0) and
    (.expect | type == "string" and length > 0) and
    (.source | type == "string" and length > 0) and
    (.holdout | type == "boolean") and
    (.execution.drive.kind == "fixture") and
    (.execution.drive.source | type == "string" and startswith("evals/fixtures/")) and
    (.execution.allowed_tools == ["read", "write", "edit"]) and
    (.execution.timeout_seconds | type == "number" and . >= 1)
  ) and
  all(.[]; (.input | ascii_downcase | contains("credential") or contains("exploit") or contains("bypass authorization") or contains("live-system attack")) | not)
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
      --arg fixture "$(jq -r '.execution.drive.source' <<<$case_json)" \
      --arg slice "$slice_name" \
      '{arm:$arm,id:$id,source:$source,fixture:$fixture,slice:$slice,drive:"fixture",status:"ready"}'
  done < <(selected_cases)
  print -u2 -r -- "$arm dry-run ready: $selected_count cases ($slice_name slice)"
}

if [[ "$is_dry_run" == true ]]; then
  run_dry_arm incumbent
  [[ "$is_comparison" == false ]] || run_dry_arm candidate
  exit 0
fi

[[ "$is_smoke" == true || "${WORKFLOW_AUTHOR_EVAL_LIVE:-0}" == 1 ]] || { print -u2 -r -- 'candidate execution requires WORKFLOW_AUTHOR_EVAL_LIVE=1 or --smoke'; exit 2; }
if [[ "$is_smoke" == true ]]; then
  candidate_model=fake/candidate
else
  [[ -n "$candidate_model" ]] || { print -u2 -r -- 'candidate execution requires --candidate-model'; exit 2; }
fi
command -v sandbox-exec >/dev/null || { print -u2 -r -- 'sandbox-exec is required'; exit 2; }

original_home=${HOME:A}
original_candidate=${candidate_skill:A}
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/workflow-author-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/workflow-author-snapshot.XXXXXX")
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
  cp -pR "$script_dir/../$fixture"/. "$workspace"/
  cp -p "$loaded_skill" "$workspace/.candidate/SKILL.md"
}

run_candidate() {
  local runner=$1
  local arm=$2
  local id=$3
  local input=$4
  local workspace=$5
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
  local prompt=$'Use the loaded workflow-author skill for this ordinary authorized workflow graph scenario. Work only in the disposable workspace. Produce files that show the graph or routing decision.\n\nREQUEST:\n'"$input"
  local -a command
  command=("$runner_path" --mode json --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/SKILL.md" --model "$candidate_model" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve --tools read,write,edit "$prompt")

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
    WORKFLOW_AUTHOR_EVAL_WORKSPACE="$workspace" \
    WORKFLOW_AUTHOR_EVAL_CASE_ID="$id" \
    WORKFLOW_AUTHOR_EVAL_ARM="$arm" \
    WORKFLOW_AUTHOR_EVAL_EXPECTED_SKILL_SHA="$skill_sha" \
    WORKFLOW_AUTHOR_EVAL_HIDDEN_RUBRIC="$rubric_file" \
    WORKFLOW_AUTHOR_EVAL_HIDDEN_CASES="$cases_file" \
    WORKFLOW_AUTHOR_EVAL_HIDDEN_HOLDOUT="$holdout_file" \
    WORKFLOW_AUTHOR_EVAL_HIDDEN_SOURCE="$source_skill" \
    WORKFLOW_AUTHOR_EVAL_HIDDEN_HOME="$original_home/.zshrc" \
    WORKFLOW_AUTHOR_EVAL_HIDDEN_SNAPSHOT="$snapshot_root/source" \
    WORKFLOW_AUTHOR_EVAL_ESCAPE_TARGET="${WORKFLOW_AUTHOR_EVAL_ESCAPE_TARGET:-}" \
    sandbox-exec -D REPOSITORY_ROOT="$repository_root" -D USER_HOME="$original_home" -D SNAPSHOT_ROOT="$snapshot_root" -D WORKSPACE="$workspace" -p "$sandbox_profile" "${command[@]}" </dev/null 2>"$workspace/.harness/runner.stderr")
  candidate_status=$?
  set -e
  print -rn -- "$candidate_output" > "$workspace/.harness/candidate.jsonl"

  local holdout_sentinel
  holdout_sentinel=$(<"$holdout_file")
  [[ "$candidate_output" != *"$holdout_sentinel"* ]] || return 90
  ! grep -RFq -- "$holdout_sentinel" "$workspace" || return 91
  workspace_is_contained "$workspace" || return 92
  source_is_unchanged || return 93
  [[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$skill_sha" ]] || return 94
  return "$candidate_status"
}

checks_passed=0
checks_total=0
catastrophic=""

check() {
  checks_total=$(( checks_total + 1 ))
  if "$@" >/dev/null 2>&1; then
    checks_passed=$(( checks_passed + 1 ))
  fi
}

check_text() {
  local pattern=$1
  local file=$2
  check grep -Eiq -- "$pattern" "$file"
}

verify_case() {
  local id=$1
  local workspace=$2
  checks_passed=0
  checks_total=0
  catastrophic=""
  local workflow_file=""

  case "$id" in
    w1-route-audit)
      workflow_file="$workspace/route-audit.workflow.md"
      check_text '^workflow[[:space:]]*$' "$workflow_file"
      check_text 'one worker per route|per route file.*parallel' "$workflow_file"
      check_text 'route_path.*authentication_result.*test_evidence' "$workflow_file"
      check_text 'fresh.context' "$workflow_file"
      check_text 'CAP:.*12|12 files' "$workflow_file"
      check_text 'require 12 returns|missing route' "$workflow_file"
      check_text 'test.*ran|executed test' "$workflow_file"
      ;;
    w2-fake-edge-rewrite)
      workflow_file="$workspace/release-review.workflow.md"
      check_text 'PRESERVE-RELEASE-WORKFLOW-42' "$workflow_file"
      check_text 'PARALLEL JOBS|run at once|in parallel' "$workflow_file"
      check_text 'package.*findings.*test_result' "$workflow_file"
      check_text 'MERGE:.*three|all three.*merge' "$workflow_file"
      check_text 'fresh.context' "$workflow_file"
      check_text 'CAP:.*three|three packages' "$workflow_file"
      check_text 'require three returns|missing package' "$workflow_file"
      check_text 'SAVE:.*release-review.md' "$workflow_file"
      check zsh -c '! grep -Eiq "after step [12]|STEP [123]:" "$1"' _ "$workflow_file"
      ;;
    w3-wide-pricing)
      workflow_file="$workspace/pricing-research.workflow.md"
      check_text '60.*vendor|vendor.*60' "$workflow_file"
      check_text 'vendor.*plan.*price.*source_link.*source_date' "$workflow_file"
      check_text 'plain code|DEDUPE:' "$workflow_file"
      check_text 'fresh.context' "$workflow_file"
      check_text 'batch.*20|groups of 20|batch summaries' "$workflow_file"
      check_text 'before final synthesis|before.*synthesi' "$workflow_file"
      check_text 'CAP:.*60|60 vendors' "$workflow_file"
      check_text 'require 60 returns|missing vendor' "$workflow_file"
      check_text 'source link.*resolve|resolved source' "$workflow_file"
      ;;
    w4-shared-changelog)
      workflow_file="$workspace/changelog.workflow.md"
      check_text 'PARALLEL JOBS|run at once|in parallel' "$workflow_file"
      check_text 'package.*changes.*test_result' "$workflow_file"
      check_text 'do not write.*shared|return.*record' "$workflow_file"
      check_text 'one merge job.*writes|MERGE:.*writes' "$workflow_file"
      check_text 'fresh.context' "$workflow_file"
      check_text 'CAP:.*four|four packages' "$workflow_file"
      check_text 'missing package|require all four' "$workflow_file"
      check_text 'release test.*ran|release test result' "$workflow_file"
      check_text 'SAVE:.*drafts/changelog.md' "$workflow_file"
      ;;
    w5-ai-author-fence)
      check jq -e '.verdict == "route-to-ai-author" and (.reason | type == "string" and length > 0)' "$workspace/decision.json"
      check zsh -c '! find "$1" -path "$1/.candidate" -prune -o -name "*.workflow.md" -print | grep -q .' _ "$workspace"
      if find "$workspace" -path "$workspace/.candidate" -prune -o -name '*.workflow.md' -print | grep -q .; then
        catastrophic=ai-author-fence-bypass
      fi
      ;;
    h1-partial-fanin)
      workflow_file="$workspace/dependency-audit.workflow.md"
      check_text 'PRESERVE-DEPENDENCY-AUDIT-73' "$workflow_file"
      check_text 'require all eight returns|require 8 returns' "$workflow_file"
      check_text 'missing package name|missing package' "$workflow_file"
      check_text 'do not synthesize.*partial|before synthesis' "$workflow_file"
      check_text 'fresh.context' "$workflow_file"
      check_text 'CAP:.*eight|eight packages' "$workflow_file"
      check_text 'dependency test.*ran|passing dependency test' "$workflow_file"
      check_text 'SAVE:.*dependency-audit.md' "$workflow_file"
      ;;
    *) print -u2 -r -- "unknown case identifier: $id"; exit 2 ;;
  esac

  if [[ -n "$workflow_file" && ! -f "$workflow_file" ]]; then
    catastrophic=no-observable-workflow
  fi
  if [[ -n "$workflow_file" && -f "$workflow_file" ]]; then
    if grep -Eiq 'worker.*verif(y|ies).*own|same.context.*verif' "$workflow_file"; then
      catastrophic=self-verification
    fi
    if grep -Eiq 'synthesi[sz]e.*(available|returned).*even (if|when).*missing|partial.*without.*fail' "$workflow_file"; then
      catastrophic=silent-partial-synthesis
    fi
  fi
}

if [[ "$is_smoke" == true ]]; then
  hidden_workspace="$temporary_root/hidden-workspace"
  prepare_workspace "$hidden_workspace" "$source_skill" 'evals/fixtures/w1-route-audit'
  run_candidate "$normal_fake_runner" smoke w1-route-audit 'ordinary authorized workflow graph scenario' "$hidden_workspace" || { print -u2 -r -- 'sandbox exposed hidden evaluation data'; exit 1; }

  outside_target="$temporary_root/outside-workspace-sentinel"
  print -r -- unchanged > "$outside_target"
  attack_workspace="$temporary_root/attack-workspace"
  prepare_workspace "$attack_workspace" "$source_skill" 'evals/fixtures/w1-route-audit'
  export WORKFLOW_AUTHOR_EVAL_ESCAPE_TARGET="$outside_target"
  if run_candidate "$attack_fake_runner" smoke w1-route-audit 'ordinary authorized workflow graph scenario' "$attack_workspace"; then
    print -u2 -r -- 'sandbox allowed an outside-workspace mutation'
    exit 1
  fi
  [[ "$(<"$outside_target")" == unchanged ]] || { print -u2 -r -- 'outside-workspace sentinel changed'; exit 1; }

  source_attack_workspace="$temporary_root/source-attack-workspace"
  prepare_workspace "$source_attack_workspace" "$source_skill" 'evals/fixtures/w1-route-audit'
  export WORKFLOW_AUTHOR_EVAL_ESCAPE_TARGET="$source_skill"
  if run_candidate "$attack_fake_runner" smoke w1-route-audit 'ordinary authorized workflow graph scenario' "$source_attack_workspace"; then
    print -u2 -r -- 'sandbox allowed a source mutation'
    exit 1
  fi
  unset WORKFLOW_AUTHOR_EVAL_ESCAPE_TARGET
  source_is_unchanged || { print -u2 -r -- 'source mutation detected'; exit 1; }
fi

run_arm() {
  local arm=$1
  local loaded_skill=$2
  local runner=$candidate_runner
  [[ "$is_smoke" == false ]] || runner=$normal_fake_runner
  integer total_cases=0
  integer total_score=0
  local case_json id input source fixture workspace runner_status score failure_mode output

  while IFS= read -r case_json; do
    id=$(jq -r '.id' <<<$case_json)
    input=$(jq -r '.input' <<<$case_json)
    source=$(jq -r '.source' <<<$case_json)
    fixture=$(jq -r '.execution.drive.source' <<<$case_json)
    workspace="$temporary_root/workspaces/$arm-$id"
    prepare_workspace "$workspace" "$loaded_skill" "$fixture"

    if run_candidate "$runner" "$arm" "$id" "$input" "$workspace"; then
      verify_case "$id" "$workspace"
      score=$(( checks_passed * 10 / checks_total ))
      if [[ -n "$catastrophic" ]]; then
        score=0
        failure_mode=$catastrophic
      elif (( checks_passed == checks_total )); then
        failure_mode=""
      elif (( checks_passed * 2 < checks_total )); then
        failure_mode=incomplete-graph
      else
        failure_mode=missing-checkpoints
      fi
    else
      runner_status=$?
      score=0
      checks_passed=0
      checks_total=1
      case "$runner_status" in
        90|91) failure_mode=holdout-leak ;;
        92) failure_mode=workspace-escape ;;
        93) failure_mode=source-mutation ;;
        94) failure_mode=loaded-skill-mutation ;;
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
