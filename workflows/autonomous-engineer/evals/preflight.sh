#!/bin/zsh
set -euo pipefail
here=${0:A:h}
candidate=${1:-}
incumbent=$here/../autonomous-engineer.workflow.js
if [[ -z $candidate || ${candidate:t} == SKILL.md || ${candidate:e} == md ]]; then
  definition=$incumbent
else
  definition=$candidate
fi
[[ -r $definition ]] || { print -u2 "workflow not found: $definition"; exit 1; }

node - "$definition" <<'NODE'
const fs = require('fs')
const definition = fs.readFileSync(process.argv[2], 'utf8')
const source = definition.replace('export const meta =', 'const meta =')
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor
const run = new AsyncFunction('args', 'agent', 'parallel', 'phase', source)
run(null, async () => null, async values => Promise.all(values.map(value => value())), () => {})
  .then(output => {
    if (output.status !== 'blocked' || output.stop_reason !== 'invalid-input') process.exit(1)
  })
  .catch(() => process.exit(1))
NODE

has() {
  grep -Fq -- "$1" "$definition"
}

checks=(
  'autonomous-engineer-state repair-worktree --repo ${repo}'
  'is_real_worktree'
  'blockedBy'
  'manual-only'
  'closingIssuesReferences'
  'workflow('
  'stopBeforeDraft'
  "return stopBeforeDraft('failed', 'implementation-failed')"
  'verificationCheckout'
  'origin/${state.branch}^{commit}'
  'const repairedDraftSafety'
  'git worktree add --detach'
  'git worktree remove --force'
  'resume-draft'
  '<!-- autonomous-engineer repairs=${state.repairs} -->'
  'Closes #${task.id}'
  "agentType: 'Plan'"
  "agentType: 'code-reviewer'"
  "'anchor-verifier'"
  'model: models.T3'
  "isolation: 'worktree'"
  'model: models.T4'
  'models.T4ReviewAfterRepair'
  'model: models.T5'
  'while (!verification.is_pass && state.repairs < maxRepairs)'
  'agent-cap-before-repair'
  "return result('verified-ready'"
)
for check in "${checks[@]}"; do
  has "$check" || { print -u2 "missing topology token: $check"; exit 1; }
done

node - "$definition" <<'NODE'
const fs = require('fs')
const source = fs.readFileSync(process.argv[2], 'utf8')

