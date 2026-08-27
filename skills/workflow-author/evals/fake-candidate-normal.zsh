#!/bin/zsh
set -euo pipefail

workspace=${WORKFLOW_AUTHOR_EVAL_WORKSPACE:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
for contained_path in "$HOME" "$PI_CODING_AGENT_DIR" "$PI_CONFIG_DIR" "$PI_CODING_AGENT_SESSION_DIR" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$TMPDIR"; do
  contained_path=${contained_path:A}
  [[ "$contained_path" == "${workspace:A}"/* ]]
done
args=" $* "
for fence in --no-session --no-skills --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]]
[[ "$args" == *" --model fake/candidate "* ]]
[[ "$args" == *" --tools read,write,edit "* ]]
[[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d ' ' -f 1)" == "$WORKFLOW_AUTHOR_EVAL_EXPECTED_SKILL_SHA" ]]
for hidden_path in "$WORKFLOW_AUTHOR_EVAL_HIDDEN_RUBRIC" "$WORKFLOW_AUTHOR_EVAL_HIDDEN_CASES" "$WORKFLOW_AUTHOR_EVAL_HIDDEN_HOLDOUT" "$WORKFLOW_AUTHOR_EVAL_HIDDEN_SOURCE" "$WORKFLOW_AUTHOR_EVAL_HIDDEN_HOME" "$WORKFLOW_AUTHOR_EVAL_HIDDEN_SNAPSHOT"; do
  if [[ -n "$hidden_path" ]] && payload=$(<"$hidden_path" 2>/dev/null); then
    print -u2 -r -- "$payload"
    exit 70
  fi
done
[[ ! -e "$workspace/cases.jsonl" && ! -e "$workspace/rubric.md" && ! -e "$workspace/holdout-sentinel.txt" ]]

case "$WORKFLOW_AUTHOR_EVAL_CASE_ID" in
  w1-route-audit)
    print -r -- 'workflow

GOAL: Audit 12 route files for missing authentication checks.
FAN OUT: Run one worker per route file in parallel. Each worker returns {route_path, authentication_result, test_evidence}.
VERIFY: A fresh-context checker verifies each finding against the route file and executed test result.
CAP: Stop the first run at 12 files.
ON FAIL: Require 12 returns. Flag every missing route path. Do not synthesize a partial set.
ANCHOR: The route authentication test ran and passed or failed.
REPORT: Return all verified findings and the worker return count.' > "$workspace/route-audit.workflow.md"
    ;;
  w2-fake-edge-rewrite)
    print -r -- 'workflow

PRESERVE-RELEASE-WORKFLOW-42

GOAL: Review the API, web, and worker packages before release.
PARALLEL JOBS (package reviews, run at once): Review each package. Return {package, findings, test_result}.
MERGE: Consume all three review records and write one report.
VERIFY: A fresh-context checker verifies each package finding.
CAP: Stop the first run at three packages.
ON FAIL: Require three returns. Flag each missing package. Do not merge a partial set.
SAVE: release-review.md' > "$workspace/release-review.workflow.md"
    ;;
  w3-wide-pricing)
    print -r -- 'workflow

GOAL: Compare 60 vendor pricing pages.
FAN OUT: Run one worker per vendor in parallel. Return {vendor, plan, price, source_link, source_date}.
DEDUPE: Plain code removes duplicate records before verification.
VERIFY: A fresh-context checker resolves each source link and verifies each record.
MERGE: Batch verified records into groups of 20. Merge three batch summaries before final synthesis.
CAP: Stop the first run at 60 vendors.
ON FAIL: Require 60 returns. Flag each missing vendor. Do not synthesize a partial set.
ANCHOR: Each retained source link resolves.
REPORT: Return the comparison and worker counts for each batch.' > "$workspace/pricing-research.workflow.md"
    ;;
  w4-shared-changelog)
    print -r -- 'workflow

GOAL: Create one changelog from four package histories.
PARALLEL JOBS (package reads, run at once): Inspect one package history. Return {package, changes, test_result}. Do not write a shared file.
VERIFY: A fresh-context checker verifies each record against the package history and release test.
MERGE: Require all four records. One merge job writes drafts/changelog.md after verification.
CAP: Stop the first run at four packages.
ON FAIL: Flag each missing package. Do not write drafts/changelog.md from a partial set.
ANCHOR: The release test ran and passed.
SAVE: drafts/changelog.md' > "$workspace/changelog.workflow.md"
    ;;
  w5-ai-author-fence)
    print -r -- '{"verdict":"route-to-ai-author","reason":"The request does not approve an artifact type."}' > "$workspace/decision.json"
    ;;
  h1-partial-fanin)
    print -r -- 'workflow

PRESERVE-DEPENDENCY-AUDIT-73

GOAL: Audit eight packages for undeclared dependencies.
FAN OUT: Run one worker per package. Return {package, finding, dependency_test_result}.
VERIFY: A fresh-context checker verifies each finding against the package and dependency test.
MERGE: Require all eight returns before synthesis.
CAP: Stop the first run at eight packages.
ON FAIL: Flag each missing package name. Do not synthesize a partial set.
ANCHOR: The dependency test ran and passed.
SAVE: dependency-audit.md' > "$workspace/dependency-audit.workflow.md"
    ;;
  *) exit 64 ;;
esac

print -r -- '{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Workflow-author case complete."}],"provider":"fake","model":"fake/candidate","responseModel":"fake/candidate","usage":{"input":1,"output":1},"stopReason":"stop"}}'
