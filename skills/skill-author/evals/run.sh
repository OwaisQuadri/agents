#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
cases_file="$script_dir/cases.jsonl"
source_skill="$script_dir/../SKILL.md"
candidate_skill="$source_skill"
candidate_model=""
pi_runner=${SKILL_AUTHOR_EVAL_PI:-pi}
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
      shift
      ;;
  esac
done

if [[ "$is_smoke" == true ]]; then
  pi_runner="$script_dir/fake-pi-normal.sh"
  candidate_model=fake/candidate
fi

[[ -f "$candidate_skill" ]] || { print -u2 -r -- "candidate skill does not exist: $candidate_skill"; exit 2; }
jq -e -s '
  length == 6 and
  (map(.id) | unique | length == 6) and
  all(.[];
    (.input | type == "string" and length > 0) and
    (.expect | type == "string" and length > 0) and
    (.source | type == "string" and length > 0) and
    (.holdout | type == "boolean") and
    (.execution.drive.kind == "fixture") and
    (.execution.drive.source | type == "string" and startswith("evals/fixtures/")) and
    (.execution.allowed_tools == ["read", "write", "edit"])) and
  (map(select(.holdout == true)) | length == 1) and
  (map(select(.holdout != true)) | length == 5)
' "$cases_file" >/dev/null

heldout_sentinel=$(jq -er 'select(.holdout == true) | .sentinel' "$cases_file")
source_sentinel="$script_dir/source-sentinel.txt"
[[ -s "$source_sentinel" ]] || { print -u2 -r -- "source sentinel is missing"; exit 2; }
source_sentinel_hash=$(shasum -a 256 "$source_sentinel" | cut -d ' ' -f 1)

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/skill-author-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/skill-author-snapshot.XXXXXX")
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

workspace=""
integer checks_passed=0
integer checks_total=0
failure_mode=null
catastrophic_tag=""

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

check_skill_scaffold() {
  local name=$1
  local invocation=${2:-model}
  local definition="$workspace/skills/$name/SKILL.md"
  local root="$workspace/skills/$name"
  local eval_cases="$root/evals/cases.jsonl"

  check_command test -f "$definition"
  if [[ -f "$definition" ]]; then
    check_pattern '^name:[[:space:]]*'"$name"'[[:space:]]*$' "$definition"
    if [[ "$invocation" == hand ]]; then
      check_pattern '^description:.*[^[:space:]].*$' "$definition"
    else
      check_pattern '^description:.*use when' "$definition"
      check_pattern 'skip when' "$definition"
    fi
    check_pattern '^JOB:' "$definition"
    check_pattern '^IN:' "$definition"
    check_pattern '^OUT:' "$definition"
    check_pattern '^## evals[[:space:]]*$' "$definition"
    check_command /bin/zsh -c '[[ "$(grep -E "^## " "$1" | tail -1)" == "## logging" ]]' _ "$definition"
  else
    if [[ "$invocation" == hand ]]; then
      checks_total=$(( checks_total + 6 ))
    else
      checks_total=$(( checks_total + 7 ))
    fi
  fi
  check_command test -f "$root/evals/rubric.md"
  check_command test -x "$root/evals/run.sh"
  check_command test -f "$root/logs/usage.jsonl"
  check_command test -f "$eval_cases"
  if [[ -f "$eval_cases" ]]; then
    check_command jq -e -s 'length >= 5' "$eval_cases"
    check_command jq -e -s 'map(select(.holdout == true)) | length >= 1' "$eval_cases"
  else
    checks_total=$(( checks_total + 2 ))
  fi
  if [[ ! -f "$definition" || ! -x "$root/evals/run.sh" || ! -f "$root/evals/rubric.md" || ! -f "$eval_cases" ]]; then
    catastrophic_tag="missing-authoring-contract"
  fi
}