const obsoleteVisualKey = ['is', 'ui', 'change'].join('_')
if (new RegExp(`\\b${obsoleteVisualKey}\\b`).test(source)) {
  console.error('obsolete visual-applicability boolean found')
  process.exit(1)
}
if (!/\bis_user_visible_change\b/.test(source)) {
  console.error('missing is_user_visible_change visual-applicability boolean')
  process.exit(1)
}
if (!/is_user_visible_change\s*:\s*\{[\s\S]{0,200}?type\s*:\s*['"]boolean['"]/.test(source)) {
  console.error('is_user_visible_change must be a schema boolean')
  process.exit(1)
}

function matchingParenthesis(openIndex, text = source) {
  let depth = 0
  let quote = null
  let isEscaped = false
  let comment = null
  for (let index = openIndex; index < text.length; index += 1) {
    const character = text[index]
    const next = text[index + 1]
    if (comment === 'line') {
      if (character === '\n') comment = null
      continue
    }
    if (comment === 'block') {
      if (character === '*' && next === '/') {
        comment = null
        index += 1
      }
      continue
    }
    if (quote) {
      if (isEscaped) {
        isEscaped = false
      } else if (character === '\\') {
        isEscaped = true
      } else if (character === quote) {
        quote = null
      }
      continue
    }
    if (character === '/' && next === '/') {
      comment = 'line'
      index += 1
      continue
    }
    if (character === '/' && next === '*') {
      comment = 'block'
      index += 1
      continue
    }
    if (character === "'" || character === '"' || character === '`') {
      quote = character
      continue
    }
    if (character === '(') depth += 1
    if (character === ')') {
      depth -= 1
      if (depth === 0) return index
    }
  }
  return -1
}

function callRanges(name, text = source) {
  const ranges = []
  const pattern = new RegExp(`\\b${name}\\s*\\(`, 'g')
  for (const match of text.matchAll(pattern)) {
    const openIndex = text.indexOf('(', match.index)
    const closeIndex = matchingParenthesis(openIndex, text)
    if (closeIndex < 0) continue
    ranges.push({ start: match.index, end: closeIndex + 1, text: text.slice(match.index, closeIndex + 1) })
  }
  return ranges
}

function fail(message) {
  console.error(message)
  process.exit(1)
}

const trackedCalls = callRanges('tracked')
const parallelCalls = callRanges('parallel')
const testerPattern = /agentType:\s*(?:verifierType|['"](?:anchor-verifier|spec-tester|maestro-tester)['"])/
const reviewerPattern = /agentType:\s*['"]code-reviewer['"]/
const testerCalls = trackedCalls.filter(call => testerPattern.test(call.text))
const reviewerCalls = trackedCalls.filter(call => reviewerPattern.test(call.text))
if (testerCalls.length === 0) fail('missing tester dispatch')
if (reviewerCalls.length === 0) fail('missing code-reviewer dispatch')

const testerCall = testerCalls[0]
const reviewerCall = reviewerCalls.find(call => call.start > testerCall.end)
if (!reviewerCall) fail('code reviewer must be dispatched after tester output')
if (!reviewerCall.text.includes('visual_evidence')) fail('code reviewer dispatch must consume tester visual_evidence')

const verifyPromptStart = source.lastIndexOf('const verifyPrompt =', testerCall.start)
if (verifyPromptStart < 0) fail('missing verifyPrompt construction before tester dispatch')
const verifyPromptConstruction = source.slice(verifyPromptStart, testerCall.start)
const visualPromptBranches = verifyPromptConstruction.match(
  /(?:===|==)\s*['"]spec(?:-tester)?['"]\s*\?\s*`((?:\\.|[^`])*)`\s*:\s*`((?:\\.|[^`])*)`/s)
if (!visualPromptBranches) fail('missing separate spec and Maestro verifyPrompt branches')
const visualPrompts = {
  spec: visualPromptBranches[1],
  Maestro: visualPromptBranches[2],
}
for (const [tester, prompt] of Object.entries(visualPrompts)) {
  if (!prompt.includes('change_scope:')) fail(`${tester} tester prompt must carry literal change_scope:`)
}

const verifySchemaStart = source.indexOf('const VERIFY_SCHEMA')
const verifySchemaEnd = source.indexOf('\nfunction ', verifySchemaStart)
const verifySchema = verifySchemaStart >= 0 && verifySchemaEnd > verifySchemaStart
  ? source.slice(verifySchemaStart, verifySchemaEnd)
  : ''
const isVisualEvidenceArray = /visual_evidence\s*:\s*\{[\s\S]*?type:\s*['"]array['"]/.test(verifySchema)
const isVisualEvidenceRequired = /required:\s*\[[^\]]*['"]visual_evidence['"][^\]]*\]/.test(verifySchema)
if (!isVisualEvidenceArray || !isVisualEvidenceRequired) fail('tester schema must require a visual_evidence array')

const afterDraftIndex = source.indexOf("await safety('after-draft'")
if (afterDraftIndex < 0 || afterDraftIndex > testerCall.start) fail('initial draft safety must run before visual testing')

for (const parallelCall of parallelCalls) {
  const isTesterInside = testerCalls.some(call => call.start > parallelCall.start && call.end < parallelCall.end)
  const isReviewerInside = reviewerCalls.some(call => call.start > parallelCall.start && call.end < parallelCall.end)
  if (isTesterInside && isReviewerInside) fail('tester and code reviewer cannot share an initial parallel batch')
}

const attachmentCalls = trackedCalls.filter(call =>
  call.start > testerCall.end
  && !reviewerPattern.test(call.text)
  && call.text.includes('visual_evidence')
  && call.text.includes('create-pr'))
if (attachmentCalls.length === 0) fail('missing separate post-test visual-evidence attachment update')
for (const parallelCall of parallelCalls) {
  const isTesterInside = testerCalls.some(call => call.start > parallelCall.start && call.end < parallelCall.end)
  const isAttachmentInside = attachmentCalls.some(call => call.start > parallelCall.start && call.end < parallelCall.end)
  if (isTesterInside && isAttachmentInside) fail('attachment update must wait for tester output')
}

function matchingBrace(openIndex, text = source) {
  let depth = 0
  let quote = null
  let isEscaped = false
  let comment = null
  for (let index = openIndex; index < text.length; index += 1) {
    const character = text[index]
    const next = text[index + 1]
    if (comment === 'line') {
      if (character === '\n') comment = null
      continue
    }
    if (comment === 'block') {
      if (character === '*' && next === '/') {
        comment = null
        index += 1
      }
      continue
    }
    if (quote) {
      if (isEscaped) {
        isEscaped = false
      } else if (character === '\\') {
        isEscaped = true
      } else if (character === quote) {
        quote = null
      }
      continue
    }
    if (character === '/' && next === '/') {
      comment = 'line'
      index += 1
      continue
    }
    if (character === '/' && next === '*') {
      comment = 'block'
      index += 1
      continue
    }
    if (character === "'" || character === '"' || character === '`') {
      quote = character
      continue
    }
    if (character === '{') depth += 1
    if (character === '}') {
      depth -= 1
      if (depth === 0) return index
    }
  }
  return -1
}

function followingStatement(conditionEnd, text = source) {
  let start = conditionEnd
  while (/\s/.test(text[start] || '')) start += 1
  if (text[start] === '{') {
    const end = matchingBrace(start, text)
    return { start, end: end + 1, text: end < 0 ? '' : text.slice(start, end + 1) }
  }
  const semicolon = text.indexOf(';', start)
  const end = semicolon < 0 ? text.length : semicolon + 1
  return { start, end, text: text.slice(start, end) }
}

const ifStatements = callRanges('if').map(call => ({
  start: call.start,
  condition: call.text,
  body: followingStatement(call.end),
}))
const implementationCandidates = trackedCalls.filter(call =>
  call.start < afterDraftIndex
  && /implement/i.test(call.text)
  && !/agentType:\s*['"]Plan['"]/.test(call.text))
if (implementationCandidates.length === 0) fail('missing implementation dispatch')
const implementationCall = implementationCandidates.reduce((latest, call) =>
  call.start > latest.start ? call : latest)
const isBlockingPlanReturn = statement =>
  /\breturn\b/.test(statement.body.text)
  && /block|invalid|stopBeforeDraft|plan-stopped|verification-mismatch/i.test(statement.body.text)
const isUserVisibleTrueCondition = condition =>
  /is_user_visible_change\s*(?:===|==)\s*true/.test(condition)
  || /(?:^|[(&])\s*(?:[A-Za-z0-9_]+\??\.)?is_user_visible_change\s*(?=&&|\))/.test(condition)
const anchorPlanGuards = ifStatements.filter(statement =>
  statement.start < implementationCall.start
  && /verification_kind[\s\S]*['"]anchor['"]|['"]anchor['"][\s\S]*verification_kind/.test(statement.condition)
  && isBlockingPlanReturn(statement))
const userVisibleAnchorPlanGuard = anchorPlanGuards.find(statement =>
  isUserVisibleTrueCondition(statement.condition)
  && statement.condition.includes('&&')
  && !statement.condition.includes('||'))
if (!userVisibleAnchorPlanGuard) fail('user-visible Plan with anchor verification must stop before implementation')
if (anchorPlanGuards.some(statement =>
  !isUserVisibleTrueCondition(statement.condition)
  || statement.condition.includes('||'))) {
  fail('non-visible Plan with anchor verification must remain valid')
}
function statementsIn(candidateSource, name) {
  return callRanges(name, candidateSource).map(call => ({
    start: call.start,
    end: call.end,
    condition: call.text,
    body: followingStatement(call.end, candidateSource),
  }))
}

function capturePlanRevision(candidateSource) {
  const candidateTrackedCalls = callRanges('tracked', candidateSource)
  const candidateAfterDraft = candidateSource.indexOf("await safety('after-draft'")
  const candidateImplementations = candidateTrackedCalls.filter(call =>
    call.start < candidateAfterDraft
    && /implement/i.test(call.text)
    && !/agentType:\s*['"]Plan['"]/.test(call.text))
  if (candidateImplementations.length === 0) return { error: 'missing implementation dispatch' }
  const candidateImplementation = candidateImplementations.reduce((latest, call) =>
    call.start > latest.start ? call : latest)
  const candidateIfs = statementsIn(candidateSource, 'if')
  const candidateLoops = [
    ...statementsIn(candidateSource, 'for'),
    ...statementsIn(candidateSource, 'while'),
  ]
  const guards = candidateIfs.filter(statement => {
    const context = candidateSource.slice(Math.max(0, statement.start - 600), statement.body.end)
    return statement.start < candidateImplementation.start
      && isUserVisibleTrueCondition(statement.condition)
      && /verification_kind[\s\S]*['"]spec['"]|['"]spec['"][\s\S]*verification_kind/.test(statement.condition)
      && /capture|screenshot|record/i.test(statement.condition)
      && /drive_matrix/.test(context)
  })
  if (guards.length === 0) return { error: 'missing user-visible spec capture-command gate' }
  const rejected = []
  for (const guard of guards) {
    const hasFeedback = /capture[ _-]*command|screenshot|screen[ _-]*recording|visual[ _-]*capture/i.test(guard.body.text)
      && /feedback|concern|revis|require|reason/i.test(guard.body.text)
    const revisionLoops = candidateLoops
      .filter(loop => {
        const enclosesGate = loop.start < guard.start && loop.body.end > guard.body.end
        const followsGate = loop.start > guard.body.end && loop.body.end < candidateImplementation.start
        return enclosesGate || followsGate
      })
      .sort((left, right) => {
        const leftDistance = Math.abs(left.start - guard.start)
        const rightDistance = Math.abs(right.start - guard.start)
        return leftDistance - rightDistance
      })
    for (const loop of revisionLoops) {
      const loopText = candidateSource.slice(loop.start, loop.body.end)
      const plannerDispatch = candidateTrackedCalls.find(call =>
        call.start > loop.body.start
        && call.end < loop.body.end
        && /agentType:\s*['"]Plan['"]/.test(call.text))
      if (!plannerDispatch) continue
      const classicBoundedFor = /^for\s*\([^;]+;[^;]*(?:<|<=|>=|>)[^;]+;[^)]*(?:\+\+|--|\+=\s*1|-=\s*1)[^)]*\)$/s.test(loop.condition)
      const finiteForOf = /^for\s*\([^)]*\bof\s*\[[^\]]+\][^)]*\)$/s.test(loop.condition)
      const counterName = '[A-Za-z_$][\\w$]*(?:round|attempt)[\\w$]*'
      const counterComparison = new RegExp(`\\b${counterName}\\s*(?:<|<=|>=|>)\\s*(?:\\d+|[A-Za-z_$][\\w$]*)`, 'i')
      const counterAdvance = new RegExp(`\\b${counterName}\\s*(?:\\+\\+|\\+=\\s*1)`, 'i')
      const boundedCounterLoop = counterComparison.test(loop.condition) && counterAdvance.test(loopText)
      const boundedAgentLoop = /(?:max_agents|maxAgents)/.test(loopText)
        && /(?:<|<=|>=|>)/.test(loopText)
      const boundedByExitGuard = counterComparison.test(loopText)
        && /\breturn\b/.test(loopText)
      const bounded = classicBoundedFor || finiteForOf || boundedCounterLoop || boundedAgentLoop || boundedByExitGuard
      const consumesRound = bounded && Boolean(plannerDispatch)
      const enclosesGate = loop.start < guard.start
      const gateExitEnd = candidateImplementation.start < loop.body.end
        ? candidateImplementation.start
        : loop.body.end
      const continueAfterGate = /\bcontinue\b/.test(candidateSource.slice(guard.body.end, gateExitEnd))
      const exhaustionBlock = candidateIfs.some(statement =>
        statement.start > loop.body.end
        && statement.start < candidateImplementation.start
        && isBlockingPlanReturn(statement))
      const deficientPlanCannotImplement = enclosesGate ? continueAfterGate : exhaustionBlock
      if (hasFeedback && consumesRound && deficientPlanCannotImplement) {
        return {
          guard,
          loop,
          plannerDispatch,
          feedbackPattern: /capture[ _-]*command|screenshot|screen[ _-]*recording|visual[ _-]*capture/i,
        }
      }
      if (!hasFeedback) rejected.push('capture-command feedback is missing')
      if (!bounded) rejected.push('the Plan retry is not bounded')
      if (!consumesRound) rejected.push('the failed Plan does not consume a round')
      if (!deficientPlanCannotImplement) rejected.push('the deficient Plan can reach implementation')
    }
  }
  return { error: [...new Set(rejected)].join('; ') || 'the capture-command gate has no Plan retry' }
}

function replaceRange(text, start, end, replacement) {
  return text.slice(0, start) + replacement + text.slice(end)
}

const captureRevision = capturePlanRevision(source)
if (captureRevision.error) fail(captureRevision.error)

const gateMutant = replaceRange(source, captureRevision.guard.start, captureRevision.guard.end, 'if (false)')
if (!capturePlanRevision(gateMutant).error) fail('mutation probe did not reject removal of capture-command gate')

const feedbackBody = captureRevision.guard.body.text
const feedbackWithoutCaptureCommand = feedbackBody.replace(
  /capture[ _-]*command|screenshot|screen[ _-]*recording|visual[ _-]*capture/gi,
  'plan requirement',
)
if (feedbackWithoutCaptureCommand === feedbackBody) fail('mutation probe could not locate capture-command feedback')
const feedbackMutant = replaceRange(
  source,
  captureRevision.guard.body.start,
  captureRevision.guard.body.end,
  feedbackWithoutCaptureCommand,
)
if (!capturePlanRevision(feedbackMutant).error) fail('mutation probe did not reject removal of capture-command feedback')

const retryMutant = replaceRange(
  source,
  captureRevision.plannerDispatch.start,
  captureRevision.plannerDispatch.end,
  captureRevision.plannerDispatch.text.replace(/agentType:\s*['"]Plan['"]/, "agentType: 'disabled'"),
)
if (!capturePlanRevision(retryMutant).error) fail('mutation probe did not reject removal of Plan retry')
const verifiedReadyIndexes = [...source.matchAll(/return\s+result\(\s*['"]verified-ready['"]/g)].map(match => match.index)
if (verifiedReadyIndexes.length === 0) fail('missing verified-ready return')
const firstVerifiedReady = Math.min(...verifiedReadyIndexes)

function propertyExpressions(text, property) {
  const expressions = []
  const pattern = new RegExp(`\\b${property}\\s*:`, 'g')
  for (const match of text.matchAll(pattern)) {
    let index = match.index + match[0].length
    const start = index
    let roundDepth = 0
    let squareDepth = 0
    let braceDepth = 0
    let quote = null
    let isEscaped = false
    for (; index < text.length; index += 1) {
      const character = text[index]
      if (quote) {
        if (isEscaped) {
          isEscaped = false
        } else if (character === '\\') {
          isEscaped = true
        } else if (character === quote) {
          quote = null
        }
        continue
      }
      if (character === "'" || character === '"' || character === '`') {
        quote = character
        continue
      }
      if (character === '(') roundDepth += 1
      if (character === ')') {
        if (roundDepth === 0 && squareDepth === 0 && braceDepth === 0) break
        roundDepth -= 1
      }
      if (character === '[') squareDepth += 1
      if (character === ']') squareDepth -= 1
      if (character === '{') braceDepth += 1
      if (character === '}') {
        if (roundDepth === 0 && squareDepth === 0 && braceDepth === 0) break
        braceDepth -= 1
      }
      if (character === ',' && roundDepth === 0 && squareDepth === 0 && braceDepth === 0) break
    }
    expressions.push(text.slice(start, index).trim())
  }
  return expressions
}

const verifiedReadyCalls = callRanges('result').filter(call =>
  /result\(\s*['"]verified-ready['"]/.test(call.text))
const readinessPassExpressions = verifiedReadyCalls.flatMap(call =>
  propertyExpressions(call.text, 'is_pass'))
const liveReviewReadiness = readinessPassExpressions.find(expression =>
  /review(?:\?\.|\.)status\s*(?:===|==)\s*['"]reviewed['"]/.test(expression)
  && /review(?:\?\.|\.)is_pass\b/.test(expression)
  && !/!\s*review(?:\?\.|\.)is_pass\b/.test(expression)
  && expression.includes('&&'))
if (!liveReviewReadiness) {
  fail('verified-ready live is_pass must directly require reviewed status and a passing review boolean')
}

const emptyEvidencePattern = /(?:visual_evidence|visualEvidence)[\s\S]{0,100}\.length\s*(?:===|==|<=)\s*0/
const userVisiblePattern = /\bis_user_visible_change\b/
const blockingEmptyEvidenceStatements = ifStatements.filter(statement =>
  statement.start < firstVerifiedReady
  && emptyEvidencePattern.test(statement.condition)
  && /\breturn\b/.test(statement.body.text)
  && !/verified-ready/.test(statement.body.text))
const emptyVisualEvidenceGuard = blockingEmptyEvidenceStatements.find(statement =>
  userVisiblePattern.test(statement.condition)
  && statement.condition.includes('&&')
  && !statement.condition.includes('||'))
if (!emptyVisualEvidenceGuard) fail('user-visible verification must reject empty visual_evidence')
if (/attach/i.test(emptyVisualEvidenceGuard.body.text)) {
  fail('empty user-visible visual_evidence is a verification failure, not an attachment failure')
}
if (blockingEmptyEvidenceStatements.some(statement => !userVisiblePattern.test(statement.condition) || statement.condition.includes('||'))) {
  fail('non-visible empty visual_evidence must remain valid')
}

const isAttachmentConditional = ifStatements.some(statement =>
  /(?:visual_evidence|visualEvidence)[\s\S]{0,100}\.length(?:\s*>\s*0)?/.test(statement.condition)
  && attachmentCalls.some(call => call.start > statement.body.start && call.end < statement.body.end))
if (!isAttachmentConditional) fail('non-visible empty visual_evidence must skip attachment update')

const repairBudgetGuard = ifStatements.find(statement =>
  statement.body.text.includes('agent-cap-before-repair'))
if (!repairBudgetGuard) fail('missing repair agent-budget guard')
const repairBudgetContext = source.slice(Math.max(0, repairBudgetGuard.start - 2000), repairBudgetGuard.body.end)
const attachmentReservations = [...repairBudgetContext.matchAll(/([\s\S]{0,300})\?\s*([01])\s*:\s*([01])/g)]
  .map(match => ({ condition: match[1], whenTrue: match[2], whenFalse: match[3] }))
const repeatVisualTesterReservation = attachmentReservations.some(reservation => {
  const expression = reservation.condition
  if (!/verification_kind|verificationKind|verifierType/.test(expression)
    || /(?:visual_evidence|visualEvidence)[\s\S]{0,100}\.length/.test(expression)) return false
  const reservesNonAnchor = /!==?\s*['"]anchor(?:-verifier)?['"]/.test(expression)
    && reservation.whenTrue === '1' && reservation.whenFalse === '0'
  const reservesAnchorElse = /={2,3}\s*['"]anchor(?:-verifier)?['"]/.test(expression)
    && reservation.whenTrue === '0' && reservation.whenFalse === '1'
  const namesBothVisualTesters = /['"]spec(?:-tester)?['"]/.test(expression)
    && /['"]maestro(?:-tester)?['"]/.test(expression)
    && reservation.whenTrue === '1' && reservation.whenFalse === '0'
  return reservesNonAnchor || reservesAnchorElse || namesBothVisualTesters
})
if (!repeatVisualTesterReservation) {
  fail('repair budget must reserve an attachment node before repeat spec or Maestro verification')
}

function stoppedAnchorDataFlowErrors(candidateSource) {
  const errors = []
  const flow = candidateSource.slice(testerCall.end, reviewerCall.start)
  const stoppedVerdictIndex = flow.search(/\bverdict\s*:\s*['"]stopped['"]/)
  const stoppedVerdictContext = stoppedVerdictIndex < 0
    ? ''
    : flow.slice(Math.max(0, stoppedVerdictIndex - 1200), stoppedVerdictIndex + 1200)
  const hasAnchorCondition = /(?:verifierType|verification_kind|verificationKind)\s*(?:===|==)\s*['"]anchor(?:-verifier)?['"]/.test(stoppedVerdictContext)
  const hasMissingVerdictCondition = /(?:!\s*[\w$?.]+\.verdict|[\w$?.]+\.verdict\s*(?:\?|\?\?|===?\s*(?:null|undefined)))/.test(stoppedVerdictContext)
  if (stoppedVerdictIndex < 0 || !hasAnchorCondition || !hasMissingVerdictCondition) {
    errors.push('missing anchor verdict must normalize to stopped')
  }
  const stoppedEvidenceExpression = propertyExpressions(stoppedVerdictContext, 'evidence').find(expression =>
    /['"]verification-stopped['"]/.test(expression))
  if (!stoppedEvidenceExpression || /^\[\s*\]$/.test(stoppedEvidenceExpression)) {
    errors.push('stopped anchor evidence must normalize to verification-stopped')
  }
  if (!/state\.verificationVerdict\s*=[^\n;]{0,120}\bverification\??\.verdict\b/.test(candidateSource)) {
    errors.push('state verification verdict must receive normalized verification verdict')
  }
  if (!/state\.checks\.push\s*\([\s\S]{0,120}?\bverification\??\.evidence\b/.test(candidateSource)) {
    errors.push('state checks must receive normalized verification evidence')
  }
  const resultStart = candidateSource.search(/\bfunction\s+result\s*\(/)
  const resultEnd = resultStart < 0 ? -1 : candidateSource.indexOf('\nfunction ', resultStart + 1)
  const resultDefinition = resultStart < 0
    ? ''
    : candidateSource.slice(resultStart, resultEnd < 0 ? candidateSource.length : resultEnd)
  if (!/verification_verdict\s*:\s*state\.verificationVerdict\b/.test(resultDefinition)) {
    errors.push('result must expose state verification verdict')
  }
  if (!/checks\s*:\s*state\.checks\b/.test(resultDefinition)) {
    errors.push('result must expose state checks')
  }
  if (/stop_reason\s*:\s*['"]verification-stopped['"]/.test(candidateSource)) {
    errors.push('verification-stopped must not be a stop reason')
  }
  return errors
}

const stoppedAnchorErrors = stoppedAnchorDataFlowErrors(source)
if (stoppedAnchorErrors.length > 0) fail(stoppedAnchorErrors[0])

function requireRejectedMutation(pattern, replacement, expectedError, label) {
  const mutant = source.replace(pattern, replacement)
  if (mutant === source) fail(`mutation probe did not modify ${label}`)
  if (!stoppedAnchorDataFlowErrors(mutant).includes(expectedError)) {
    fail(`mutation probe did not reject ${label}`)
  }
}

requireRejectedMutation(
  /(evidence\s*:[\s\S]{0,300}?)['"]verification-stopped['"]/,
  "$1'verification-missing'",
  'stopped anchor evidence must normalize to verification-stopped',
  'stopped anchor evidence assignment')
requireRejectedMutation(
  /state\.verificationVerdict\s*=[^\n;]{0,120}\bverification\??\.verdict\b/,
  "state.verificationVerdict = 'failed'",
  'state verification verdict must receive normalized verification verdict',
  'state verification verdict assignment')

const attachmentFailureGuard = ifStatements.find(statement =>
  attachmentCalls.some(call => statement.start > call.end)
  && statement.start < firstVerifiedReady
  && /attach/i.test(statement.condition)
  && /(?:status|control|outcome|is_pass)/.test(statement.condition)
  && (/(?:!==|!=|===\s*false|==\s*false)/.test(statement.condition) || /!\s*[\w?.]*attach/i.test(statement.condition))
  && /\breturn\b/.test(statement.body.text)
  && !/verified-ready/.test(statement.body.text))
if (!attachmentFailureGuard) fail('attachment failure must block verified-ready')
NODE

if grep -Eiq 'gh pr merge|git merge|\bmerge\(' "$definition"; then
  print -u2 'forbidden merge operation found'
  exit 1
fi
if grep -Eq 'if .*reason|reason.*if' "$definition"; then
  print -u2 'reason text controls flow'
  exit 1
fi
