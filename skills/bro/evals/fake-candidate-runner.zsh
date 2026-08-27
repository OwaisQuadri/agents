#!/bin/zsh
set -euo pipefail

prompt=${argv[-1]}
input=${prompt##*$'TASK:\n'}
[[ "$(print -rn -- "$input" | shasum -a 256 | cut -d ' ' -f 1)" == "$BRO_EVAL_CASE_INPUT_SHA" ]]

skill_path=""
while (( $# > 0 )); do
  case "$1" in
    --skill)
      skill_path=${2:?missing skill path}
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

[[ -n "$skill_path" && -f "$skill_path" ]]
[[ "$skill_path" == "$BRO_EVAL_WORKSPACE"/* ]]
[[ "$(shasum -a 256 "$skill_path" | cut -d ' ' -f 1)" == "$BRO_EVAL_EXPECTED_SKILL_SHA" ]]

case "$BRO_EVAL_CASE_ID" in
  r1) print -r -- "Running the same batch again changes nothing. Response time stayed at 21ms in bench/resolve.ts." ;;
  r2) print -r -- "The system checks the list of steps before the run. Each step waits for the steps that must finish before it." ;;
  r3) print -r -- "Parsing took 340ms before and 210ms after. Writing took 88ms before and 84ms after." ;;
  r4) print -r -- "Two tests failed after the bump. They are test_retry_window and test_backoff_cap." ;;
  r5) print -r -- "This change adds caching for the rollout." ;;
  r6) print -r -- "Which of the three checks before merging do you want handled first?" ;;
  r7) print -r -- "There is nothing to simplify yet." ;;
  r8) print -r -- 'O deploy falhou porque o token expirou. Rode `vercel env pull` antes de tentar de novo.' ;;
  r9) print -r -- "The scheduler allows at most 8 jobs during each time period. It checks new jobs before it resets the limit. A group at the edge between two periods can exceed 8." ;;
  r10) print -r -- 'Set `--deterministic` on the run because the serializer can give different results by default.' ;;
  r11) print -r -- "The rollback reversed the database change, restored the old way to read data, and turned the scheduled job on again." ;;
  r12) print -r -- 'Step 3 of 5 finished. The schema changed. The indexes are pending. Run `make migrate` and paste the output.' ;;
  r13) print -r -- "The two runs show different middle values. I could not tell which one is right." ;;
  *) print -u2 -r -- "unknown fake case: $BRO_EVAL_CASE_ID"; exit 2 ;;
esac