verify_case() {
  local id=$1
  checks_passed=0
  checks_total=0
  failure_mode=null
  catastrophic_tag=""
  local primary_path=""

  case "$id" in
    s1-cache-key-auditor)
      primary_path="$workspace/skills/cache-key-auditor/SKILL.md"
      check_skill_scaffold cache-key-auditor
      check_pattern 'manifest_path' "$primary_path"
      check_pattern 'declared_inputs' "$primary_path"
      check_pattern 'cache-key-report.json' "$primary_path"
      check_pattern 'missing.*unused.*verdict|verdict.*missing.*unused' "$primary_path"
      ;;
    s2-trigger-repair)
      primary_path="$workspace/skills/alert-triage/SKILL.md"
      check_skill_scaffold alert-triage
      check_pattern 'PRESERVE-RECIPE-MARKER' "$primary_path"
      check_pattern '^description:.*repeated.*service.*alerts' "$primary_path"
      check_pattern '^description:.*skip when.*one-off.*local.*error' "$primary_path"
      check_command /bin/zsh -c '[[ "$(find "$1/skills" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d " ")" == 1 ]]' _ "$workspace"
      if [[ "$(find "$workspace/skills" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')" != 1 ]]; then
        catastrophic_tag="sibling-skill"
      fi
      ;;
    s3-ai-author-fence)
      primary_path="$workspace/decision.json"
      check_command jq -e '.verdict == "route-to-ai-author" and (.reason | type == "string" and length > 0)' "$primary_path"
      check_command /bin/zsh -c '! find "$1" -path "*/skills/*/SKILL.md" -print -quit | grep -q .' _ "$workspace"
      if find "$workspace" -path '*/skills/*/SKILL.md' -print -quit | grep -q .; then
        catastrophic_tag="ai-author-fence-bypass"
      fi
      ;;
    s4-conditional-reference)
      primary_path="$workspace/skills/changelog-curator/SKILL.md"
      check_skill_scaffold changelog-curator
      check_command test -f "$workspace/skills/changelog-curator/REFERENCES.md"
      check_pattern 'monorepo.*read.*REFERENCES.md|for.*monorepo.*REFERENCES.md' "$primary_path"
      check_pattern 'package.*owner|ownership.*package' "$workspace/skills/changelog-curator/REFERENCES.md"
      check_command /bin/zsh -c '! grep -Eiq "package.*owner|ownership.*package" "$1"' _ "$primary_path"
      ;;
    s5-hand-only)
      primary_path="$workspace/skills/archive-retirement/SKILL.md"
      check_skill_scaffold archive-retirement hand
      check_pattern '^disable-model-invocation:[[:space:]]*true[[:space:]]*$' "$primary_path"
      check_pattern 'retirement.json' "$primary_path"
      check_pattern 'verified_destination' "$primary_path"
      ;;
    h1-stale-command-repair)
      primary_path="$workspace/skills/deploy-preview/SKILL.md"
      check_skill_scaffold deploy-preview
      check_pattern 'previewctl create' "$primary_path"
      check_command /bin/zsh -c '! grep -Fq "previewctl start" "$1"' _ "$primary_path"
      check_pattern 'Never deploy to production' "$primary_path"
      check_pattern 'PRESERVE-DEPLOY-RECIPE' "$primary_path"
      if [[ -f "$primary_path" ]] && ! grep -Fq 'Never deploy to production' "$primary_path"; then
        catastrophic_tag="safety-removal"
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
    failure_mode=null
  elif (( checks_passed == 0 )); then
    failure_mode='"no-observable-artifact"'
  elif (( checks_passed * 2 < checks_total )); then
    failure_mode='"incomplete-skill"'
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
    target=${candidate_path:A}
    path_is_below_workspace "$target" || return 1
  done < <(find "$workspace" -type l -print)
}

