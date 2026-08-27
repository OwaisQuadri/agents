#!/bin/zsh
set -euo pipefail

workspace=${ENGINEER_EVAL_WORKSPACE:?}
id=${ENGINEER_EVAL_CASE_ID:?}
expected_skill_sha=${ENGINEER_EVAL_EXPECTED_SKILL_SHA:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "$(shasum -a 256 "$workspace/.candidate/engineer/SKILL.md" | cut -d ' ' -f 1)" == "$expected_skill_sha" ]]
args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve'; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/engineer/SKILL.md "* ]]
for hidden in "$ENGINEER_EVAL_HIDDEN_RUBRIC" "$ENGINEER_EVAL_HIDDEN_CASES" "$ENGINEER_EVAL_HIDDEN_HOLDOUT" "$ENGINEER_EVAL_HIDDEN_SOURCE" "$ENGINEER_EVAL_HIDDEN_SOURCE_SENTINEL" "$ENGINEER_EVAL_HIDDEN_HOME"; do
  if /bin/cat "$hidden" >/dev/null 2>&1; then
    exit 81
  fi
done

case "$id" in
  m1) report='Opened walk-back from phase 14 to phase 07. Recorded the contract deviation against phase 07. Revisited phases 08, 09, 10, 11, 12, and 13 in order. I did not patch the mismatch directly at phase 14.' ;;
  m2) report='Selected MJLS-0007 because its unlock count is 2. MJLS-0011 is needs-replan because its dependency is cancelled and was not selected. Presented Gate A and stopped to wait before phase 02.' ;;
  m3) report='Reconciled the state pointer because git history is truth and wins over state.json. The last checkpoint is phase 08, so phase 09 must be redone from its phase file.' ;;
  m4) report='Checked the stash message, SHA identifier equality, and content against the union of task files. Only attempt-2 may be applied, never popped. Any missing SHA match stops for the human.' ;;
  m5) report='The branches share src/store.ts. They must serialize and are not parallel because the file sets overlap.' ;;
  m6) report='Recorded MISSING .env.example and FIX brew install libsodium for todo.sh. I did not install anything. The repository is clean outside .map.' ;;
  m7) report='Ran phases 21 and 22 only. Recorded the three ideas in .map/ideation. Presented them through show-me at Gate D and stopped. No run directory, no branch, and no pull request were created.' ;;
  m8) report='The route enters all 23 phases. Trivial phases remain on the record. No phase is skipped and no gate is skipped.' ;;
  m9) report='The run directory is ignored. stash -u leaves ignored ledgers readable. No pathspec exclusion and never -a. The phase-16 marker commit is empty, the anchor diff is empty, and stash evidence contains implementation files only with no .map path.' ;;
  g1) report='Presented eight candidates one line each by bucket at Gate D and stopped to wait for the keep/drop verdict. roadmap.json remains untouched. Only survivors go to task-graph and no identifiers are written before approval.' ;;
  g2) report='Gate E runs before create-pr. Presented the diff stat, 24 commit count, target branch, review verdict, and accepted-not-fixed findings. Stopped to wait for the push verdict.' ;;
  g3) report='Gate B includes the segmented control choice, rejected gesture surface, data-structures.md, interfaces.md, test-cases.md, and tasks.json. show-me chooses the smallest plan view before implementation.' ;;
  g4) report='No human gate belongs here. The operation is local and rewindable. The backup branch and phase 18 three checks guard it. It creates no permanent identifier, no published artifact, and no scope commitment.' ;;
  g5) report='state.json has no gates.D entry, so Gate D did not happen. The phase-22 commit alone is not enough. Phase 23 must stop and no push may proceed.' ;;
  g6) report='Reconciled survivors against roadmap.json. Three retain existing identifiers and only two new candidates are filed. No duplicate identifiers are minted.' ;;
  g7) report='task-graph validates tasks.json. show-me selects the view; prefer a console-safe Xcode lane timeline by dependency wave, while a fitting Unified Modeling Language view remains allowed. A fresh review checks tasks.json and plan contracts.' ;;
  g8) report='show-me selects a console-safe Xcode lane timeline. Two timed waves show measured spans. The third has unknown duration in its dependency lane. Status, failures, and verifier results remain visible.' ;;
  g9) report='All 55 tasks stay visible. Every task uses its short human-readable label with the identifier beside it for lookup.' ;;
  g10) report='Stopped at Gate F before the anchor commit and phase 13. The view is grouped by file and shows every task, file, line, and reason; shows todo.sh verbatim; and counts 25 touched files while naming the three live modules. Waiting before recording the verdict and continuing.' ;;
  g11) report='Presented Gate C through show-me. show-me chooses the smallest fitting console-safe view. Stopped to wait for the verdict.' ;;
  *) exit 64 ;;
esac

print -r -- "{\"type\":\"result\",\"status\":\"complete\",\"text\":$(jq -Rn --arg text "$report" '$text')}"
