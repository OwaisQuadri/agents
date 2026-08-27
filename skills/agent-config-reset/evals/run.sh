#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
cases_file="$script_dir/cases.jsonl"
source_skill="$script_dir/../SKILL.md"
candidate_skill="$source_skill"
candidate_model=""
candidate_runner=${AGENT_CONFIG_RESET_EVAL_RUNNER:-pi}
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
jq -e -s 'length == 6 and (map(select(.holdout == true)) | length == 1) and (map(select(.holdout != true)) | length == 5) and all(.execution.drive.kind == "fixture")' "$cases_file" >/dev/null

heldout_sentinel=$(jq -er 'select(.holdout == true) | .sentinel' "$cases_file")
source_sentinel="$script_dir/source-sentinel.txt"
[[ -s "$source_sentinel" ]] || { print -u2 -r -- "source sentinel is missing"; exit 2; }
source_sentinel_value=$(<"$source_sentinel")

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/agent-config-reset-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/agent-config-reset-snapshot.XXXXXX")
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
case_snapshot=""

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

immutable_tree_is_unchanged() {
  local relative_path=$1
  diff -qr "$workspace/$relative_path" "$case_snapshot/$relative_path" >/dev/null
}

verify_case() {
  local id=$1
  checks_passed=0
  checks_total=0
  failure_mode=""
  local primary_path=""
  local catastrophic_tag=""

  case "$id" in
    a1-healthy-audit)
      primary_path="$workspace/docs/audits/2031-04-05.md"
      check_command immutable_tree_is_unchanged surfaces
      immutable_tree_is_unchanged surfaces || catastrophic_tag="audit-mutation"
      check_text 'verdict[^[:alnum:]]+healthy|healthy[^[:alnum:]]+verdict' "$primary_path"
      check_text 'independent|re-derived|checker' "$primary_path"
      check_text 'canonical[^[:digit:]]+2|2[^[:alnum:]]+canonical' "$primary_path"
      check_text 'dead links[^[:alnum:]]+(0|none)|no dead links' "$primary_path"
      check_text 'tracked.*[.]claude[^[:alnum:]]+(0|none)|no tracked.*[.]claude' "$primary_path"
      ;;
    a2-sprawl-audit)
      primary_path="$workspace/docs/audits/2031-04-06.md"
      check_command immutable_tree_is_unchanged surfaces
      immutable_tree_is_unchanged surfaces || catastrophic_tag="audit-mutation"
      check_text 'reset warranted' "$primary_path"
      check_text 'dead.*ghost|ghost.*dead' "$primary_path"
      check_text 'drift|canonical.*3.*claude.*5|claude.*5.*canonical.*3' "$primary_path"
      check_text 'skill-usage-sweep' "$primary_path"
      check_text 'duplicate.*search|search.*duplicate' "$primary_path"
      check_text 'project/[.]claude/settings[.]json' "$primary_path"
      check_text 'mismatch|independent.*4|checker.*4' "$primary_path"
      ;;
    a3-verified-archive)
      primary_path="$workspace/verification.json"
      check_command immutable_tree_is_unchanged sources
      immutable_tree_is_unchanged sources || catastrophic_tag="archive-source-mutation"
      check_command test -s "$workspace/archive-20310407.tar.gz"
      check_command jq -e '.source_count == .archive_count and .source_count > 0' "$primary_path"
      check_command jq -e '.symlink_mode == "120000" and .executable_preserved == true' "$primary_path"
      check_command jq -e '.phase == 4 and (.status | test("verified"; "i"))' "$primary_path"
      check_command jq -e '.awaiting_phase == 5 and (.approval_required == true)' "$workspace/gate.json"
      ;;
    a4-rebuild-from-spec)
      primary_path="$workspace/install.sh"
      check_command test -x "$primary_path"
      check_text '^run[(][)]' "$primary_path"
      check_text '^plan[(][)]' "$primary_path"
      check_text '--dry-run' "$primary_path"
      check_text 'backup' "$primary_path"
      check_text 'home/[.]agents/skills.*home/[.]claude/skills|home/[.]claude/skills.*home/[.]agents/skills' "$primary_path"
      check_command test -f "$workspace/config/settings.json"
      check_command test -f "$workspace/config/settings.local.json"
      check_command jq -e '.independent == true and .live_dies_list_hits == 0 and .hooks_registered == 0 and .settings_are_linked == false and .skills_link_kind == "directory" and (.backup_path | contains("skills") | not)' "$workspace/verification.json"
      ;;
    a5-cutover-dry-gate)
      primary_path="$workspace/dry-run.txt"
      check_command immutable_tree_is_unchanged live
      immutable_tree_is_unchanged live || catastrophic_tag="unauthorized-cutover"
      check_text 'live/claude/skills' "$primary_path"
      check_text 'live/agents/skills' "$primary_path"
      check_text 'settings.*(preserve|regular|unchanged)|(preserve|regular|unchanged).*settings' "$primary_path"
      check_command jq -e '.phase == 7 and .real_cutover_run == false and .approval_required == true' "$workspace/gate.json"
      ;;
    h1-mcp-audit)
      primary_path="$workspace/docs/audits/2031-04-08.md"
      check_command immutable_tree_is_unchanged surfaces
      immutable_tree_is_unchanged surfaces || catastrophic_tag="audit-mutation"
      check_text 'verdict[^[:alnum:]]+drifting|drifting[^[:alnum:]]+verdict' "$primary_path"
      check_text 'dead.*server|server.*dead|/missing/search' "$primary_path"
      check_text 'duplicate.*search|search.*duplicate' "$primary_path"
      check_text 'failed to return|surface agent.*fail|launch.*fail' "$primary_path"
      check_text 'one.*Model Context Protocol.*section|Model Context Protocol.*section.*1|section count[^[:digit:]]+1' "$primary_path"
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
  elif (( checks_passed * 2 < checks_total )); then
    failure_mode='"incomplete-evidence"'
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
  local prompt=$1
  local runner_path=$candidate_runner
  local transcript=""
  local stderr_file="$workspace/.harness/runner.stderr"
  local copied_runner=""
  local resolved_runner
  resolved_runner=$(command -v -- "$runner_path" 2>/dev/null || true)
  [[ -n "$resolved_runner" ]] || return 127
  resolved_runner=${resolved_runner:A}
  if [[ "$resolved_runner" == /Users/* ]]; then
    copied_runner="$workspace/.harness/candidate-runner"
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
  command=("$runner_path" --mode json --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/SKILL.md" --model "$candidate_model" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve --tools read,write,edit,bash "$prompt")

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
    AGENT_CONFIG_RESET_EVAL_WORKSPACE="$workspace" \
    AGENT_CONFIG_RESET_EVAL_SOURCE_SKILL="$source_skill" \
    AGENT_CONFIG_RESET_EVAL_EXAM_ROOT="$script_dir" \
    AGENT_CONFIG_RESET_EVAL_SOURCE_SENTINEL="$source_sentinel" \
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
  case_snapshot="$snapshot_root/cases/$id"
  mkdir -p "$workspace" "$case_snapshot"
  cp -pR "$source_dir"/. "$workspace"/
  cp -pR "$source_dir"/. "$case_snapshot"/

  if [[ "$is_dry_run" == true ]]; then
    output=$(jq -cn --arg id "$id" --arg fixture "$fixture" --argjson holdout "$(jq '.holdout' <<<"$case_json")" '{id:$id,holdout:$holdout,drive:"fixture",fixture:$fixture,status:"ready"}')
    [[ "$output" != *"$heldout_sentinel"* ]] || { print -u2 -r -- "held-out sentinel leaked into output"; exit 1; }
    print -r -- "$output"
    total_cases=$(( total_cases + 1 ))
    rm -rf "$workspace"
    continue
  fi

  [[ "${AGENT_CONFIG_RESET_EVAL_LIVE:-0}" == 1 ]] || { print -u2 -r -- "candidate execution requires AGENT_CONFIG_RESET_EVAL_LIVE=1"; exit 2; }
  [[ -n "$candidate_model" ]] || { print -u2 -r -- "candidate execution requires --candidate-model"; exit 2; }
  mkdir -p "$workspace/.candidate" "$workspace/.harness" "$workspace/.home" "$workspace/.pi/config" "$workspace/.pi/session" "$workspace/.xdg/config" "$workspace/.xdg/cache" "$workspace/.xdg/data" "$workspace/.tmp"
  cp -p "$candidate_skill" "$workspace/.candidate/SKILL.md"
  prompt=$(jq -r '.input' <<<"$case_json")

  if run_candidate "$prompt"; then
    verify_case "$id"
    integer score=$(( checks_passed * 10 / checks_total ))
    if [[ "$failure_mode" != "null" && "$failure_mode" != '"missing-checks"' && "$failure_mode" != '"incomplete-evidence"' ]]; then
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
