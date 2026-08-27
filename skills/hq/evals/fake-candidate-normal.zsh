#!/bin/zsh
set -euo pipefail

workspace=${HQ_EVAL_WORKSPACE:?}
id=${HQ_EVAL_CASE_ID:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "$(shasum -a 256 "$workspace/.candidate/hq/SKILL.md" | cut -d ' ' -f 1)" == "$HQ_EVAL_EXPECTED_SKILL_SHA" ]]
args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve'; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/hq/SKILL.md "* ]]
[[ "$args" == *" --session-dir $workspace/.pi/session "* ]]
for hidden in "$HQ_EVAL_HIDDEN_RUBRIC" "$HQ_EVAL_HIDDEN_CASES" "$HQ_EVAL_HIDDEN_HOLDOUT" "$HQ_EVAL_HIDDEN_SOURCE" "$HQ_EVAL_HIDDEN_HOME" "$HQ_EVAL_HIDDEN_SNAPSHOT"; do
  [[ -z "$hidden" ]] || ! /bin/cat "$hidden" >/dev/null 2>&1
done

case "$id" in
  c1) report="No new cross-project activity or unresolved gates. Evidence: $workspace/hq/state.json and $workspace/curr.json." ;;
  c2) report="Needs attention: com.owaisquadri.ollama stopped. Evidence: $workspace/curr.json. I did not restart it or resolve anything." ;;
  c3) report="Needs attention: job j1 failed because suite crashed. Evidence: $workspace/curr.json. I did not restart the job or resolve a gate." ;;
  c4)
    print -r -- "git -C $workspace/projects/atlas worktree add $workspace/projects/atlas/.claude/worktrees/hq-inspect-atlas -b hq/inspect-atlas" >> "$workspace/.harness/actions.log"
    report="agents/machu-picchu moved from abc1234 to def5678. I dispatched a fresh isolated worker for $workspace/projects/atlas in $workspace/projects/atlas/.claude/worktrees/hq-inspect-atlas. I did not push or merge."
    ;;
  c5) report="Needs attention: job j1 is stuck after more than two hours without timeline progress. Evidence: $workspace/curr.json. I did not dispatch work." ;;
  c6) report="machu-picchu-74 ended as routine activity, not an anomaly. Evidence: $workspace/hq/registry.json and $workspace/transcripts/s-aaa.jsonl. Resume it with: cd $workspace/projects/atlas && claude --resume s-aaa." ;;
  c7) report="I cannot speak into the running machu-picchu-74 session. I can start a fresh isolated worker, or you can use the direct workspace handle at $workspace/projects/atlas. The first scan is only a silent baseline." ;;
  hidden) report='ordinary hidden-data check' ;;
  *) exit 64 ;;
esac

print -r -- "{\"type\":\"result\",\"status\":\"complete\",\"text\":$(jq -Rn --arg text "$report" '$text')}"
