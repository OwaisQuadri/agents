#!/bin/zsh
set -euo pipefail
here=${0:A:h}
candidate=${1:-}
incumbent=$here/../research-sweep.workflow.js
if [[ -z $candidate || ${candidate:t} == SKILL.md || ${candidate:e} == md ]]; then
  definition=$incumbent
else
  definition=$candidate
fi
[[ -r $definition ]] || { print -u2 "workflow not found: $definition"; exit 1; }
has() {
  grep -qE "$1" "$definition"
}

has 'missing input: goal' && has 'if \(!GOAL\) return' || { print -u2 'goal guard missing'; exit 1; }
has 'MAX_PLANNED' && has 'MAX_FILL' && has 'slice\(0, *MAX_PLANNED\)' && has 'slice\(0, *MAX_FILL\)' || { print -u2 'cap missing'; exit 1; }
if grep -A2 'completeness critic' "$definition" | grep -qE 'transcript|conversation history'; then
  print -u2 'critic context leak'
  exit 1
fi
has 'findings block: \$\{b.label\}' || { print -u2 'critic input is unverifiable'; exit 1; }
has 'critic.missing.slice' && has 'isSufficient' || { print -u2 'fill round missing'; exit 1; }
has 'missingLabels' && has 'returned' && has 'expected' && has 'if \(!text\) return null' && has 'plan node returned nothing' || { print -u2 'fan-in guard missing'; exit 1; }
has 'codebase_dispatches' && has "'Explore'" && has 'Promise.all' || { print -u2 'codebase fan-out missing'; exit 1; }
has 'INCLUDE_CODEBASE' && has 'includeCodebase' || { print -u2 'codebase flag missing'; exit 1; }
has 'never invent a codebase angle' || { print -u2 'codebase planning rule missing'; exit 1; }
has 'isValidFindingsBlock' && has 'resume: d.label' && has "agentType !== 'web-research-summarizer'" && has 'claim:' && has 'cited=' || { print -u2 'shape check or same-child retry missing'; exit 1; }
