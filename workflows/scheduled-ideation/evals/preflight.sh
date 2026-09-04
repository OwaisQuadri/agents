#!/bin/zsh
set -euo pipefail
here=${0:A:h}
candidate=${1:-}
incumbent=$here/../scheduled-ideation.workflow.js
if [[ -z $candidate || ${candidate:t} == SKILL.md || ${candidate:e} == md ]]; then
  definition=$incumbent
else
  definition=$candidate
fi
[[ -r $definition ]] || { print -u2 "workflow not found: $definition"; exit 1; }
has() {
  grep -qE "$1" "$definition"
}

has 'if \(!rawCandidates.length\)' && has 'no candidates today' || { print -u2 'zero-candidate guard missing'; exit 1; }
has 'MAX_TOOL_RADAR = Math.min' && has 'slice\(0, MAX_TOOL_RADAR\)' && has 'MAX_DIGEST_CANDIDATES = 10' && has 'slice\(0, MAX_DIGEST_CANDIDATES\)' || { print -u2 'cap missing'; exit 1; }
if grep -A6 'fresh-context filter' "$definition" | grep -qE 'transcript|conversation history'; then
  print -u2 'filter context leak'
  exit 1
fi
has 'rawCandidates.map\(\(c, i\)' || { print -u2 'filter isolation is unverifiable'; exit 1; }
has 'generateMissing' && has 'missingLabels: generateMissing' && has 'returned: generateResults.length' || { print -u2 'fan-in guard missing'; exit 1; }
mining_block=$(awk '/parallel\(toolRadar\.map/{exit} /parallel\(mining\.map/{is_printing=1} is_printing{print}' "$definition")
has "agentType: 'web-research-summarizer'" && has 'mining.map' && has 'toolRadar.map' || { print -u2 'routing missing'; exit 1; }
if print -r -- "$mining_block" | grep -q "agentType: 'web-research-summarizer'"; then
  print -u2 'mining uses the web research summarizer'
  exit 1
fi
has 'skills/ai-author/SKILL.md' && has 'read that file first, then follow its numbered steps' || { print -u2 'authoring procedure handoff missing'; exit 1; }
if has 'Read no more than ten artifacts'; then
  print -u2 'authoring procedure duplicated inline'
  exit 1
fi
has 'omit a heading entirely if it has zero candidates' && has 'never write an empty section' || { print -u2 'empty heading guard missing'; exit 1; }
has 'NOT limited to artifact usage logs' && has 'greps 2\+ times' && has 'is measured evidence' || { print -u2 'friction instruction missing'; exit 1; }
mining_index=$(grep -n 'await parallel(mining.map' "$definition" | head -1 | cut -d: -f1)
tool_radar_index=$(grep -n 'parallel(toolRadar.map' "$definition" | head -1 | cut -d: -f1)
[[ -n $mining_index && -n $tool_radar_index && $mining_index -lt $tool_radar_index ]] && has 'usageGrounding' || { print -u2 'barrier sequence missing'; exit 1; }
has 'addresses no measured friction this run' && has 'MUST explicitly name which of these' && has 'agent\(dispatchPrompt\(d, usageGrounding\)' || { print -u2 'tool radar grounding missing'; exit 1; }
