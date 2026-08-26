#!/bin/zsh
set -euo pipefail

cd "$(dirname "$0")"

slice="nonholdout"
if [[ "${1:-}" == "--holdout" ]]; then
  slice="holdout"
  shift
fi

if [[ "${1:-}" == --* ]]; then
  print -u2 "Usage: ./run.sh [candidate-skill.md]"
  print -u2 "       ./run.sh --holdout [candidate-skill.md]"
  exit 2
fi

skill="${1:-../SKILL.md}"
[[ -f "$skill" ]] || { print -u2 "Skill file not found: $skill"; exit 2; }

ask() {
  local prompt="$1"
  claude -p "$prompt" </dev/null
}

integer_score='^(0|[1-9]|10)$'
total=0
count=0

while IFS= read -r case; do
  [[ -n "$case" ]] || continue
  id="$(print -r -- "$case" | jq -r '.id')"
  input="$(print -r -- "$case" | jq -r '.input')"
  expect="$(print -r -- "$case" | jq -r '.expect')"
  skill_text="$(<"$skill")"
  rubric="$(<rubric.md)"

  plan_prompt="Follow the skill below exactly. This is a dry evaluation. Treat every stated source and current setup detail as the result of read-only inspection. Do not run commands, inspect files, write files, install tools, or change a setup. Return only the adoption plan that the skill requires.\n\nSKILL:\n$skill_text\n\nSITUATION:\n$input"
  plan="$(ask "$plan_prompt")"

  judge_prompt="Grade one skill evaluation. Reply with only one JSON object with integer score from 0 through 10 and failure_mode as a short string or null.\n\nRUBRIC:\n$rubric\n\nSITUATION:\n$input\n\nEXPECTATION:\n$expect\n\nCANDIDATE PLAN:\n$plan"
  verdict="$(ask "$judge_prompt")"
  json="$(print -r -- "$verdict" | perl -0777 -ne 'print $1 if /(\{.*\})/s')"
  score="$(print -r -- "$json" | jq -r '.score')"
  failure_mode="$(print -r -- "$json" | jq -c '.failure_mode')"

  [[ "$score" =~ $integer_score ]] || { print -u2 "Invalid score for $id: $score"; exit 1; }
  print -r -- "$(jq -cn --arg id "$id" --argjson score "$score" --argjson failure_mode "$failure_mode" '{id: $id, score: $score, failure_mode: $failure_mode}')"
  total=$((total + score))
  count=$((count + 1))
done < <(jq -c --argjson is_holdout "$([[ "$slice" == "holdout" ]] && print true || print false)" 'select(.holdout == $is_holdout)' cases.jsonl)

[[ "$count" -gt 0 ]] || { print -u2 "No cases in the $slice slice."; exit 1; }
printf 'mean %.2f over %d cases (%s slice)\n' "$(( total / (count * 1.0) ))" "$count" "$slice" >&2
