#!/bin/zsh
set -euo pipefail

workspace=${AGENT_AUTHOR_EVAL_WORKSPACE:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
for contained_path in "$HOME" "$PI_CODING_AGENT_DIR" "$PI_CONFIG_DIR" "$PI_CODING_AGENT_SESSION_DIR" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$TMPDIR"; do
  contained_path=${contained_path:A}
  [[ "$contained_path" == "${workspace:A}"/* ]]
done

args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve'; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --session-dir $workspace/.pi/session "* ]]
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]]
[[ ! -e "$workspace/cases.jsonl" && ! -e "$workspace/rubric.md" && ! -e "$workspace/evals" ]]

make_scaffold() {
  local name=$1
  local tier=$2
  local extra=$3
  local root="$workspace/agents/$name"
  mkdir -p "$root/evals" "$root/logs" "$root/votes"
  print -r -- "---
name: $name
description: Use when the named bounded role is required. Skip when the request is outside this role.
tools: read
---

# $name

## Input contract

Accept the required named input only.

## Output contract

Return the fixed result object only.

## Context discipline

Exclude ambient context and all unrelated transcripts.

## Trigger conditions

Run only when every required input exists.

## Success rubric

The output has every required field and no extra field.

## Failure-mode watch-list

Stop when an input is missing.

$extra

## logging

Append one bounded usage record." > "$root/$name.md"
  print -r -- '# Harsh rubric' > "$root/evals/rubric.md"
  print -r -- '#!/bin/zsh' > "$root/evals/run.sh"
  chmod +x "$root/evals/run.sh"
  print -r -- '{"input":"valid","holdout":false}
{"input":"missing required input","holdout":false}
{"input":"outside role; decline","holdout":false}
{"input":"valid second","holdout":false}
{"input":"held out","holdout":true}' > "$root/evals/cases.jsonl"
  : > "$root/logs/usage.jsonl"
  : > "$root/votes/votes.jsonl"
  jq --arg name "$name" --arg tier "$tier" '.agents[$name] = $tier' "$workspace/config/model-tiers.json" > "$workspace/config/model-tiers.next"
  mv "$workspace/config/model-tiers.next" "$workspace/config/model-tiers.json"
}

case "${workspace:t}" in
  a1-contract-checker)
    make_scaffold dependency-contract-checker T4 'The manifest_path and dependency_name inputs produce verdict, reason, and anchor fields.'
    ;;
  a2-json-normalizer)
    make_scaffold json-normalizer T2 'The record input produces labels in one JavaScript Object Notation object.'
    perl -0pi -e 's/tools: read\n//' "$workspace/agents/json-normalizer/json-normalizer.md"
    ;;
  a3-release-checklist)
    print -r -- '{"verdict":"skill","reason":"The request is one linear recipe."}' > "$workspace/decision.json"
    ;;
  a4-missing-dispatch-input)
    print -r -- '{"verdict":"invalid-dispatch","missing":"source_path"}' > "$workspace/dispatch-gap.json"
    ;;
  a5-fresh-reviewer)
    make_scaffold change-finding-reviewer T4 'The finding and source_path inputs exclude the builder transcript and every prior verdict.'
    ;;
  h1-fixture-curator)
    make_scaffold fixture-curator T3 'The fixture_root and case_id inputs produce changed_paths. Write only below fixture_root.'
    ;;
  *)
    exit 64
    ;;
esac

print -r -- '{"type":"result","status":"complete"}'
