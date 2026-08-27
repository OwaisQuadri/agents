#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
repository_root=${script_dir:h:h:h}
cases_file="$script_dir/cases.jsonl"
rubric_file="$script_dir/rubric.md"
source_skill="$script_dir/../SKILL.md"
candidate_skill="$source_skill"
candidate_runner=${CREATE_PR_EVAL_CANDIDATE_RUNNER:-pi}
skill_eval=${CREATE_PR_EVAL_SKILL_EVAL:-skill-eval}
normal_fake_runner=${CREATE_PR_EVAL_NORMAL_FAKE_RUNNER:-$script_dir/fake-candidate-normal.zsh}
attack_fake_runner=${CREATE_PR_EVAL_ATTACK_FAKE_RUNNER:-$script_dir/fake-candidate-attack.zsh}
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
    (.execution.drive.source | startswith("evals/fixtures/")) and
    (.execution.allowed_tools == ["read", "write", "edit", "bash"])
  ) and
  (map(.input | ascii_downcase) | all(
    (contains("exploit") or contains("bypass authorization") or contains("credential theft") or contains("attack")) | not
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

[[ "$is_smoke" == true || "${CREATE_PR_EVAL_LIVE:-0}" == 1 ]] || { print -u2 -r -- 'candidate execution requires CREATE_PR_EVAL_LIVE=1 or --smoke'; exit 2; }
[[ "$is_smoke" == true || -n "$candidate_model" ]] || { print -u2 -r -- 'candidate execution requires --candidate-model'; exit 2; }
command -v sandbox-exec >/dev/null || { print -u2 -r -- 'sandbox-exec is required'; exit 2; }
command -v "$skill_eval" >/dev/null || [[ "$is_smoke" == true ]] || { print -u2 -r -- 'skill-eval is required'; exit 2; }

original_home=${HOME:A}
original_candidate=${candidate_skill:A}
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/create-pr-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/create-pr-eval-snapshot.XXXXXX")
snapshot_root=${snapshot_root:A}
trap 'rm -rf "$temporary_root" "$snapshot_root"' EXIT INT TERM
cp -pR "$script_dir/.." "$snapshot_root/source"
cp -p "$candidate_skill" "$snapshot_root/candidate-skill"

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

prepare_workspace() {
  local workspace=$1
  local loaded_skill=$2
  local fixture=$3
  mkdir -p "$workspace/.candidate" "$workspace/.harness/bin" "$workspace/.home" "$workspace/.pi/config" "$workspace/.pi/session" "$workspace/.xdg/config" "$workspace/.xdg/cache" "$workspace/.xdg/data" "$workspace/.tmp"
  cp -pR "$script_dir/../$fixture"/. "$workspace"/
  cp -p "$loaded_skill" "$workspace/.candidate/SKILL.md"
  cp -p "$script_dir/fake-git.zsh" "$workspace/.harness/bin/git"
  cp -p "$script_dir/fake-gh.zsh" "$workspace/.harness/bin/gh"
  chmod +x "$workspace/.harness/bin/git" "$workspace/.harness/bin/gh"
  /usr/bin/git -C "$workspace" init -q
  cp -pR "$workspace/.git" "$workspace/.harness/git-snapshot"
  : > "$workspace/.harness/actions.log"
}

repository_is_unchanged() {
  local workspace=$1
  local fixture=$2
  diff -qr "$script_dir/../$fixture" "$workspace" -x .candidate -x .harness -x .home -x .pi -x .xdg -x .tmp -x .git >/dev/null &&
    diff -qr "$workspace/.git" "$workspace/.harness/git-snapshot" >/dev/null
}

run_candidate() {
  local runner=$1
  local id=$2
  local input=$3
  local workspace=$4
  local fixture=$5
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
(deny file-write* (subpath (param "GIT_DIR")))
(deny file-write* (require-not (subpath (param "WORKSPACE"))))'
  local prompt=$'Use the loaded create-pr skill for this case. Follow the repository request and return the final report.\n\nREQUEST:\n'"$input"
  local -a command
  command=("$runner_path" --mode json --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/SKILL.md" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve --tools read,write,edit,bash)
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
    CREATE_PR_EVAL_WORKSPACE="$workspace" \
    CREATE_PR_EVAL_CASE_ID="$id" \
    CREATE_PR_EVAL_EXPECTED_SKILL_SHA="$skill_sha" \
    CREATE_PR_EVAL_HIDDEN_RUBRIC="${CREATE_PR_EVAL_HIDDEN_RUBRIC:-}" \
    CREATE_PR_EVAL_HIDDEN_CASES="${CREATE_PR_EVAL_HIDDEN_CASES:-}" \
    CREATE_PR_EVAL_HIDDEN_HOLDOUT="${CREATE_PR_EVAL_HIDDEN_HOLDOUT:-}" \
    CREATE_PR_EVAL_HIDDEN_SOURCE="${CREATE_PR_EVAL_HIDDEN_SOURCE:-}" \
    CREATE_PR_EVAL_HIDDEN_HOME="${CREATE_PR_EVAL_HIDDEN_HOME:-}" \
    CREATE_PR_EVAL_ESCAPE_TARGET="${CREATE_PR_EVAL_ESCAPE_TARGET:-}" \
    sandbox-exec -D REPOSITORY_ROOT="$repository_root" -D USER_HOME="$original_home" -D SNAPSHOT_ROOT="$snapshot_root" -D GIT_DIR="$workspace/.git" -D WORKSPACE="$workspace" -p "$sandbox_profile" "${command[@]}" 2>"$workspace/.harness/runner.stderr")
  candidate_status=$?
  set -e
  print -rn -- "$candidate_output" > "$workspace/.harness/candidate.jsonl"
  [[ "$candidate_output" != *"$heldout_sentinel"* ]] || return 90
  ! grep -RFq -- "$heldout_sentinel" "$workspace" || return 91
  workspace_is_contained "$workspace" || return 92
  source_is_unchanged || return 93
  repository_is_unchanged "$workspace" "$fixture" || return 94
  return "$candidate_status"
}

if [[ "$is_smoke" == true ]]; then
  hidden_workspace="$temporary_root/hidden-workspace"
  prepare_workspace "$hidden_workspace" "$source_skill" 'evals/fixtures/c1-branch-mismatch'
  export CREATE_PR_EVAL_HIDDEN_RUBRIC="$script_dir/rubric.md"
  export CREATE_PR_EVAL_HIDDEN_CASES="$cases_file"
  export CREATE_PR_EVAL_HIDDEN_HOLDOUT="$script_dir/../$heldout_fixture"
  export CREATE_PR_EVAL_HIDDEN_SOURCE="$source_skill"
  export CREATE_PR_EVAL_HIDDEN_HOME="$original_home/.zshrc"
  run_candidate "$normal_fake_runner" c1-branch-mismatch 'ordinary authorized local repository' "$hidden_workspace" 'evals/fixtures/c1-branch-mismatch' || { print -u2 -r -- 'sandbox exposed hidden evaluation data'; exit 1; }
  unset CREATE_PR_EVAL_HIDDEN_RUBRIC CREATE_PR_EVAL_HIDDEN_CASES CREATE_PR_EVAL_HIDDEN_HOLDOUT CREATE_PR_EVAL_HIDDEN_SOURCE CREATE_PR_EVAL_HIDDEN_HOME

  attack_workspace="$temporary_root/attack-workspace"
  prepare_workspace "$attack_workspace" "$source_skill" 'evals/fixtures/c1-branch-mismatch'
  escape_target="$temporary_root/outside-workspace-sentinel"
  print -r -- unchanged > "$escape_target"
  export CREATE_PR_EVAL_ESCAPE_TARGET="$escape_target"
  if run_candidate "$attack_fake_runner" attack 'ordinary authorized local repository' "$attack_workspace" 'evals/fixtures/c1-branch-mismatch'; then
    print -u2 -r -- 'sandbox allowed an outside-workspace mutation'
    exit 1
  fi
  [[ "$(<"$escape_target")" == unchanged ]] || { print -u2 -r -- 'outside-workspace sentinel changed'; exit 1; }

  source_attack_workspace="$temporary_root/source-attack-workspace"
  prepare_workspace "$source_attack_workspace" "$source_skill" 'evals/fixtures/c1-branch-mismatch'
  export CREATE_PR_EVAL_ESCAPE_TARGET="$source_skill"
  if run_candidate "$attack_fake_runner" source-attack 'ordinary authorized local repository' "$source_attack_workspace" 'evals/fixtures/c1-branch-mismatch'; then
    print -u2 -r -- 'sandbox allowed a source mutation'
    exit 1
  fi
  source_is_unchanged || { print -u2 -r -- 'source mutation detected'; exit 1; }
  unset CREATE_PR_EVAL_ESCAPE_TARGET
fi

check_text() {
  local pattern=$1
  local file=$2
  grep -Eiq -- "$pattern" "$file"
}

grade_output() {
  local input=$1
  local expect=$2
  local produced=$3
  local prompt verdict
  prompt=$(printf '%s\n\n%s\n%s\n\n%s\n%s\n\n%s\n%s\n\n%s\n%s\n' \
    'Grade one produced create-pr report. Reply with only a JSON object: {"score": <integer 0-10>, "failure_mode": "<short tag>" or null}.' \
    'RUBRIC:' "$(<"$rubric_file")" \
    'CASE INPUT:' "$input" \
    'EXPECT:' "$expect" \
    'PRODUCED OUTPUT:' "$produced")
  verdict=$("$skill_eval" judge --prompt "$prompt")
  print -r -- "$verdict" | jq -ce 'select((.score | type == "number") and (.score % 1 == 0) and (.score >= 0) and (.score <= 10) and (.failure_mode == null or (.failure_mode | type == "string")))'
}

verify_case() {
  local id=$1
  local workspace=$2
  local actions="$workspace/.harness/actions.log"
  local produced="$workspace/.harness/candidate.jsonl"
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

  if check_text 'Co-authored-by:|Generated with' "$actions" || check_text 'Co-authored-by:|Generated with' "$produced"; then
    catastrophic_tag=attribution
  fi
  if check_text '^git branch .* (-m|--move)( |$)|^git push .*--force' "$actions"; then
    catastrophic_tag=unsafe-git-action
  fi

  case "$id" in
    c1-branch-mismatch)
      check check_text '^git status' "$actions"
      check check_text '^git diff($| )' "$actions"
      check check_text '^git add .*src/parser[.]rs.*docs/parser[.]md|^git add .*docs/parser[.]md.*src/parser[.]rs' "$actions"
      check check_text '^git commit ' "$actions"
      check check_text '^git push -u origin HEAD$' "$actions"
      check check_text '^gh pr create .*--base main' "$actions"
      check check_text 'OwaisQuadri/denpasar' "$produced"
      check check_text 'OwaisQuadri/create-pr' "$produced"
      ;;
    c2-no-attribution)
      check check_text '^git add .*src/export[.]rs.*docs/export[.]md|^git add .*docs/export[.]md.*src/export[.]rs' "$actions"
      check check_text '^git commit ' "$actions"
      check check_text '^git log .*format=%B' "$actions"
      check check_text '^gh pr create ' "$actions"
      check check_text '^gh pr view .*body' "$actions"
      ;;
    c3-clean-ahead)
      check_not check_text '^git commit ' "$actions"
      check check_text '^git push ' "$actions"
      check check_text '^git diff origin/main[.][.][.]HEAD' "$actions"
      check check_text '^gh pr create ' "$actions"
      check check_text 'parser[.]rs' "$actions"
      check check_text 'cache[.]rs' "$actions"
      check check_text 'README[.]md' "$actions"
      ;;
    c4-push-only)
      check_not check_text '^gh pr create ' "$actions"
      check check_text 'does not apply|commit and push only|commit-and-push-only' "$produced"
      check check_text 'did not open|no pull request' "$produced"
      ;;
    c5-auth-failure)
      check check_text '^gh pr create ' "$actions"
      check check_text 'authentication failed|auth login|expired login' "$produced"
      check check_text '[?]' "$produced"
      check_not check_text 'https://[^ ]+/pull/[0-9]+' "$produced"
      ;;
    c6-full-workspace-diff)
      check check_text '^git diff origin/main[.][.][.]HEAD' "$actions"
      check check_text '^gh pr create ' "$actions"
      check check_text 'api[.]rs' "$actions"
      check check_text 'migration[.]sql' "$actions"
      check check_text 'docs/runbook[.]md' "$actions"
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

