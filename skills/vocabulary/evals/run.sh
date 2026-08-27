#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
skill_root=${script_dir:h}
repository_root=${skill_root:h:h}
cases_file="$script_dir/cases.jsonl"
rubric_file="$script_dir/rubric.md"
source_skill="$skill_root/SKILL.md"
candidate_skill="$source_skill"
candidate_runner=${VOCABULARY_EVAL_CANDIDATE_RUNNER:-pi}
normal_fake_runner=${VOCABULARY_EVAL_NORMAL_FAKE_RUNNER:-$script_dir/fake-candidate-normal.zsh}
attack_fake_runner=${VOCABULARY_EVAL_ATTACK_FAKE_RUNNER:-$script_dir/fake-candidate-attack.zsh}
skill_eval=${VOCABULARY_EVAL_SKILL_EVAL:-skill-eval}
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
  length == 7 and
  (map(.id) | unique | length) == length and
  (map(select(.holdout == true)) | length) == 1 and
  (map(select(.holdout != true)) | length) == 6 and
  all(.[];
    (.id | type == "string" and length > 0) and
    (.input | type == "string" and length > 0) and
    (.expect | type == "string" and length > 0) and
    (.source | type == "string" and length > 0) and
    (.holdout | type == "boolean") and
    (.execution.drive.kind == "response") and
    (.execution.drive.source == ("evals/fixtures/" + .id)) and
    (.execution.allowed_tools == ["read"]) and
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
expected_count=6
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
      '{arm:$arm,id:$id,source:$source,fixture:$fixture,slice:$slice,drive:"response",status:"ready"}'
  done < <(selected_cases)
  print -u2 -r -- "$arm dry-run ready: $selected_count cases ($slice_name slice)"
}

if [[ "$is_dry_run" == true ]]; then
  run_dry_arm incumbent
  [[ "$is_comparison" == false ]] || run_dry_arm candidate
  exit 0
fi

[[ "$is_smoke" == true || "${VOCABULARY_EVAL_LIVE:-0}" == 1 ]] || { print -u2 -r -- 'candidate execution requires VOCABULARY_EVAL_LIVE=1 or --smoke'; exit 2; }
[[ "$is_smoke" == true || -n "$candidate_model" ]] || { print -u2 -r -- 'candidate execution requires --candidate-model'; exit 2; }
command -v sandbox-exec >/dev/null || { print -u2 -r -- 'sandbox-exec is required'; exit 2; }
command -v jq >/dev/null || { print -u2 -r -- 'jq is required'; exit 2; }
command -v "$skill_eval" >/dev/null || [[ "$is_smoke" == true ]] || { print -u2 -r -- 'skill-eval is required'; exit 2; }

original_home=${HOME:A}
original_candidate=${candidate_skill:A}
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/vocabulary-eval.XXXXXX")
temporary_root=${temporary_root:A}
snapshot_root=$(mktemp -d "${TMPDIR:-/tmp}/vocabulary-eval-snapshot.XXXXXX")
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
  local include_response=$4
  mkdir -p "$workspace/.candidate/vocabulary" "$workspace/.harness" "$workspace/.home" "$workspace/.pi/config" "$workspace/.pi/session" "$workspace/.xdg/config" "$workspace/.xdg/cache" "$workspace/.xdg/data" "$workspace/.tmp"
  cp -p "$loaded_skill" "$workspace/.candidate/vocabulary/SKILL.md"
  cp -p "$skill_root/vocabulary.md" "$skill_root/terms-index.md" "$skill_root/symptom-map.md" "$workspace/.candidate/vocabulary/"
  cp -p "$script_dir/../$fixture/context.md" "$workspace/context.md"
  if [[ "$include_response" == true ]]; then
    cp -p "$script_dir/../$fixture/response.txt" "$workspace/.harness/response.txt"
  fi
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
  skill_sha=$(shasum -a 256 "$workspace/.candidate/vocabulary/SKILL.md" | cut -d ' ' -f 1)
  local sandbox_profile='(version 1)
