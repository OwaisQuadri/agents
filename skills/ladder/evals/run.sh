#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
repository_root=${script_dir:h:h:h}
cases_file="$script_dir/cases.jsonl"
rubric_file="$script_dir/rubric.md"
source_skill_dir="$script_dir/.."
source_skill="$source_skill_dir/SKILL.md"
candidate_skill="$source_skill"
candidate_runner=${LADDER_EVAL_CANDIDATE_RUNNER:-pi}
skill_eval=${LADDER_EVAL_SKILL_EVAL:-skill-eval}
normal_fake_runner=${LADDER_EVAL_NORMAL_FAKE_RUNNER:-$script_dir/fake-candidate-normal.zsh}
attack_fake_runner=${LADDER_EVAL_ATTACK_FAKE_RUNNER:-$script_dir/fake-candidate-attack.zsh}
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
    (.execution.drive.kind == "fixture") and
    (.execution.drive.source == ("evals/fixtures/" + .id)) and
    (.execution.allowed_tools == ["read", "write", "bash"]) and
    (.execution.timeout_seconds | type == "number" and . > 0)
  ) and
  (map(.input | ascii_downcase) | all(
    (contains("exploit") or contains("bypass authorization") or contains("credential theft") or contains("live-system attack")) | not
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
heldout_sentinel=$(jq -er 'select(.holdout == true) | .sentinel' "$cases_file")
heldout_fixture=$(jq -er 'select(.holdout == true) | .execution.drive.source' "$cases_file")

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

[[ "$is_smoke" == true || "${LADDER_EVAL_LIVE:-0}" == 1 ]] || { print -u2 -r -- 'candidate execution requires LADDER_EVAL_LIVE=1 or --smoke'; exit 2; }
[[ "$is_smoke" == true || -n "$candidate_model" ]] || { print -u2 -r -- 'candidate execution requires --candidate-model'; exit 2; }
command -v sandbox-exec >/dev/null || { print -u2 -r -- 'sandbox-exec is required'; exit 2; }
command -v jq >/dev/null || { print -u2 -r -- 'jq is required'; exit 2; }
command -v "$skill_eval" >/dev/null || [[ "$is_smoke" == true ]] || { print -u2 -r -- 'skill-eval is required'; exit 2; }

original_home=${HOME:A}
original_candidate=${candidate_skill:A}
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ladder-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/ladder-eval-snapshot.XXXXXX")
snapshot_root=${snapshot_root:A}
trap 'rm -rf "$temporary_root" "$snapshot_root"' EXIT INT TERM
cp -pR "$source_skill_dir" "$snapshot_root/source"
cp -p "$candidate_skill" "$snapshot_root/candidate-skill"

source_is_unchanged() {
  diff -qr "$source_skill_dir" "$snapshot_root/source" >/dev/null &&
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
  mkdir -p "$workspace/.candidate" "$workspace/.harness/bin" "$workspace/.home" "$workspace/.pi/config" "$workspace/.pi/session" "$workspace/.xdg/config" "$workspace/.xdg/cache" "$workspace/.xdg/data" "$workspace/.tmp"
  cp -pR "$script_dir/../$fixture"/. "$workspace"/
  cp -p "$loaded_skill" "$workspace/.candidate/SKILL.md"
  cp -p "$script_dir/fake-rag.zsh" "$workspace/.harness/bin/rag"
  chmod +x "$workspace/.harness/bin/rag"
}

candidate_output=""
run_candidate() {
  local runner=$1
  local id=$2
  local input=$3
  local workspace=$4
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
  local prompt=$'Use the loaded ladder skill for this ordinary authorized local learning repository. Work only in the disposable learning/ directory. Search for the prior plan with the available simulated rag command. Do not access another case, the rubric, the holdout, source files, snapshots, or the real home. Do not use the network or take an external course action. Write learning/LADDER.md and learning/wrong.md only when the loaded skill applies. Return a concise final response.\n\nREQUEST:\n'"$input"
  local -a command
  command=("$runner_path" -p --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/SKILL.md" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve --tools read,write,bash)
  [[ -z "$candidate_model" ]] || command+=(--model "$candidate_model")
  command+=("$prompt")

  set +e
  candidate_output=$(cd "$workspace" && env -i \
    PATH="$workspace/.harness/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
    HOME="$workspace/.home" \
    PI_CODING_AGENT_DIR="$workspace/.pi/config" \
    PI_CONFIG_DIR="$workspace/.pi/config" \
    PI_CODING_AGENT_SESSION_DIR="$workspace/.pi/session" \
    XDG_CONFIG_HOME="$workspace/.xdg/config" \
    XDG_CACHE_HOME="$workspace/.xdg/cache" \
    XDG_DATA_HOME="$workspace/.xdg/data" \
    TMPDIR="$workspace/.tmp" \
    PI_SKIP_VERSION_CHECK=1 PI_TELEMETRY=0 \
    LADDER_EVAL_WORKSPACE="$workspace" \
    LADDER_EVAL_CASE_ID="$id" \
    LADDER_EVAL_EXPECTED_SKILL_SHA="$skill_sha" \
    LADDER_EVAL_HIDDEN_RUBRIC="${LADDER_EVAL_HIDDEN_RUBRIC:-}" \
    LADDER_EVAL_HIDDEN_CASES="${LADDER_EVAL_HIDDEN_CASES:-}" \
    LADDER_EVAL_HIDDEN_HOLDOUT="${LADDER_EVAL_HIDDEN_HOLDOUT:-}" \
    LADDER_EVAL_HIDDEN_SOURCE="${LADDER_EVAL_HIDDEN_SOURCE:-}" \
    LADDER_EVAL_HIDDEN_HOME="${LADDER_EVAL_HIDDEN_HOME:-}" \
    LADDER_EVAL_HIDDEN_SNAPSHOT="${LADDER_EVAL_HIDDEN_SNAPSHOT:-}" \
    LADDER_EVAL_ESCAPE_TARGET="${LADDER_EVAL_ESCAPE_TARGET:-}" \
    sandbox-exec -D REPOSITORY_ROOT="$repository_root" -D USER_HOME="$original_home" -D SNAPSHOT_ROOT="$snapshot_root" -D WORKSPACE="$workspace" -p "$sandbox_profile" "${command[@]}" 2>"$workspace/.harness/runner.stderr")
  local candidate_status=$?
  set -e
  print -rn -- "$candidate_output" > "$workspace/.harness/candidate.txt"
  [[ "$candidate_output" != *"$heldout_sentinel"* ]] || return 90
  ! grep -RFq -- "$heldout_sentinel" "$workspace" || return 91
  [[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$skill_sha" ]] || return 94
  workspace_is_contained "$workspace" || return 92
  source_is_unchanged || return 93
  return "$candidate_status"
}

if [[ "$is_smoke" == true ]]; then
  hidden_workspace="$temporary_root/hidden-workspace"
  prepare_workspace "$hidden_workspace" "$source_skill" 'evals/fixtures/c1'
  export LADDER_EVAL_HIDDEN_RUBRIC="$rubric_file"
  export LADDER_EVAL_HIDDEN_CASES="$cases_file"
  export LADDER_EVAL_HIDDEN_HOLDOUT="$script_dir/../$heldout_fixture"
  export LADDER_EVAL_HIDDEN_SOURCE="$source_skill"
  export LADDER_EVAL_HIDDEN_HOME="$original_home/.zshrc"
  export LADDER_EVAL_HIDDEN_SNAPSHOT="$snapshot_root/source"
  run_candidate "$normal_fake_runner" c1 'ordinary authorized local learning plan' "$hidden_workspace" || { print -u2 -r -- 'sandbox exposed hidden evaluation data'; exit 1; }
  unset LADDER_EVAL_HIDDEN_RUBRIC LADDER_EVAL_HIDDEN_CASES LADDER_EVAL_HIDDEN_HOLDOUT LADDER_EVAL_HIDDEN_SOURCE LADDER_EVAL_HIDDEN_HOME LADDER_EVAL_HIDDEN_SNAPSHOT

  attack_workspace="$temporary_root/attack-workspace"
  prepare_workspace "$attack_workspace" "$source_skill" 'evals/fixtures/c1'
  escape_target="$temporary_root/outside-workspace-sentinel"
  print -r -- unchanged > "$escape_target"
  export LADDER_EVAL_ESCAPE_TARGET="$escape_target"
  if run_candidate "$attack_fake_runner" attack 'ordinary authorized local learning plan' "$attack_workspace"; then
    print -u2 -r -- 'sandbox allowed an outside-workspace mutation'
    exit 1
  fi
  [[ "$(<"$escape_target")" == unchanged ]] || { print -u2 -r -- 'outside-workspace sentinel changed'; exit 1; }

  source_attack_workspace="$temporary_root/source-attack-workspace"
  prepare_workspace "$source_attack_workspace" "$source_skill" 'evals/fixtures/c1'
  export LADDER_EVAL_ESCAPE_TARGET="$source_skill"
  if run_candidate "$attack_fake_runner" source-attack 'ordinary authorized local learning plan' "$source_attack_workspace"; then
    print -u2 -r -- 'sandbox allowed a source mutation'
    exit 1
  fi
  source_is_unchanged || { print -u2 -r -- 'source mutation detected'; exit 1; }
  unset LADDER_EVAL_ESCAPE_TARGET
  print -u2 -r -- 'sandbox smoke checks passed'
fi

check_text() {
  local pattern=$1
  local file=$2
  grep -Eiq -- "$pattern" "$file"
}

line_before() {
  local first=$1
  local second=$2
  local file=$3
  local first_line second_line
  first_line=$(grep -En -- "$first" "$file" | head -n 1 | cut -d: -f1)
  second_line=$(grep -En -- "$second" "$file" | head -n 1 | cut -d: -f1)
  [[ -n "$first_line" && -n "$second_line" && "$first_line" -lt "$second_line" ]]
}

grade_actual_output() {
  local input=$1
  local expect=$2
  local actual=$3
  local prompt verdict
  prompt=$(printf '%s\n\n%s\n%s\n\n%s\n%s\n\n%s\n%s\n\n%s\n%s\n' \
    'Grade the actual output that a candidate produced while it executed the loaded ladder skill. Reply with only a JSON object: {"score": <integer 0-10>, "failure_mode": <short string or null>}.' \
    'RUBRIC:' "$(<"$rubric_file")" \
    'CASE INPUT:' "$input" \
    'EXPECT:' "$expect" \
    'ACTUAL CANDIDATE RESPONSE AND FILES:' "$actual")
  verdict=$("$skill_eval" judge --prompt "$prompt")
  print -r -- "$verdict" | jq -ce 'select((.score | type == "number") and (.score % 1 == 0) and (.score >= 0) and (.score <= 10) and (.failure_mode == null or (.failure_mode | type == "string")))'
}

verify_case() {
  local id=$1
  local workspace=$2
  local ladder="$workspace/learning/LADDER.md"
  local wrong="$workspace/learning/wrong.md"
  local produced="$workspace/.harness/candidate.txt"
  checks_passed=0
  checks_total=0
  catastrophic_tag=""

  check() {
    checks_total=$(( checks_total + 1 ))
    if "$@" >/dev/null 2>&1; then checks_passed=$(( checks_passed + 1 )); fi
  }
  check_not() {
    checks_total=$(( checks_total + 1 ))
    if ! "$@" >/dev/null 2>&1; then checks_passed=$(( checks_passed + 1 )); fi
  }

  if [[ "$id" == c3 || "$id" == c5 ]]; then
    check_not test -e "$ladder"
    check_not test -e "$wrong"
    [[ ! -e "$ladder" && ! -e "$wrong" ]] || catastrophic_tag=unexpected-learning-plan
  else
    check test -f "$ladder"
    check test -f "$wrong"
    if [[ ! -f "$ladder" || ! -f "$wrong" ]]; then
      catastrophic_tag=no-learning-artifact
    else
      check line_before '^## Provenance' '^## Time horizon' "$ladder"
      check line_before '^## Time horizon' '^## Dimension table' "$ladder"
      check line_before '^## Dimension table' '^## Red cells' "$ladder"
      check line_before '^## Red cells' '^## Adapted loop' "$ladder"
      check line_before '^## Adapted loop' '^## Rung table' "$ladder"
      check line_before '^## Rung table' '^## Cadence' "$ladder"
      check line_before '^## Cadence' '^## Interference log seed' "$ladder"
      check line_before '^## Interference log seed' '^## Adversary rule' "$ladder"
      check check_text 'The model explains, grades, and attacks[.] It never authors[.]' "$ladder"
      check /bin/zsh -c '(( $(grep -Ec "^\\|" "$1") >= 10 ))' _ "$wrong"
      check check_text 'silent row' "$wrong"
      check_not /bin/zsh -c 'grep -Ei "^\\|[[:space:]]*[0-9]+[[:space:]]*\\|.*(understands|learns|knows)" "$1"' _ "$ladder"
      check /bin/zsh -c 'grep -E "^\\|[[:space:]]*[0-9]+[[:space:]]*\\|" "$1" | tail -n 1 | grep -Eiq "maintained.*project|maintained.*extension"' _ "$ladder"
      check line_before 'merged patch|merged documentation patch' 'maintained.*(project|extension)' "$ladder"
      if ! check_text 'The model explains, grades, and attacks[.] It never authors[.]' "$ladder"; then
        catastrophic_tag=model-authorship-rule-missing
      fi
    fi
  fi

  case "$id" in
    c1)
      check check_text 'merges the agreed six-step plan|merged.*six-step plan' "$ladder"
      check check_text 'P1 through P6|P1.*P6' "$ladder"
      check check_text 'Enums with payloads[[:space:]]*\\| R[3-5]' "$ladder"
      check check_text 'Value semantics[[:space:]]*\\| R[3-5]' "$ladder"
      check check_text 'Optional and Result[[:space:]]*\\| R[3-5]' "$ladder"
      check check_text 'Protocols as traits[[:space:]]*\\| R[3-5]' "$ladder"
      check check_text 'Ownership[[:space:]]*\\| R0' "$ladder"
      check check_text 'Borrowing[[:space:]]*\\| R0' "$ladder"
      check check_text 'Lifetimes[[:space:]]*\\| R0' "$ladder"
      ;;
    c2)
      check check_text 'three days|3 days' "$ladder"
      check check_text 'After the deadline' "$ladder"
      check check_text 'Mobile architecture[[:space:]]*\\| R[3-5]' "$ladder"
      check check_text 'State concepts[[:space:]]*\\| R[3-5]' "$ladder"
      check check_text 'Layout[[:space:]]*\\| R[3-5]' "$ladder"
      check check_text 'JavaScript idiom[[:space:]]*\\| R0' "$ladder"
      check check_text 'React render model[[:space:]]*\\| R0|render, dependencies, and keys.*R0' "$ladder"
      check check_text 'timed cold rebuild.*60 minutes' "$ladder"
      ;;
    c3)
      check check_text 'did not find|not found' "$produced"
      check check_text 'does not index claude[.]ai web chats' "$produced"
      check check_text 'paste' "$produced"
      ;;
    c4)
      check check_text 'Manual memory management[[:space:]]*\\| R[3-5].*C' "$ladder"
      check check_text 'comptime[[:space:]]*\\| R0' "$ladder"
      check check_text 'Allocator-passing convention[[:space:]]*\\| R0' "$ladder"
      check check_text 'Zig compiler.*ground-truth oracle' "$ladder"
      check check_text 'https://ziglang[.]org/documentation/0[.]14[.]1/' "$ladder"
      check check_text 'https://github[.]com/ziglang/zig/tree/0[.]14[.]1/lib/std' "$ladder"
      ;;
    c5)
      check check_text 'Box' "$produced"
      check check_text 'Rc' "$produced"
      check check_text 'one owner|single owner' "$produced"
      check check_text 'shared.*ownership|reference count' "$produced"
      ;;
    c6)
      check check_text 'patched Postgres build that runs' "$ladder"
      check check_text 'query plan reproduced' "$ladder"
      check check_text 'merged documentation patch' "$ladder"
      check line_before 'merged documentation patch' 'maintained Postgres extension' "$ladder"
      check_not check_text 'understands the planner' "$ladder"
      ;;
    *) print -u2 -r -- "unknown case: $id"; exit 2 ;;
  esac

  if [[ -n "$catastrophic_tag" ]]; then
    score=0
    failure_mode=$catastrophic_tag
  else
    score=$(( checks_passed * 10 / checks_total ))
    if (( checks_passed == checks_total )); then
      failure_mode=""
    elif (( checks_passed * 2 < checks_total )); then
      failure_mode=incomplete-behavior
    else
      failure_mode=missing-checks
    fi
  fi
}

