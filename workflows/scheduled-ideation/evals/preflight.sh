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

node - "$definition" <<'NODE'
const fs = require('node:fs')
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor
const definition = process.argv[2]
const source = fs.readFileSync(definition, 'utf8').replace(/^export const meta/m, 'const meta')
const runWorkflow = new AsyncFunction('args', 'agent', 'parallel', 'phase', 'log', source)
const auditLabel = 'prompt-snippet-audit'
const miningLabels = ['skill-evidence-sweep', 'agent-candidate-scan', 'correction-mining']
const toolLabels = ['tool-one', 'tool-two', 'tool-three']
const dispatches = [
  ...miningLabels.map((label, index) => ({
    label,
    category: ['skill', 'agent', 'checker'][index],
    objective: 'test',
    boundaries: 'test',
    source_guidance: 'test',
    recency: 'test',
  })),
  ...toolLabels.map(label => ({
    label,
    category: 'tool',
    objective: 'test',
    boundaries: 'test',
    source_guidance: 'test',
    recency: 'test',
  })),
]
const auditCandidate = {
  name: 'snippet-audit-sentinel',
  category: 'skill',
  rationale: 'measured test candidate',
  evidence: 'measured test evidence',
  source: 'aggregate audit',
}

const fail = message => {
  console.error(message)
  process.exit(1)
}

async function run(mode) {
  const calls = []
  let activeWave = -1
  let nextWave = 0
  let filterPrompt = ''
  const plannedDispatches = mode === 'label-collision'
    ? [{ ...dispatches[0], label: auditLabel }, ...dispatches.slice(1)]
    : dispatches

  const agent = async (prompt, options = {}) => {
    calls.push({ label: options.label, phase: options.phase, prompt, wave: activeWave })
    if (options.label === 'plan') return { dispatches: plannedDispatches }
    if (options.phase === 'Filter') {
      filterPrompt = prompt
      return { survivors: [], droppedCount: 1, dropReason: 'test' }
    }
    if (options.phase === 'Digest') return '# test digest'
    if (options.label === auditLabel && (mode === 'missing' || mode === 'label-collision')) return null
    if (options.label === auditLabel && mode === 'candidate') return { candidates: [auditCandidate] }
    return { candidates: [] }
  }

  const parallel = async tasks => {
    const wave = nextWave++
    activeWave = wave
    const promises = tasks.map(task => task())
    activeWave = -1
    return Promise.all(promises)
  }

  const result = await runWorkflow(
    { max_tool_radar: 99 },
    agent,
    parallel,
    () => {},
    () => {},
  )
  return { calls, filterPrompt, result }
}

