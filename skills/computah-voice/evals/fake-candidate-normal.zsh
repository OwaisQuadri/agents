#!/bin/zsh
set -euo pipefail

workspace=${COMPUTAH_VOICE_EVAL_WORKSPACE:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$COMPUTAH_VOICE_EVAL_EXPECTED_SKILL_SHA" ]]
args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve' '--no-tools'; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]]
[[ "$args" == *" --session-dir $workspace/.pi/session "* ]]
for hidden_path in "$COMPUTAH_VOICE_EVAL_HIDDEN_RUBRIC" "$COMPUTAH_VOICE_EVAL_HIDDEN_CASES" "$COMPUTAH_VOICE_EVAL_HIDDEN_SOURCE" "$COMPUTAH_VOICE_EVAL_HIDDEN_HOME" "$COMPUTAH_VOICE_EVAL_HIDDEN_SNAPSHOT"; do
  [[ -z "$hidden_path" ]] || ! /bin/cat "$hidden_path" >/dev/null 2>&1
done

case "$COMPUTAH_VOICE_EVAL_CASE_ID" in
  c1) print -r -- 'The retrieval augmented generation merge finished cleanly as a fast-forward. Main is now at commit one e one f nine five five.' ;;
  c2) print -r -- 'Two tests failed after the patience setting increased: test retries after timeout and test stops at deadline.' ;;
  c3) print -r -- 'Which merge gate should I handle first, security, continuous integration, or review?' ;;
  c4) print -r -- 'Origin main contains two committed live credentials. They appear in config deploy dot env, line four, and config c i dot env, line nine.' ;;
  c5) print -r -- 'The worktree cleanup finished, and nothing else is pending.' ;;
  c6) print -r -- 'The merge fast-forwarded main from commit a one two b c three four to commit d five six e f seven eight. It added the retry timeout change and deleted no files.' ;;
  c7) print -r -- "Yeah, I'll look through every workspace." ;;
  hidden) print -r -- 'Yeah, I can handle that.' ;;
  *) exit 64 ;;
esac
