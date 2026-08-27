#!/bin/zsh
set -euo pipefail

workspace=${TASK_GRAPH_EVAL_WORKSPACE:?}
id=${TASK_GRAPH_EVAL_CASE_ID:?}
expected_skill_sha=${TASK_GRAPH_EVAL_EXPECTED_SKILL_SHA:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
[[ "$(shasum -a 256 "$workspace/.candidate/task-graph/SKILL.md" | cut -d ' ' -f 1)" == "$expected_skill_sha" ]]
args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve'; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/task-graph/SKILL.md "* ]]
for hidden in "$TASK_GRAPH_EVAL_HIDDEN_RUBRIC" "$TASK_GRAPH_EVAL_HIDDEN_CASES" "$TASK_GRAPH_EVAL_HIDDEN_HOLDOUT" "$TASK_GRAPH_EVAL_HIDDEN_SOURCE" "$TASK_GRAPH_EVAL_HIDDEN_HOME"; do
  if /bin/cat "$hidden" >/dev/null 2>&1; then
    exit 81
  fi
done

created='2031-04-10T12:00:00-0400'
case "$id" in
  t1)
    jq --arg created "$created" '{ticket:.ticket,tasks:[.tasks | to_entries[] | {id:("MJLS-0042.T" + ((.key + 1) | tostring | if length == 1 then "0" + . else . end)),short:.value.short,long:.value.long,deps:[.value.deps[] | "MJLS-0042.T" + (tostring | if length == 1 then "0" + . else . end)],status:"todo",files:.value.files,blame_phase:null,created:$created,kind:"code"}]}' request.json > tasks.json
    report='Created MJLS-0042.T01 through MJLS-0042.T06 in creation order. The left and right branches share wave 2. All six statuses are todo. tasks.json validates.'
    ;;
  t2)
    report='Rejected the graph. Cycle path: MJLS-0042.T01 -> MJLS-0042.T02 -> MJLS-0042.T01. I did not overwrite tasks.json or tasks.mmd.'
    ;;
  t3)
    jq --arg created "$created" '.tickets += [{id:"MJLS-0018",short:"first",long:"File first.",deps:[],status:"todo",files:["src/first"],blame_phase:null,created:$created,kind:"ticket"},{id:"MJLS-0019",short:"second",long:"File second.",deps:[],status:"todo",files:["src/second"],blame_phase:null,created:$created,kind:"ticket"},{id:"MJLS-0020",short:"third",long:"File third.",deps:[],status:"todo",files:["src/third"],blame_phase:null,created:$created,kind:"ticket"}] | .next_nnnn = 21' roadmap.json > roadmap.json.new
    mv roadmap.json.new roadmap.json
    report='Filed MJLS-0018 through MJLS-0020 in creation order. The counter is 21. Existing identifier MJLS-0017 is unchanged. Reverse edges: none for each new ticket.'
    ;;
  t4)
    jq --arg created "$created" '{ticket:.ticket,tasks:[{id:"MJLS-0042.T01",short:.tasks[0].short,long:.tasks[0].long,deps:[],status:"todo",files:.tasks[0].files,blame_phase:null,created:$created,kind:"code"},{id:"MJLS-0042.T02",short:.tasks[1].short,long:.tasks[1].long,deps:["MJLS-0042.T01"],status:"todo",files:.tasks[1].files,blame_phase:null,created:$created,kind:"code"}]}' request.json > tasks.json
    report='Added MJLS-0042.T01 -> MJLS-0042.T02 because both tasks own src/store.ts. They are in separate waves and are not parallelizable.'
    ;;
  t5)
    jq '(.tickets[] | select(.id == "MJLS-0001") | .status) = "done"' roadmap.json > roadmap.json.new
    mv roadmap.json.new roadmap.json
    report='Marked MJLS-0001 done. Refused renumbering because identifiers are immutable after assignment. MJLS-0003 and MJLS-0005 remain unchanged.'
    ;;
  t6)
    jq --arg created "$created" '.tickets += [{id:"MJLS-0012",short:"first",long:"File first.",deps:[],status:"todo",files:["src/first"],blame_phase:null,created:$created,kind:"ticket"},{id:"MJLS-0013",short:"second",long:"File second.",deps:[],status:"todo",files:["src/second"],blame_phase:null,created:$created,kind:"ticket"},{id:"MJLS-0014",short:"third",long:"File third.",deps:[],status:"todo",files:["src/third"],blame_phase:null,created:$created,kind:"ticket"}] | .next_nnnn = 15' roadmap.json > roadmap.json.new
    mv roadmap.json.new roadmap.json
    report='Preserved cancelled MJLS-0011 and did not reuse it. Filed MJLS-0012 through MJLS-0014. The counter is maximum NNNN plus one at 15.'
    ;;
  t7)
    jq --arg created "$created" '(.tickets[] | select(.id == "MJLS-0012" or .id == "MJLS-0013" or .id == "MJLS-0014") | .deps) = ["MJLS-0015"] | .tickets += [{id:"MJLS-0015",short:"extract auth token",long:"Extract the shared auth-token module.",deps:[],status:"todo",files:["src/auth-token"],blame_phase:null,created:$created,kind:"ticket"}] | .next_nnnn = 16' roadmap.json > roadmap.json.new
    mv roadmap.json.new roadmap.json
    report='Existing-depends-on-new reverse edges: MJLS-0012 -> MJLS-0015, MJLS-0013 -> MJLS-0015, and MJLS-0014 -> MJLS-0015. MJLS-0015 has no dependencies and unlocks 3. next-ticket.sh selects MJLS-0015.'
    ;;
  t8)
    jq --arg created "$created" '.tickets += [{id:"MJLS-0006",short:"six",long:"Independent six.",deps:[],status:"todo",files:["src/six"],blame_phase:null,created:$created,kind:"ticket"},{id:"MJLS-0007",short:"seven",long:"Independent seven.",deps:[],status:"todo",files:["src/seven"],blame_phase:null,created:$created,kind:"ticket"}] | .next_nnnn = 8' roadmap.json > roadmap.json.new
    mv roadmap.json.new roadmap.json
    report='Checked the existing-depends-on-new direction. Reverse edges: none for MJLS-0006. Reverse edges: none for MJLS-0007. Existing dependencies remain unchanged.'
    ;;
  t9)
    pick=$($workspace/.candidate/task-graph/scripts/next-ticket.sh roadmap.json 2>selection.txt)
    [[ "$pick" == MJLS-0022 ]]
    report='Selected MJLS-0022. High priority ranks before unlock count, so it beats low MJLS-0021 despite 0 versus 2 unlocks. Missing MJLS-0023 and unknown MJLS-0024 both rank as med with 1 unlock; MJLS-0023 wins their identifier tie. No dependency changed.'
    ;;
  *) exit 64 ;;
esac

print -r -- "{\"type\":\"result\",\"status\":\"complete\",\"text\":$(jq -Rn --arg text "$report" '$text')}"