(allow default)
(deny network*)
(deny file-read* (subpath (param "REPOSITORY_ROOT")))
(deny file-read* (subpath (param "USER_HOME")))
(deny file-read* (subpath (param "SNAPSHOT_ROOT")))
(deny file-write* (require-not (subpath (param "WORKSPACE"))))'
  local prompt=$'Use the loaded vocabulary skill for this ordinary authorized interface terminology request. Read only the loaded skill files and context.md. The offline context replaces a web search when it contains a search result. Do not access the rubric, another case, the holdout, source files, snapshots, or the real home. Do not use the network or modify files. Return only the response to the request.\n\nREQUEST:\n'"$input"
  local -a command
  command=("$runner_path" -p --no-session --session-dir "$workspace/.pi/session" --no-skills --skill "$workspace/.candidate/vocabulary/SKILL.md" --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve --tools read)
  [[ -z "$candidate_model" ]] || command+=(--model "$candidate_model")
  command+=("$prompt")

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
    VOCABULARY_EVAL_WORKSPACE="$workspace" \
    VOCABULARY_EVAL_CASE_ID="$id" \
    VOCABULARY_EVAL_EXPECTED_SKILL_SHA="$skill_sha" \
    VOCABULARY_EVAL_HIDDEN_RUBRIC="${VOCABULARY_EVAL_HIDDEN_RUBRIC:-}" \
    VOCABULARY_EVAL_HIDDEN_CASES="${VOCABULARY_EVAL_HIDDEN_CASES:-}" \
    VOCABULARY_EVAL_HIDDEN_HOLDOUT="${VOCABULARY_EVAL_HIDDEN_HOLDOUT:-}" \
    VOCABULARY_EVAL_HIDDEN_SOURCE="${VOCABULARY_EVAL_HIDDEN_SOURCE:-}" \
    VOCABULARY_EVAL_HIDDEN_HOME="${VOCABULARY_EVAL_HIDDEN_HOME:-}" \
    VOCABULARY_EVAL_HIDDEN_SNAPSHOT="${VOCABULARY_EVAL_HIDDEN_SNAPSHOT:-}" \
    VOCABULARY_EVAL_ESCAPE_TARGET="${VOCABULARY_EVAL_ESCAPE_TARGET:-}" \
    sandbox-exec -D REPOSITORY_ROOT="$repository_root" -D USER_HOME="$original_home" -D SNAPSHOT_ROOT="$snapshot_root" -D WORKSPACE="$workspace" -p "$sandbox_profile" "${command[@]}" 2>"$workspace/.harness/runner.stderr")
  local candidate_status=$?
  set -e
  print -rn -- "$candidate_output" > "$workspace/.harness/candidate.txt"
  [[ "$candidate_output" != *"$heldout_sentinel"* ]] || return 90
  ! grep -RFq -- "$heldout_sentinel" "$workspace" || return 91
  workspace_is_contained "$workspace" || return 92
  source_is_unchanged || return 93
  [[ "$(shasum -a 256 "$workspace/.candidate/vocabulary/SKILL.md" | cut -d ' ' -f 1)" == "$skill_sha" ]] || return 94
  return "$candidate_status"
}

if [[ "$is_smoke" == true ]]; then
  hidden_workspace="$temporary_root/hidden-workspace"
  prepare_workspace "$hidden_workspace" "$source_skill" 'evals/fixtures/c1' true
  export VOCABULARY_EVAL_HIDDEN_RUBRIC="$rubric_file"
  export VOCABULARY_EVAL_HIDDEN_CASES="$cases_file"
  export VOCABULARY_EVAL_HIDDEN_HOLDOUT="$script_dir/../$heldout_fixture"
  export VOCABULARY_EVAL_HIDDEN_SOURCE="$source_skill"
  export VOCABULARY_EVAL_HIDDEN_HOME="$original_home/.zshrc"
  export VOCABULARY_EVAL_HIDDEN_SNAPSHOT="$snapshot_root/source"
  run_candidate "$normal_fake_runner" hidden 'ordinary authorized interface terminology' "$hidden_workspace" || { print -u2 -r -- 'sandbox exposed hidden evaluation data'; exit 1; }
  unset VOCABULARY_EVAL_HIDDEN_RUBRIC VOCABULARY_EVAL_HIDDEN_CASES VOCABULARY_EVAL_HIDDEN_HOLDOUT VOCABULARY_EVAL_HIDDEN_SOURCE VOCABULARY_EVAL_HIDDEN_HOME VOCABULARY_EVAL_HIDDEN_SNAPSHOT

  outside_target="$temporary_root/outside-workspace-sentinel"
  print -r -- unchanged > "$outside_target"
  attack_workspace="$temporary_root/attack-workspace"
  prepare_workspace "$attack_workspace" "$source_skill" 'evals/fixtures/c1' false
  export VOCABULARY_EVAL_ESCAPE_TARGET="$outside_target"
  if run_candidate "$attack_fake_runner" attack 'ordinary authorized interface terminology' "$attack_workspace"; then
    print -u2 -r -- 'sandbox allowed an outside-workspace mutation'
    exit 1
  fi
  [[ "$(<"$outside_target")" == unchanged ]] || { print -u2 -r -- 'outside-workspace sentinel changed'; exit 1; }

  source_attack_workspace="$temporary_root/source-attack-workspace"
  prepare_workspace "$source_attack_workspace" "$source_skill" 'evals/fixtures/c1' false
  export VOCABULARY_EVAL_ESCAPE_TARGET="$source_skill"
  if run_candidate "$attack_fake_runner" source-attack 'ordinary authorized interface terminology' "$source_attack_workspace"; then
    print -u2 -r -- 'sandbox allowed a source mutation'
    exit 1
  fi
  unset VOCABULARY_EVAL_ESCAPE_TARGET
  source_is_unchanged || { print -u2 -r -- 'source mutation detected'; exit 1; }
  print -u2 -r -- 'sandbox smoke checks passed'
