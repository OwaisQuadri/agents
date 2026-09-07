#!/bin/zsh
set -euo pipefail
here=${0:A:h}
candidate=${1:-}
incumbent=$here/../pr-ready.workflow.js
if [[ -z $candidate || ${candidate:t} == SKILL.md || ${candidate:e} == md ]]; then
  definition=$incumbent
else
  definition=$candidate
fi
[[ -r $definition ]] || { print -u2 "workflow not found: $definition"; exit 1; }
has() {
  rg -q "$1" "$definition"
}

has 'missing input: repo_path' && has 'if \(!repo_path\) return' || { print -u2 'repo_path guard missing'; exit 1; }
has 'T6_PRIMARY' && has 'T6_FALLBACK' && has 'T5_CHAIN' || { print -u2 'tier chains missing' ; exit 1; }
has 'if \(!ready.ready\)' || { print -u2 'ready-gate missing: review/triage must not run past a blocker'; exit 1; }
if rg -A4 'triagePrompt =' "$definition" | rg -q 'transcript|conversation|caller'; then
  print -u2 'triage context leak'
  exit 1
fi
has 'usedProviders' && has 'c.provider' && has 'triageChain' && has 'effectiveTriageChain' || { print -u2 'non-same-provider rule missing'; exit 1; }
has 'deadNodes' && has 'deadReview' || { print -u2 'fan-in guard missing'; exit 1; }
has "'triage-skipped-no-review'" || { print -u2 'both-reviewers-dead case unhandled'; exit 1; }
has 'rawReviews' || { print -u2 'triage-exhausted fallback (raw reviews) missing'; exit 1; }
has "verdict: \{ type: 'string', enum: \['legit', 'not-legit'\] \}" && has "reasoning: \{ type: 'string' \}" || { print -u2 'finding schema missing verdict/reasoning'; exit 1; }
has 'Do not create a PR yourself' || { print -u2 'pre-PR mode PR-creation guard missing'; exit 1; }
has 'Never merge the PR' || { print -u2 'PR-merge guard missing'; exit 1; }
