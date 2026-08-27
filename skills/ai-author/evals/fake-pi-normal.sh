#!/bin/zsh
set -euo pipefail
workspace=${AI_AUTHOR_EVAL_WORKSPACE:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
for contained_path in "$HOME" "$PI_CODING_AGENT_DIR" "$PI_CONFIG_DIR" "$PI_CODING_AGENT_SESSION_DIR" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$TMPDIR"; do
  contained_path=${contained_path:A}
  [[ "$contained_path" == "${workspace:A}"/* ]]
done
args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve'; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]]
[[ "$args" != *'ai-author rubric'* && "$args" != *'Case expectation:'* && "$args" != *'SENTINEL-PRIOR-VOTE'* ]]
grep -q '^# ai-author$' "$workspace/.candidate/SKILL.md"
[[ ! -e "$workspace/cases.jsonl" && ! -e "$workspace/rubric.md" && ! -e "$workspace/votes" ]]

make_contract() {
  local root=$1
  local definition=$2
  local body=$3
  mkdir -p "$root/evals" "$root/logs" "$root/votes"
  print -r -- "$body

## logging" > "$definition"
  print -r -- '{"id":"n1","holdout":false}
{"id":"n2","holdout":false}
{"id":"n3","holdout":false}
{"id":"n4","holdout":false}
{"id":"h1","holdout":true}' > "$root/evals/cases.jsonl"
  print -r -- '# Rubric' > "$root/evals/rubric.md"
  print -r -- '#!/bin/zsh' > "$root/evals/run.sh"
  chmod +x "$root/evals/run.sh"
  : > "$root/logs/usage.jsonl"
  : > "$root/votes/votes.jsonl"
}

case "${workspace:t}" in
  *-a4)
    print -r -- '{"task":"mobile-testing-research","tools":["Appium","Maestro","Mobile Next"],"summary":"The owned catalog contains three useful mobile-testing tools."}' > "$workspace/research-results.json"
    ;;
  *-b1)
    make_contract "$workspace/skills/create-pr" "$workspace/skills/create-pr/SKILL.md" '# create-pr

Rule 3 applies because one agent follows this linear recipe.'
    ;;
  *-b2)
    make_contract "$workspace/agents/diff-reviewer" "$workspace/agents/diff-reviewer/diff-reviewer.md" '# diff-reviewer

Use a fresh context and read-only tools. Return ranked findings by severity.'
    ;;
  *-b3)
    make_contract "$workspace/workflows/research-sweep" "$workspace/workflows/research-sweep/research-sweep.workflow.js" '# research-sweep

Fan out five researchers in parallel. A skeptic judge grades each result. Synthesize the survivors.'
    ;;
  *-c1)
    "$workspace/skills/create-pr/evals/run.sh" > "$workspace/.harness/nonholdout.out" 2> "$workspace/.harness/nonholdout.err"
    "$workspace/skills/create-pr/evals/run.sh" --holdout > "$workspace/.harness/holdout.out" 2> "$workspace/.harness/holdout.err"
    print -r -- 'Per-case score: n1 scored 10. Non-holdout passed 1 of 1. Holdout scored 10 and passed. The vote file exists at votes/votes.jsonl. SKILL.md ends in logging.' > "$workspace/RESULT.md"
    ;;
esac

print -r -- '{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"The loaded ai-author skill was executed on this case."}],"provider":"fake","model":"fake-candidate","responseModel":"fake-candidate","usage":{"input":1,"output":1},"stopReason":"stop"}}'