fi

has_exact() {
  local text=$1
  local exact=$2
  [[ "$text" == *"$exact"* ]]
}

verify_response() {
  local id=$1
  local response=$2
  checks_passed=0
  checks_total=0
  failure_mode=""
  check() {
    checks_total=$(( checks_total + 1 ))
    if "$@"; then checks_passed=$(( checks_passed + 1 )); fi
  }

  local leading="The vertical space between lines of text. Too tight and text suffocates. Too loose and it stops reading as a paragraph."
  local tracking="Letter-spacing applied uniformly across a word or block of text. Uppercase labels almost always need more of it."
  local negative_space="The empty area around and between elements. It defines shape, creates breathing room, and guides the eye. Crowding elements together removes clarity, it doesn't add information."
  local gap="Space between flex or grid children, set on the parent. Unlike margin, it leaves no trailing space after the last item."
  local contrast="The luminance difference between a foreground and background color. WCAG requires 4.5:1 for body text, 3:1 for large text and UI components."
  local weight="How thick or thin a typeface's strokes are. Bold is for UI emphasis and hierarchy. Italic is for linguistic stress or citation, not UI hierarchy."
  local saturation="How vivid or muted a color is. Brand colors at full saturation in dark mode can vibrate off the screen. Pulling back 20-30% settles them."
  local chroma="The OKLCH equivalent of saturation, but perceptually accurate. Reducing chroma for a light tint keeps the color alive. Reducing opacity does the same but turns it grey and lifeless."
  local layout_shift="When elements move unexpectedly as a page loads: images popping in, fonts swapping, content reordering. Avoided by reserving space before content loads and using size-matched font fallbacks."
  local font_stack="The ordered list of fonts the browser tries before giving up. A well-matched fallback font avoids layout shift when the primary font loads, since mismatched x-heights and weights cause content to reflow."
  local hover_state="The visual change when a cursor moves over an interactive element. Should confirm interactivity through cursor change, color shift, or both. Color alone is not enough."
  local optimistic_update="Updating the UI before the server confirms the action. Feels instant. Requires a rollback if the request fails."
  local optical_centre="Where something looks centred versus where it mathematically is. A play button centred by coordinates looks left-heavy. Nudge it right and it sits."
  local stroke_weight="How thick the lines of an icon are. Thin strokes disappear at small sizes. At large sizes they look frail. Weight needs to scale with size."
  local tabular_nums="A font feature that gives every digit the same width so numbers in columns stay aligned as they change. Essential for prices, stats, and any data that updates."

  case "$id" in
    c1)
      if has_exact "$response" "$leading" || has_exact "$response" "$tracking"; then check true; else check false; fi
      if has_exact "$response" "$negative_space" || has_exact "$response" "$gap"; then check true; else check false; fi
      check grep -Eiq 'continue|carry on|build' <<<"$response"
      if has_exact "$response" "$tracking"; then check grep -Eiq 'kerning|specific (characters|character).*uniform|uniform.*specific (characters|character)' <<<"$response"; fi
      ;;
    c2)
      local term_count=0
      for definition in "$contrast" "$weight" "$saturation" "$chroma"; do
        has_exact "$response" "$definition" && term_count=$(( term_count + 1 ))
      done
      check test "$term_count" -ge 2
      check test "$term_count" -le 3
      if has_exact "$response" "$saturation" || has_exact "$response" "$chroma"; then
        check grep -Eiq 'chroma.*saturation|saturation.*chroma|OKLCH.*percept' <<<"$response"
      fi
      ;;
    c3)
      check has_exact "$response" "$layout_shift"
      check has_exact "$response" "$font_stack"
      ;;
    c4)
      check has_exact "$response" "$leading"
      check test "$(print -r -- "$response" | wc -l | tr -d ' ')" -le 4
      ;;
    c5)
      check grep -Eiq 'motion as feedback.*(not in|missing from|isn.t in).*bundled|indexed.*not.*bundled' <<<"$response"
      check grep -Eiq 'Material Design' <<<"$response"
      if has_exact "$response" "$hover_state" || has_exact "$response" "$optimistic_update"; then check true; else check false; fi
      ;;
    c6)
      local icon_count=0
      has_exact "$response" "$optical_centre" && icon_count=$(( icon_count + 1 ))
      has_exact "$response" "$stroke_weight" && icon_count=$(( icon_count + 1 ))
      if grep -Eiq 'unified weight.*(not in|missing from|isn.t in).*bundled|unified weight.*index-only' <<<"$response"; then icon_count=$(( icon_count + 1 )); fi
      check test "$icon_count" -ge 2
      check grep -Eiq 'optical centre.*(align|cent)|stroke weight.*(line|heavy|light|thick)' <<<"$response"
      ;;
    c7)
      check has_exact "$response" "$tabular_nums"
      ;;
    *) print -u2 -r -- "unknown case: $id"; exit 2 ;;
  esac

  score=$(( checks_passed * 10 / checks_total ))
  if (( checks_passed != checks_total )); then
    failure_mode=deterministic-term-check
  fi
}