;(async () => {
  const complete = await run('complete')
  const generateCalls = complete.calls.filter(call => call.phase === 'Generate')
  const auditCalls = generateCalls.filter(call => call.label === auditLabel)
  if (auditCalls.length !== 1) fail('fixed prompt-snippet audit missing or duplicated')

  const miningWave = generateCalls.find(call => call.label === miningLabels[0])?.wave
  if (miningWave === undefined || miningWave < 0) fail('planned mining wave missing')
  if (!miningLabels.every(label => generateCalls.some(call => call.label === label && call.wave === miningWave))) {
    fail('planned mining jobs do not share one parallel wave')
  }
  if (auditCalls[0].wave !== miningWave) fail('prompt-snippet audit is outside the mining parallel wave')
  if (!toolLabels.every(label => generateCalls.some(call => call.label === label && call.wave > miningWave))) {
    fail('tool-radar wave does not follow the complete mining wave')
  }
  if (complete.result.expected !== 7 || complete.result.returned !== 7 || complete.result.missingLabels.length !== 0) {
    fail('prompt-snippet audit missing from complete-run accounting')
  }

  const missing = await run('missing')
  if (missing.result.expected !== 7 || missing.result.returned !== 6 || !missing.result.missingLabels.includes(auditLabel)) {
    fail('prompt-snippet audit missing from expected/returned/missingLabels accounting')
  }

  const candidate = await run('candidate')
  if (!candidate.filterPrompt.includes(auditCandidate.name)) {
    fail('prompt-snippet audit does not reach the shared candidate collection and Filter')
  }
  if (candidate.calls.some(call => toolLabels.includes(call.label) && call.prompt.includes(auditCandidate.name))) {
    fail('private prompt-snippet audit evidence reaches a web-research dispatch')
  }

  const collision = await run('label-collision')
  const collisionLabels = collision.calls.filter(call => call.phase === 'Generate').map(call => call.label)
  if (new Set(collisionLabels).size !== collisionLabels.length) {
    fail('planned dispatch labels collide with the fixed prompt-snippet audit')
  }
  if (collision.result.expected !== 7 || collision.result.returned !== 6 || !collision.result.missingLabels.includes(auditLabel)) {
    fail('a planned label collision hides a missing fixed prompt-snippet audit')
  }

  const prompt = auditCalls[0].prompt
  const requires = (pattern, message) => {
    if (!pattern.test(prompt)) fail(message)
  }
  requires(/pi\/extensions\/prompt-snippets\/snippets\//i, 'prompt-snippet audit source path missing')
  requires(/(?:at most|no more than|cap(?:ped)?(?: at| to)?)\s+500|500\s+(?:direct-use\s+)?records/i, 'prompt-snippet direct-use record cap missing')
  requires(/(?:latest|last|previous|most recent)\s+30\s+days/i, 'prompt-snippet 30-day window missing')
  requires(/PI_CODING_AGENT_DIR\/prompt-snippets-usage\.jsonl/i, 'configured prompt-snippet usage path missing')
  requires(/~\/\.pi\/agent\/prompt-snippets-usage\.jsonl/i, 'default prompt-snippet usage path missing')
  requires(/~\/\.pi\/agent\/sessions\//i, 'parent-session transcript directory missing')
  requires(/direct[- ]use/i, 'direct-use evidence class missing')
  requires(/(?:equivalent|manually typed|manual request)/i, 'equivalent manually typed request class missing')
  requires(/at most 200 parent(?: Pi)? session transcripts/i, 'parent-session count cap missing')
  requires(/grep.{0,40}first/i, 'grep-first transcript rule missing')
  requires(/bounded.{0,40}(?:window|offset|limit)|(?:window|offset|limit).{0,40}bounded/i, 'bounded transcript-window rule missing')
  requires(/(?:distinguish|separate|do not (?:combine|conflate|merge)).{0,100}(?:direct[- ]use|manually typed)|(?:direct[- ]use).{0,100}(?:distinct|separate|manually typed)/is, 'direct and manual evidence are not distinguished')
  requires(/source \(the literal value "aggregate prompt-snippet audit"; never a path or identifier\)/i, 'aggregate-only audit source rule missing')
  if (/source \(a file\/log path for mining/.test(prompt)) fail('audit prompt requests a private source path')
  requires(/(?:do not|never|must not).{0,240}prompt text/is, 'prompt-text privacy rule missing')
  for (const term of ['response text', 'raw records', 'transcript excerpts', 'session identifiers', 'session paths']) {
    if (!prompt.toLowerCase().includes(term)) fail(`${term} privacy rule missing`)
  }
  requires(/absence.{0,80}unknown|unknown.{0,80}absence/is, 'absence-is-unknown rule missing')
  requires(/(?:add|addition).{0,160}(?:at least|2\+?|two).{0,40}parent sessions/is, 'two-parent-session add threshold missing')
  requires(/(?:merge|removal|remove).{0,240}(?:co-use|conflict|overlap).{0,120}existing skill/is, 'merge/removal evidence threshold missing')
  requires(/(?:weak|insufficient|below (?:the )?threshold).{0,120}zero candidates|zero candidates.{0,120}(?:weak|insufficient|below (?:the )?threshold)/is, 'weak-evidence zero-candidate rule missing')
})().catch(error => fail(error.stack || String(error)))
NODE