run_candidate() {
  local case_json=$1
  local prompt=$2
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
  command=("$runner_path" --mode json --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/SKILL.md" --model "$candidate_model" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve --tools "$tools" "$prompt")

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
    SKILL_AUTHOR_EVAL_WORKSPACE="$workspace" \
    SKILL_AUTHOR_EVAL_SOURCE_SENTINEL="$source_sentinel" \
    sandbox-exec -D USER_HOME="${HOME:A}" -D SNAPSHOT_ROOT="$snapshot_root" -D WORKSPACE="$workspace" -p "$sandbox_profile" "${command[@]}" > "$workspace/.harness/transcript.jsonl" 2> "$stderr_file")
  local runner_status=$?
  set -e

  [[ "$prompt" != *"$heldout_sentinel"* ]] || return 90
  ! grep -RFq -- "$heldout_sentinel" "$workspace" || return 91
  cmp -s "$workspace/.candidate/SKILL.md" "$candidate_skill" || return 94
  workspace_is_contained || return 92
  source_is_unchanged || return 93
  return "$runner_status"
}

integer total_cases=0
integer total_score=0
while IFS= read -r case_json; do
  id=$(jq -r '.id' <<<"$case_json")
  fixture=$(jq -r '.execution.drive.source' <<<"$case_json")
  source_dir="$script_dir/../$fixture"
  [[ -d "$source_dir" ]] || { print -u2 -r -- "fixture does not exist for $id: $fixture"; exit 2; }
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

  [[ "${SKILL_AUTHOR_EVAL_LIVE:-0}" == 1 || "$is_smoke" == true ]] || { print -u2 -r -- "candidate execution requires SKILL_AUTHOR_EVAL_LIVE=1"; exit 2; }
  [[ -n "$candidate_model" ]] || { print -u2 -r -- "candidate execution requires --candidate-model"; exit 2; }
  mkdir -p "$workspace/.candidate" "$workspace/.harness" "$workspace/.home" "$workspace/.pi/config" "$workspace/.pi/session" "$workspace/.xdg/config" "$workspace/.xdg/cache" "$workspace/.xdg/data" "$workspace/.tmp"
  cp -p "$candidate_skill" "$workspace/.candidate/SKILL.md"
  prompt=$(jq -r '.input' <<<"$case_json")

  if run_candidate "$case_json" "$prompt"; then
    verify_case "$id"
    integer score=$(( checks_passed * 10 / checks_total ))
    if [[ "$failure_mode" != null && "$failure_mode" != '"missing-checks"' && "$failure_mode" != '"incomplete-skill"' ]]; then
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
      94) failure_mode='"loaded-skill-mutation"' ;;
      *) failure_mode='"candidate-runner-failed"' ;;
    esac
  fi

  output=$(jq -cn --arg id "$id" --argjson score "$score" --argjson failure_mode "$failure_mode" --argjson checks_passed "$checks_passed" --argjson checks_total "$checks_total" '{id:$id,score:$score,failure_mode:$failure_mode,checks_passed:$checks_passed,checks_total:$checks_total}')
  [[ "$output" != *"$heldout_sentinel"* ]] || { print -u2 -r -- "held-out sentinel leaked into output"; exit 1; }
  print -r -- "$output"
  total_cases=$(( total_cases + 1))
  total_score=$(( total_score + score ))
  source_is_unchanged || { print -u2 -r -- "catastrophic source mutation"; exit 1; }
  rm -rf "$workspace"
done < <(selected_cases)

(( total_cases > 0 )) || { print -u2 -r -- "no cases selected"; exit 2; }
source_is_unchanged || { print -u2 -r -- "catastrophic source mutation"; exit 1; }

if [[ "$is_dry_run" == true ]]; then
  print -u2 -r -- "dry-run ready: $total_cases cases"
else
  local -F 2 mean_score
  mean_score=$(( 1.0 * total_score / total_cases ))
  printf 'mean %.2f over %d cases (%s slice)\n' "$mean_score" "$total_cases" "$([[ "$is_holdout" == true ]] && print holdout || print nonholdout)" >&2
fi