grade_actual_output() {
  local input=$1
  local expect=$2
  local actual=$3
  local prompt verdict
  prompt=$(printf '%s\n\n%s\n%s\n\n%s\n%s\n\n%s\n%s\n\n%s\n%s\n' \
    'Grade only the actual response that a candidate produced while it executed the loaded vocabulary skill. Reply with only JSON: {"score":<integer 0-10>,"failure_mode":<string or null>}.' \
    'RUBRIC:' "$(<"$rubric_file")" \
    'CASE INPUT:' "$input" \
    'EXPECT:' "$expect" \
    'ACTUAL CANDIDATE RESPONSE:' "$actual")
  verdict=$("$skill_eval" judge --prompt "$prompt") || return $?
  print -r -- "$verdict" | jq -ce 'select((.score | type == "number") and (.score % 1 == 0) and (.score >= 0) and (.score <= 10) and (.failure_mode == null or (.failure_mode | type == "string")))'
}

run_arm() {
  local arm=$1
  local loaded_skill=$2
  local runner=$candidate_runner
  [[ "$is_smoke" == false ]] || runner=$normal_fake_runner
  integer total_cases=0
  integer total_score=0
  local case_json id input expect source fixture workspace runner_status response verdict judge_score judge_failure output

  while IFS= read -r case_json; do
    id=$(jq -r '.id' <<<$case_json)
    input=$(jq -r '.input' <<<$case_json)
    expect=$(jq -r '.expect' <<<$case_json)
    source=$(jq -r '.source' <<<$case_json)
    fixture=$(jq -r '.execution.drive.source' <<<$case_json)
    workspace="$temporary_root/workspaces/$arm-$id"
    prepare_workspace "$workspace" "$loaded_skill" "$fixture" "$is_smoke"

    if run_candidate "$runner" "$id" "$input" "$workspace"; then
      response=${candidate_output%$'\n'}
      if [[ -z "$response" ]]; then
        score=0
        checks_passed=0
        checks_total=1
        failure_mode=no-response
      else
        verify_response "$id" "$response"
        if [[ "$is_smoke" == false ]]; then
          verdict=$(grade_actual_output "$input" "$expect" "$response") || { print -u2 -r -- "shared skill-eval judge failed for $arm/$id"; exit 1; }
          judge_score=$(jq -r '.score' <<<$verdict)
          judge_failure=$(jq -r '.failure_mode // ""' <<<$verdict)
          if (( judge_score < score )); then
            score=$judge_score
            failure_mode=${judge_failure:-actual-output}
          fi
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

    output=$(jq -cn --arg arm "$arm" --arg id "$id" --arg source "$source" --arg fixture "$fixture" --argjson score "$score" --arg failure_mode "$failure_mode" --argjson checks_passed "$checks_passed" --argjson checks_total "$checks_total" '{arm:$arm,id:$id,source:$source,fixture:$fixture,drive:"response",score:$score,failure_mode:(if $failure_mode == "" then null else $failure_mode end),checks_passed:$checks_passed,checks_total:$checks_total}')
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