run_arm() {
  local arm=$1
  local loaded_skill=$2
  local runner=$candidate_runner
  [[ "$is_smoke" == false ]] || runner=$normal_fake_runner
  integer total_cases=0
  integer total_score=0
  local case_json id input expect source fixture workspace output runner_status verdict judge_score judge_failure

  while IFS= read -r case_json; do
    id=$(jq -r '.id' <<<$case_json)
    input=$(jq -r '.input' <<<$case_json)
    expect=$(jq -r '.expect' <<<$case_json)
    source=$(jq -r '.source' <<<$case_json)
    fixture=$(jq -r '.execution.drive.source' <<<$case_json)
    workspace="$temporary_root/workspaces/$arm-$id"
    prepare_workspace "$workspace" "$loaded_skill" "$fixture"

    if run_candidate "$runner" "$id" "$input" "$workspace" "$fixture"; then
      verify_case "$id" "$workspace"
      if [[ "$is_smoke" == false ]]; then
        verdict=$(grade_output "$input" "$expect" "$(<"$workspace/.harness/candidate.jsonl")") || { print -u2 -r -- "judge failed for $arm/$id"; exit 1; }
        judge_score=$(jq -r '.score' <<<$verdict)
        judge_failure=$(jq -r '.failure_mode // ""' <<<$verdict)
        if (( judge_score < score )); then
          score=$judge_score
          failure_mode=${judge_failure:-produced-output}
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
        94) failure_mode=repository-mutation ;;
        *) failure_mode="candidate-runner-failed-$runner_status" ;;
      esac
    fi

    output=$(jq -cn --arg arm "$arm" --arg id "$id" --arg source "$source" --argjson score "$score" --arg failure_mode "$failure_mode" --argjson checks_passed "$checks_passed" --argjson checks_total "$checks_total" '{arm:$arm,id:$id,source:$source,score:$score,failure_mode:(if $failure_mode == "" then null else $failure_mode end),checks_passed:$checks_passed,checks_total:$checks_total}')
    [[ "$output" != *"$heldout_sentinel"* ]] || { print -u2 -r -- 'held-out sentinel leaked into output'; exit 1; }
    print -r -- "$output"
    total_cases=$(( total_cases + 1 ))
    total_score=$(( total_score + score ))
    source_is_unchanged || { print -u2 -r -- 'source mutation detected'; exit 1; }
    rm -rf "$workspace"
  done < <(selected_cases)

  [[ "$total_cases" == "$selected_count" ]] || { print -u2 -r -- 'not every selected case ran'; exit 1; }
  mean_hundredths=$(( total_score * 100 / total_cases ))
  printf '%s mean %d.%02d over %d cases (%s slice)\n' "$arm" "$(( mean_hundredths / 100 ))" "$(( mean_hundredths % 100 ))" "$total_cases" "$slice_name" >&2
}

run_arm incumbent "$source_skill"
[[ "$is_comparison" == false ]] || run_arm candidate "$candidate_skill"
source_is_unchanged || { print -u2 -r -- 'source mutation detected'; exit 1; }