actual_artifact() {
  local workspace=$1
  print -r -- 'CANDIDATE RESPONSE:'
  command cat "$workspace/.harness/candidate.txt"
  if [[ -f "$workspace/learning/LADDER.md" ]]; then
    print -r -- '\nLADDER.md:'
    command cat "$workspace/learning/LADDER.md"
  fi
  if [[ -f "$workspace/learning/wrong.md" ]]; then
    print -r -- '\nwrong.md:'
    command cat "$workspace/learning/wrong.md"
  fi
}

run_arm() {
  local arm=$1
  local loaded_skill=$2
  local runner=$candidate_runner
  [[ "$is_smoke" == false ]] || runner=$normal_fake_runner
  integer total_cases=0
  integer total_score=0
  local case_json id input expect source fixture workspace output runner_status verdict judge_score judge_failure actual

  while IFS= read -r case_json; do
    id=$(jq -r '.id' <<<$case_json)
    input=$(jq -r '.input' <<<$case_json)
    expect=$(jq -r '.expect' <<<$case_json)
    source=$(jq -r '.source' <<<$case_json)
    fixture=$(jq -r '.execution.drive.source' <<<$case_json)
    workspace="$temporary_root/workspaces/$arm-$id"
    prepare_workspace "$workspace" "$loaded_skill" "$fixture"

    if run_candidate "$runner" "$id" "$input" "$workspace"; then
      verify_case "$id" "$workspace"
      if [[ "$is_smoke" == false ]]; then
        actual=$(actual_artifact "$workspace")
        verdict=$(grade_actual_output "$input" "$expect" "$actual") || { print -u2 -r -- "shared skill-eval judge failed for $arm/$id"; exit 1; }
        judge_score=$(jq -r '.score' <<<$verdict)
        judge_failure=$(jq -r '.failure_mode // ""' <<<$verdict)
        if (( judge_score < score )); then
          score=$judge_score
          failure_mode=${judge_failure:-actual-output}
        fi
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

    output=$(jq -cn --arg arm "$arm" --arg id "$id" --arg source "$source" --argjson score "$score" --arg failure_mode "$failure_mode" --argjson checks_passed "$checks_passed" --argjson checks_total "$checks_total" '{arm:$arm,id:$id,source:$source,drive:"fixture",score:$score,failure_mode:(if $failure_mode == "" then null else $failure_mode end),checks_passed:$checks_passed,checks_total:$checks_total}')
    [[ "$output" != *"$heldout_sentinel"* ]] || { print -u2 -r -- 'held-out sentinel leaked into output'; exit 1; }
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
