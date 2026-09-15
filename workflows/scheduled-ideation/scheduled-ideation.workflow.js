export const meta = {
  name: 'scheduled-ideation',
  description: 'Select one supported money-making opportunity test from owner context and early market evidence.',
  whenToUse: 'A scheduled daily run needs an inspectable recommendation only; a human approves any later filing, contact, purchase, publication, or execution.',
  phases: [
    { title: 'Discovery', detail: 'six fixed internal and public research jobs run together' },
    { title: 'Generate', detail: 'one internal agent creates bounded opportunities' },
    { title: 'Distribution', detail: 'one public researcher tests candidate-specific access' },
    { title: 'Verify', detail: 'two fresh skeptics check every enriched candidate together' },
    { title: 'Rank', detail: 'one fresh ranker intersects approvals and ranks survivors' },
  ],
}

const DISCOVERY_LABELS = ['owner-profile', 'active-commitments', 'hacker-news', 'design-signals', 'broad-news', 'market-corroboration']
const EXTERNAL_LABELS = new Set(['hacker-news', 'design-signals', 'broad-news', 'market-corroboration'])
const MAX_OPPORTUNITIES = 6
const MAX_GENERATED_RECORDS = MAX_OPPORTUNITIES * 2
const MAX_SURVIVORS = 3
const FACT_LIMITS = { 'owner-profile': 20, 'active-commitments': 20, 'hacker-news': 20, 'design-signals': 12, 'broad-news': 20, 'market-corroboration': 24 }
const MAX_COVERAGE_PER_DISCOVERY = 13
const MAX_FIELD_LENGTH = 1000
const MAX_EVIDENCE_IDS = 8
const MAX_AGENTS = DISCOVERY_LABELS.length + 5
const EVIDENCE_SIGNAL_TYPES = ['paid-pain', 'budget', 'adoption', 'commitment', 'owner-fit', 'distribution-access', 'contrary', 'trend', 'market-size', 'work-demand', 'launch-activity', 'regulation', 'active-commitment', 'capability']
const READ_ONLY_BOUNDARY = 'Work read-only. Do not file, contact, purchase, publish, or execute anything. Human approval is required before any later action.'

const EVIDENCE_SCHEMA = {
  type: 'object',
  properties: {
    id: { type: 'string', maxLength: 120 },
    discoveryKind: { type: 'string', maxLength: 40 },
    signalType: { type: 'string', maxLength: 40, enum: EVIDENCE_SIGNAL_TYPES },
    claim: { type: 'string', maxLength: 1000 },
    source: { type: 'string', maxLength: 500 },
    observedAt: { type: 'string', maxLength: 10 },
    strength: { type: 'string', enum: ['direct', 'proxy', 'unknown'] },
  },
  required: ['id', 'discoveryKind', 'signalType', 'claim', 'source', 'observedAt', 'strength'],
}

const COVERAGE_SCHEMA = {
  type: 'object',
  properties: {
    source: { type: 'string', maxLength: 500 },
    status: { type: 'string', maxLength: 20, enum: ['complete', 'partial', 'unavailable'] },
    window: { type: 'string', maxLength: 100 },
    itemsReviewed: { type: 'number' },
    limitation: { type: 'string', maxLength: 1000 },
  },
  required: ['source', 'status', 'window', 'itemsReviewed', 'limitation'],
}

const DISCOVERY_SCHEMA = {
  type: 'object',
  properties: {
    facts: { type: 'array', items: EVIDENCE_SCHEMA },
    coverage: { type: 'array', items: COVERAGE_SCHEMA },
  },
  required: ['facts', 'coverage'],
}

const TEST_SCHEMA = {
  type: 'object',
  properties: {
    timeboxDays: { type: 'number' },
    buyer: { type: 'string' },
    paidPain: { type: 'string' },
    offer: { type: 'string' },
    distributionChannel: { type: 'string' },
    firstBuyerPath: { type: 'string' },
    channelEvidenceIds: { type: 'array', items: { type: 'string' } },
    smallestBuild: { type: 'string' },
    successSignal: { type: 'string' },
    stopCondition: { type: 'string' },
  },
  required: ['timeboxDays', 'buyer', 'paidPain', 'offer', 'distributionChannel', 'firstBuyerPath', 'channelEvidenceIds', 'smallestBuild', 'successSignal', 'stopCondition'],
}

const OPPORTUNITY_SCHEMA = {
  type: 'object',
  properties: {
    id: { type: 'string' },
    name: { type: 'string' },
    thesis: { type: 'string' },
    ownerFit: { type: 'string' },
    marketTiming: { type: 'string' },
    marketTimingEvidenceIds: { type: 'array', items: { type: 'string' } },
    ownerFitEvidenceIds: { type: 'array', items: { type: 'string' } },
    confirmingEvidenceIds: { type: 'array', items: { type: 'string' } },
    contraryEvidenceIds: { type: 'array', items: { type: 'string' } },
    maturity: { type: 'string', enum: ['early', 'emerging', 'mature-with-supported-wedge'] },
    limitations: { type: 'array', items: { type: 'string' } },
    test: TEST_SCHEMA,
  },
  required: ['id', 'name', 'thesis', 'ownerFit', 'marketTiming', 'marketTimingEvidenceIds', 'ownerFitEvidenceIds', 'confirmingEvidenceIds', 'contraryEvidenceIds', 'maturity', 'limitations', 'test'],
}

const DISTRIBUTION_SCHEMA = {
  type: 'object',
  properties: {
    candidateKey: { type: 'string' },
    channel: { type: 'string' },
    buyerPresence: { type: 'string' },
    ownerAccess: { type: 'string' },
    accessRequirements: { type: 'string' },
    costOrLimits: { type: 'string' },
    delay: { type: 'string' },
    accessTimeDays: { type: 'number' },
    allowedOffer: { type: 'string' },
    firstBuyerPath: { type: 'string' },
    commitmentSignal: { type: 'string' },
    sourceRefs: { type: 'array', items: { type: 'string' } },
    limitations: { type: 'array', items: { type: 'string' } },
  },
  required: ['candidateKey', 'channel', 'buyerPresence', 'ownerAccess', 'accessRequirements', 'costOrLimits', 'delay', 'accessTimeDays', 'allowedOffer', 'firstBuyerPath', 'commitmentSignal', 'sourceRefs', 'limitations'],
}

const RANKED_SCHEMA = {
  type: 'object',
  properties: {
    survivorIds: { type: 'array', items: { type: 'string' } },
    droppedIds: { type: 'array', items: { type: 'string' } },
    droppedCount: { type: 'number' },
    dropReasons: { type: 'array', items: { type: 'string' } },
    duplicateGroups: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          keptId: { type: 'string' },
          droppedIds: { type: 'array', items: { type: 'string' } },
          reason: { type: 'string' },
        },
        required: ['keptId', 'droppedIds', 'reason'],
      },
    },
  },
  required: ['survivorIds', 'droppedIds', 'droppedCount', 'dropReasons', 'duplicateGroups'],
}

const REVIEW_SCHEMA = {
  type: 'object',
  properties: {
    approvedIds: { type: 'array', items: { type: 'string' } },
    rejected: {
      type: 'array',
      items: {
        type: 'object',
        properties: { id: { type: 'string' }, reason: { type: 'string' } },
        required: ['id', 'reason'],
      },
    },
  },
  required: ['approvedIds', 'rejected'],
}

const isText = value => typeof value === 'string' && value.trim().length > 0
const isBoundedText = (value, maxLength) => isText(value) && value.length <= maxLength
const isNonEmptyArray = value => Array.isArray(value) && value.length > 0
const isBoundedIdArray = (value, maxItems) => isNonEmptyArray(value) && value.length <= maxItems && value.every(id => isBoundedText(id, 120))
const isKnownEvidence = (id, facts) => facts.some(fact => fact.id === id)
const isValidEvidence = fact => Boolean(fact && isBoundedText(fact.id, 120) && isBoundedText(fact.discoveryKind, 40) && EVIDENCE_SIGNAL_TYPES.includes(fact.signalType) && isBoundedText(fact.claim, 1000) && isBoundedText(fact.source, 500) && isObservedDate(fact.observedAt) && ['direct', 'proxy', 'unknown'].includes(fact.strength))
const isPaidSignal = signal => ['paid-pain', 'budget', 'adoption', 'commitment'].includes(signal)
const isPublicUrl = value => isBoundedText(value, MAX_FIELD_LENGTH) && /^https?:\/\/[^\s]+$/i.test(value)
const isObservedDate = value => {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value || '')
  if (!match) return false
  const [, year, month, day] = match
  return new Date(Date.UTC(Number(year), Number(month) - 1, Number(day))).toISOString().slice(0, 10) === value
}
const safeText = value => String(value).replace(/\r?\n/g, ' ').replace(/\s+/g, ' ').trim()
const cleanCoverage = (item, discoveryKind) => ({ discoveryKind, source: item.source, status: item.status, window: item.window, itemsReviewed: item.itemsReviewed, limitation: item.limitation })
const cleanFact = (fact, discoveryKind) => ({ id: fact.id, discoveryKind, signalType: fact.signalType, claim: fact.claim, source: fact.source, observedAt: fact.observedAt, strength: fact.strength })
const cleanDistribution = (item, opportunityId) => ({ candidateKey: item.candidateKey, opportunityId, channel: item.channel, buyerPresence: item.buyerPresence, ownerAccess: item.ownerAccess, accessRequirements: item.accessRequirements, costOrLimits: item.costOrLimits, delay: item.delay, accessTimeDays: item.accessTimeDays, allowedOffer: item.allowedOffer, firstBuyerPath: item.firstBuyerPath, commitmentSignal: item.commitmentSignal, sourceRefs: item.sourceRefs.slice(0, 4), limitations: item.limitations.slice(0, 4) })

function cleanOpportunity(opportunity) {
  return {
    id: opportunity.id,
    name: opportunity.name,
    thesis: opportunity.thesis,
    ownerFit: opportunity.ownerFit,
    marketTiming: opportunity.marketTiming,
    marketTimingEvidenceIds: [...new Set(opportunity.marketTimingEvidenceIds)].slice(0, 4),
    ownerFitEvidenceIds: [...new Set(opportunity.ownerFitEvidenceIds)].slice(0, 4),
    confirmingEvidenceIds: [...new Set(opportunity.confirmingEvidenceIds)].slice(0, MAX_EVIDENCE_IDS),
    contraryEvidenceIds: [...new Set(opportunity.contraryEvidenceIds)].slice(0, 4),
    maturity: opportunity.maturity,
    limitations: opportunity.limitations.slice(0, 4),
    test: {
      timeboxDays: opportunity.test.timeboxDays,
      buyer: opportunity.test.buyer,
      paidPain: opportunity.test.paidPain,
      offer: opportunity.test.offer,
      distributionChannel: opportunity.test.distributionChannel,
      firstBuyerPath: opportunity.test.firstBuyerPath,
      channelEvidenceIds: [...new Set(opportunity.test.channelEvidenceIds)].slice(0, 4),
      smallestBuild: opportunity.test.smallestBuild,
      successSignal: opportunity.test.successSignal,
      stopCondition: opportunity.test.stopCondition,
    },
  }
}

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`
  if (value && typeof value === 'object') return `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(',')}}`
  return JSON.stringify(value)
}

function isValidOpportunity(opportunity, facts) {
  const test = opportunity && opportunity.test
  if (!opportunity || !test) return false
  const requiredText = [opportunity.id, opportunity.name, opportunity.thesis, opportunity.ownerFit, opportunity.marketTiming, opportunity.maturity, test.buyer, test.paidPain, test.offer, test.distributionChannel, test.firstBuyerPath, test.smallestBuild, test.successSignal, test.stopCondition]
  if (!requiredText.every(value => isBoundedText(value, MAX_FIELD_LENGTH)) || !isNonEmptyArray(opportunity.limitations) || opportunity.limitations.length > 4 || !opportunity.limitations.every(value => isBoundedText(value, MAX_FIELD_LENGTH)) || !isBoundedIdArray(opportunity.marketTimingEvidenceIds, 4) || !isBoundedIdArray(opportunity.ownerFitEvidenceIds, 4) || !isBoundedIdArray(opportunity.confirmingEvidenceIds, MAX_EVIDENCE_IDS) || !isBoundedIdArray(opportunity.contraryEvidenceIds, 4) || !isBoundedIdArray(test.channelEvidenceIds, 4)) return false
  if (!Number.isFinite(test.timeboxDays) || test.timeboxDays < 1 || test.timeboxDays > 7) return false
  if (!['early', 'emerging', 'mature-with-supported-wedge'].includes(opportunity.maturity)) return false
  const timingFacts = [...new Set(opportunity.marketTimingEvidenceIds)].map(id => facts.find(fact => fact.id === id)).filter(Boolean)
  const ownerFitFacts = [...new Set(opportunity.ownerFitEvidenceIds)].map(id => facts.find(fact => fact.id === id)).filter(Boolean)
  if (timingFacts.length < 2 || timingFacts.some(fact => !EXTERNAL_LABELS.has(fact.discoveryKind)) || new Set(timingFacts.map(fact => fact.observedAt)).size < 2) return false
  if (!ownerFitFacts.some(fact => ['owner-profile', 'active-commitments'].includes(fact.discoveryKind) && ['owner-fit', 'capability', 'active-commitment'].includes(fact.signalType))) return false
  const confirming = [...new Set(opportunity.confirmingEvidenceIds)].filter(id => isKnownEvidence(id, facts))
  const confirmingFacts = confirming.map(id => facts.find(fact => fact.id === id))
  const signals = new Set(confirmingFacts.map(fact => fact.signalType))
  const sources = new Set(confirmingFacts.map(fact => fact.source))
  const isPaidEvidencePresent = confirmingFacts.some(fact => isPaidSignal(fact.signalType) && fact.strength === 'direct')
  const publicFacts = confirmingFacts.filter(fact => EXTERNAL_LABELS.has(fact.discoveryKind))
  const publicSources = new Set(publicFacts.map(fact => fact.source))
  const isPublicPaidEvidencePresent = publicFacts.some(fact => isPaidSignal(fact.signalType) && fact.strength === 'direct')
  const channelFacts = test.channelEvidenceIds.map(id => facts.find(fact => fact.id === id)).filter(Boolean)
  return confirming.length >= 3 && signals.size >= 2 && sources.size >= 3 && isPaidEvidencePresent && publicSources.size >= 2 && isPublicPaidEvidencePresent && opportunity.contraryEvidenceIds.every(id => isKnownEvidence(id, facts)) && channelFacts.length === test.channelEvidenceIds.length && channelFacts.some(fact => fact.signalType === 'distribution-access')
}

function isValidDistribution(item) {
  const requiredText = item && [item.candidateKey, item.channel, item.buyerPresence, item.ownerAccess, item.accessRequirements, item.costOrLimits, item.delay, item.allowedOffer, item.firstBuyerPath, item.commitmentSignal]
  return Boolean(requiredText && requiredText.every(value => isBoundedText(value, MAX_FIELD_LENGTH)) && Number.isFinite(item.accessTimeDays) && item.accessTimeDays >= 0 && item.accessTimeDays <= 7 && isNonEmptyArray(item.sourceRefs) && item.sourceRefs.length <= 4 && item.sourceRefs.every(isPublicUrl) && Array.isArray(item.limitations) && item.limitations.length <= 4 && item.limitations.every(value => isBoundedText(value, MAX_FIELD_LENGTH)))
}

function isCompleteReview(review, candidates) {
  if (!review || !Array.isArray(review.approvedIds) || !Array.isArray(review.rejected)) return false
  const candidateIds = new Set(candidates.map(candidate => candidate.id))
  const approvedIds = new Set(review.approvedIds)
  const rejectedIds = new Set(review.rejected.map(item => item && item.id).filter(isText))
  if (approvedIds.size !== review.approvedIds.length || rejectedIds.size !== review.rejected.length) return false
  if ([...approvedIds].some(id => !candidateIds.has(id) || rejectedIds.has(id))) return false
  return candidates.every(candidate => approvedIds.has(candidate.id) || rejectedIds.has(candidate.id))
}

function isValidCoverage(item) {
  return Boolean(item && isBoundedText(item.source, 500) && ['complete', 'partial', 'unavailable'].includes(item.status) && isBoundedText(item.window, 100) && Number.isFinite(item.itemsReviewed) && item.itemsReviewed >= 0 && (item.status !== 'complete' || item.itemsReviewed > 0) && typeof item.limitation === 'string' && item.limitation.length <= 1000)
}

const designFallbackName = source => {
  const normalized = source.toLowerCase().replaceAll('-', ' ').replace(/\s+\([^)]*\)$/, '')
  if (normalized === 'sidebar' || normalized.startsWith('sidebar.') || normalized.startsWith('sidebar ')) return 'sidebar'
  if (normalized.startsWith('smashing magazine') || normalized.startsWith('smashingmagazine.')) return 'smashing magazine'
  if (normalized.startsWith('ux collective') || normalized.startsWith('uxdesign.') || normalized.startsWith('medium.com/ux collective')) return 'ux collective'
  if (normalized.startsWith('design system news') || normalized.startsWith('design systems news')) return 'design system news'
  return null
}

function isDesignComplete(coverage) {
  const summaryRows = coverage.filter(item => item && isText(item.source) && item.source.toLowerCase().replaceAll('-', ' ') === 'design signals')
  if (summaryRows.some(item => item.status === 'complete')) return true
  if (summaryRows.length) return false
  const fallbacks = new Set(coverage.filter(item => item && item.status === 'complete' && isText(item.source)).map(item => designFallbackName(item.source)).filter(Boolean))
  return fallbacks.size >= 2
}

function isCoverageComplete(label, coverage) {
  if (label === 'design-signals') return isDesignComplete(coverage)
  const normalizedLabel = label.replaceAll('-', ' ')
  return coverage.some(item => item && isText(item.source) && item.status === 'complete' && item.source.toLowerCase().replaceAll('-', ' ') === normalizedLabel)
}

function boundedCoverage(label, coverage) {
  const validCoverage = coverage.filter(isValidCoverage).map(item => cleanCoverage(item, label))
  const normalizedLabel = label.replaceAll('-', ' ')
  const summaryRows = validCoverage.filter(item => item.source.toLowerCase().replaceAll('-', ' ') === normalizedLabel)
  const summary = summaryRows.find(item => item.status === 'complete') || summaryRows[0]
  const fallbackRows = label === 'design-signals' ? [...new Map(validCoverage.filter(item => item.status === 'complete').map(item => [designFallbackName(item.source), item]).filter(([name]) => name)).values()].slice(0, 2) : []
  const unavailableDesignRows = label === 'design-signals' ? validCoverage.filter(item => item !== summary && item.status === 'unavailable') : []
  const unavailableDesignSource = unavailableDesignRows.find(item => item.source.toLowerCase().replace(/[^a-z]/g, '').includes('designernews')) || unavailableDesignRows[0]
  const required = label === 'design-signals' ? [...new Set([summary, unavailableDesignSource, ...fallbackRows].filter(Boolean))] : summary ? [summary] : []
  const bounded = validCoverage.filter(item => !required.includes(item)).slice(0, Math.max(0, MAX_COVERAGE_PER_DISCOVERY - required.length))
  return [...bounded, ...required]
}

function renderCoverage(coverage) {
  const validCoverage = coverage.filter(isValidCoverage)
  if (!validCoverage.length) return '- none returned'
  return validCoverage.map(item => `- [${safeText(item.discoveryKind)}] ${safeText(item.source)}: ${item.status}; reviewed ${item.itemsReviewed}; limit: ${safeText(item.limitation || 'none')}`).join('\n')
}

function renderOpportunity(opportunity, facts) {
  const test = opportunity.test
  const distribution = opportunity.distributionResearch
  const evidence = ids => ids.map(id => facts.find(fact => fact.id === id)).filter(Boolean).map(fact => `${safeText(fact.id)} (${safeText(fact.signalType)}, ${fact.strength}; ${safeText(fact.source)}; ${safeText(fact.observedAt)}): ${safeText(fact.claim)}`).join('; ')
  return `### ${safeText(opportunity.name)}\n\n- Buyer: ${safeText(test.buyer)}\n- Paid pain: ${safeText(test.paidPain)}\n- Offer: ${safeText(test.offer)}\n- Why now: ${safeText(opportunity.marketTiming)}\n- Why the owner can win: ${safeText(opportunity.ownerFit)}\n- Confirming evidence: ${evidence(opportunity.confirmingEvidenceIds)}\n- Contrary evidence: ${evidence(opportunity.contraryEvidenceIds)}\n- Maturity: ${safeText(opportunity.maturity)}\n- Limitations: ${opportunity.limitations.map(safeText).join('; ')}\n- Distribution channel: ${safeText(test.distributionChannel)}\n- Buyer presence: ${safeText(distribution.buyerPresence)}\n- Owner access: ${safeText(distribution.ownerAccess)}\n- Access requirements: ${safeText(distribution.accessRequirements)}\n- Cost or limits: ${safeText(distribution.costOrLimits)}\n- Access delay: ${safeText(distribution.delay)} (${distribution.accessTimeDays} days)\n- Allowed offer: ${safeText(distribution.allowedOffer)}\n- First-buyer path: ${safeText(test.firstBuyerPath)}\n- Commitment signal: ${safeText(distribution.commitmentSignal)}\n- Initial channel evidence: ${test.channelEvidenceIds.map(safeText).join(', ')}\n- Reported distribution sources: ${distribution.sourceRefs.map(safeText).join(', ')}\n- Distribution limitations: ${distribution.limitations.map(safeText).join('; ') || 'none'}\n- Test: ${test.timeboxDays} days; smallest build: ${safeText(test.smallestBuild)}\n- Success signal: ${safeText(test.successSignal)}\n- Stop condition: ${safeText(test.stopCondition)}`
}

function renderDigest(status, recommendation, candidates, facts, coverage, drops, missingLabels, rawCandidateCount, survivorCount, expectedAgents, returnedAgents) {
  const title = `# Scheduled ideation — ${status}`
  const summary = status === 'recommendation-ready'
    ? `## Recommended action\n\n${renderOpportunity(recommendation, facts)}`
    : status === 'no-surviving-candidates'
      ? '## No surviving candidates\n\nComplete research found no opportunity that cleared the evidence, distribution, and skeptic gates.'
      : `## Incomplete research\n\nNo recommendation is available. Bounded retry scope: ${missingLabels.join(', ') || 'an unreported failure'}.`
  const alternatives = candidates.slice(1, 3).map(item => renderOpportunity(item, facts)).join('\n\n')
  return `${title}\n\n${summary}${alternatives ? `\n\n## Alternatives\n\n${alternatives}` : ''}\n\n## Discovery coverage\n\n${renderCoverage(coverage)}\n\n## Drops and failures\n\n- Drops: ${drops.length ? drops.map(safeText).join('; ') : 'none'}\n- Missing or failed: ${missingLabels.length ? missingLabels.map(safeText).join(', ') : 'none'}\n\n## Counts and caps\n\n- Discovery: ${DISCOVERY_LABELS.length} fixed jobs\n- Raw opportunities received: ${rawCandidateCount}; processing cap: ${MAX_OPPORTUNITIES}\n- Ranked survivors: ${survivorCount}/${MAX_SURVIVORS}\n- Complete agent results: ${returnedAgents}/${expectedAgents}\n- Agents: ${MAX_AGENTS} maximum\n\n## Human approval\n\nThis digest stops at recommendation. A human must approve any filing, buyer contact, purchase, publication, or execution.`
}

const run = async (prompt, options) => {
  try {
    return await agent(prompt, options)
  } catch (error) {
    log(`${options.label} failed: ${error instanceof Error ? error.message : String(error)}`)
    return null
  }
}

const discovery = [
  {
    label: 'owner-profile',
    prompt: `Build a bounded owner profile from at most the ten newest parent sessions and eight personal-memory results. Set discoveryKind=owner-profile on every fact. Return at most twenty aggregate facts and source references; do not return raw records, transcripts, paths, customer details, or private text. Cover demonstrated skills, interests, available time, and owner-fit constraints. Each fact needs id, discoveryKind, signalType, claim, source, observedAt, and direct/proxy/unknown strength. Return a summary coverage row whose source is owner-profile. This output stays internal and will never be sent to a web agent.`,
  },
  {
    label: 'active-commitments',
    prompt: `Read at most thirty closed issues, thirty merged pull requests, fifty commits, and two hundred recent project-state events. Set discoveryKind=active-commitments on every fact. Return at most twenty aggregate facts and source references; do not return raw private project or cross-project records. Identify active commitments, already-tracked work, owner capacity, and adjacent assets. Each fact needs id, discoveryKind, signalType, claim, source, observedAt, and direct/proxy/unknown strength. Return a summary coverage row whose source is active-commitments. This output stays internal and will never be sent to a web agent.`,
  },
  {
    label: 'hacker-news',
    prompt: `Research current public technical discussion only. Review at most forty Hacker News items and twenty relevant comments across top, new, show, ask, and jobs. Fetch at most eight linked pages. Attempt official Hacker News interfaces and feeds, then Algolia when needed. Record each attempted source and fallback in coverage. Return at most twenty evidence facts with id, discoveryKind=hacker-news, signalType, claim, source, observedAt, and strength direct/proxy/unknown. Return a summary coverage row whose source is hacker-news. Popularity is proxy evidence, not payment proof. Do not request or use private records.`,
  },
  {
    label: 'design-signals',
    prompt: `Research current public design discussion only. Review at most twenty items across no more than six sources. Attempt Designer News editions and discussions first. If access is denied, record the denial and use at least two named fallbacks from Sidebar, Smashing Magazine, UX Collective, and design-system news. Record every attempt and fallback in coverage. Return at most twelve evidence facts with id, discoveryKind=design-signals, signalType, claim, source, observedAt, and strength direct/proxy/unknown. Return a summary coverage row whose source is design-signals when the direct source or two named fallbacks complete the class. Do not request or use private records.`,
  },
  {
    label: 'broad-news',
    prompt: `Research current broad technology and business news only. Use a strict bounded pass: make no more than eight source attempts and stop after twenty reviewed items or the first access failure for a source. Prefer feeds, search-result pages, or official public interfaces that you can fetch directly; do not spend time retrying a blocked, paywalled, or unavailable publisher. Cover at least three independent publishers from Reuters, BBC, Ars Technica, TechCrunch, and company newsrooms when accessible. If a named publisher fails, record the failure and continue with another publisher or official regulator/newsroom source. When a claim depends on a company or regulator, include an applicable first-party source. Always return a result, even when no articles are available: facts may be empty, but coverage must include source attempts, statuses, reviewed item counts, and limitations, including any timeout or access failure. Return at most twenty evidence facts with id, discoveryKind=broad-news, signalType, claim, source, observedAt, and strength direct/proxy/unknown. Return a summary coverage row whose source is broad-news. Do not request or use private records.`,
  },
  {
    label: 'market-corroboration',
    prompt: `Research early market corroboration from at least four source families and no more than twelve sources. Review at most thirty items. Include commercial proof from first-party pricing, marketplace, case-study, or customer pages, plus one adoption or work-demand source. Use applicable public sources from Product Hunt, Show Hacker News, GitHub Trending/releases/activity/discussions, npm/PyPI/crates.io, Stack Overflow, arXiv/OpenAlex/Crossref, Upwork/public jobs, SAM.gov/TED, YC/funding/directories, Federal Register/EU/regulators, and commercial proof pages. Label funding and popularity as proxy; label work demand by what it directly establishes. Record every source family and limitation in coverage. Return at most twenty-four evidence facts with id, discoveryKind=market-corroboration, signalType, claim, source, observedAt, and strength direct/proxy/unknown. Return a summary coverage row whose source is market-corroboration. Do not request or use private records.`,
  },
]

phase('Discovery')
const discoveryResults = await parallel(discovery.map(job => () => run(`${READ_ONLY_BOUNDARY}\n\nPrefix every evidence id with ${job.label}: so ids stay unique across parallel jobs. Use YYYY-MM-DD for every observedAt value. Use distribution-access for observed buyer or channel access. Use only these signalType values: ${EVIDENCE_SIGNAL_TYPES.join(', ')}. Each coverage row needs source, status, a nonempty window of at most 100 characters, numeric itemsReviewed, and a limitation of at most 1000 characters. Mark the summary coverage row complete when the stated minimum coverage succeeds, even if an extra source fails. Mark it partial only when the minimum does not succeed.\n\n${job.prompt}`, {
  label: job.label,
  phase: 'Discovery',
  ...(EXTERNAL_LABELS.has(job.label) ? { agentType: 'web-research-summarizer' } : {}),
  schema: DISCOVERY_SCHEMA,
})))
const boundedDiscoveryCoverage = discoveryResults.map((result, index) => result && Array.isArray(result.coverage) ? boundedCoverage(discovery[index].label, result.coverage) : [])
const returnedDiscovery = boundedDiscoveryCoverage.map((items, index) => isCoverageComplete(discovery[index].label, items) ? discovery[index].label : null).filter(Boolean)
const coverage = boundedDiscoveryCoverage.flat()
const rawFacts = discoveryResults.flatMap((result, index) => result && Array.isArray(result.facts) ? result.facts.filter(isValidEvidence).slice(0, FACT_LIMITS[discovery[index].label]).map(fact => cleanFact(fact, discovery[index].label)) : [])
const factIdCounts = new Map()
for (const fact of rawFacts) factIdCounts.set(fact.id, (factIdCounts.get(fact.id) || 0) + 1)
const duplicateFactIds = [...factIdCounts].filter(([, count]) => count > 1).map(([id]) => id)
const facts = rawFacts.filter(fact => factIdCounts.get(fact.id) === 1)
const missingLabels = DISCOVERY_LABELS.filter(label => !returnedDiscovery.includes(label))

phase('Generate')
const generator = await run(`${READ_ONLY_BOUNDARY}\n\nGenerate at most ${MAX_OPPORTUNITIES} distinct market opportunities from these bounded aggregate internal and external facts. Return only complete opportunity records. Name a buyer category, never a private person, company, customer, or project from internal evidence. Each record needs paid pain, owner fit, marketTimingEvidenceIds for two external market facts with different observation dates, ownerFitEvidenceIds that cite owner-fit, capability, or active-commitment facts from owner-profile or active-commitments, three confirming facts from three distinct sources across at least two signal types, at least two public market sources, and one direct public paid-pain/budget/adoption/commitment fact, contrary evidence, maturity as early, emerging, or mature-with-supported-wedge, limitations, and a test lasting one to seven days. The test needs buyer, paid pain, offer, accessible distribution channel, first-buyer path, channel evidence identifiers, smallest build, measurable paid commitment signal, and stop condition. Describe every proposed action as a noun phrase or conditional test, never as an instruction to the owner. Reject invented forecasts, inaccessible buyers, missing distribution, and attention-only proof.\n\nFacts:\n${JSON.stringify({ facts, coverage })}`, {
  label: 'opportunity-generator',
  phase: 'Generate',
  schema: { type: 'object', properties: { candidates: { type: 'array', maxItems: MAX_OPPORTUNITIES, items: OPPORTUNITY_SCHEMA } }, required: ['candidates'] },
})
const isGeneratorComplete = Boolean(generator && Array.isArray(generator.candidates))
const generated = isGeneratorComplete ? generator.candidates : []
const unique = []
const ids = new Set()
let duplicateOpportunityCount = 0
let invalidOpportunityCount = 0
let inspectedOpportunityCount = 0
const opportunityIdCollisions = new Set()
const drops = duplicateFactIds.map(id => `${id}: duplicate evidence identifier removed`)
for (const opportunity of generated.slice(0, MAX_GENERATED_RECORDS)) {
  inspectedOpportunityCount += 1
  if (!isValidOpportunity(opportunity, facts)) {
    invalidOpportunityCount += 1
    continue
  }
  if (unique.length >= MAX_OPPORTUNITIES) break
  const cleaned = cleanOpportunity(opportunity)
  const id = cleaned.id
  if (ids.has(id)) {
    const existing = unique.find(item => item.id === id)
    if (canonicalJson(existing) === canonicalJson(cleaned)) duplicateOpportunityCount += 1
    else opportunityIdCollisions.add(id)
  } else {
    ids.add(id)
    unique.push(cleaned)
  }
}
if (invalidOpportunityCount) drops.push(`${invalidOpportunityCount} opportunity record${invalidOpportunityCount === 1 ? '' : 's'} failed validation`)
if (duplicateOpportunityCount) drops.push(`${duplicateOpportunityCount} exact duplicate opportunity record${duplicateOpportunityCount === 1 ? '' : 's'} removed`)
if (generated.length > MAX_OPPORTUNITIES) drops.push(`raw opportunity limit exceeded; received ${generated.length}, inspected ${inspectedOpportunityCount}, accepted ${unique.length}`)
const valid = unique

phase('Distribution')
const candidateIdByKey = new Map()
const distributionCandidates = valid.map((opportunity, index) => {
  const candidateKey = `c${index + 1}`
  candidateIdByKey.set(candidateKey, opportunity.id)
  return {
    candidateKey,
    publicEvidence: opportunity.confirmingEvidenceIds.map(id => facts.find(fact => fact.id === id)).filter(fact => fact && EXTERNAL_LABELS.has(fact.discoveryKind)).map(fact => ({ claim: fact.claim, source: fact.source, observedAt: fact.observedAt, signalType: fact.signalType })),
  }
})
const isDistributionNeeded = valid.length > 0
const distributionResult = isDistributionNeeded ? await run(`${READ_ONLY_BOUNDARY}\n\nResearch candidate-specific public distribution for every opportunity below. Use at most twelve public sources across the complete candidate set. Receive only opaque candidate keys and public evidence, with no generator-authored proposal text or private raw session, memory, customer, project, or cross-project record. Infer a public buyer category and allowed offer from that evidence. Compare direct owner access, Upwork or a paid marketplace, GitHub Marketplace or an integration store, Show Hacker News or Product Hunt, platform marketplaces, partners, buyer-specific outbound, and search/publication where applicable. Return candidateKey, channel, buyerPresence, ownerAccess, accessRequirements, costOrLimits, delay, accessTimeDays, allowedOffer, firstBuyerPath, commitmentSignal, at most four fetched public URL sourceRefs, and limitations. Describe routes and offers as noun phrases, never as instructions to the owner. Attention such as views, votes, stars, and replies is not payment proof. Show Hacker News requires a usable project; a landing page is not enough. GitHub Marketplace is a later channel because of verification, installation, transaction, and payout limits. Return one record per candidate.\n\nCandidates:\n${JSON.stringify(distributionCandidates)}`, {
  label: 'distribution-research',
  phase: 'Distribution',
  agentType: 'web-research-summarizer',
  schema: { type: 'object', properties: { distribution: { type: 'array', items: DISTRIBUTION_SCHEMA } }, required: ['distribution'] },
}) : { distribution: [] }
const isDistributionResultComplete = Boolean(distributionResult && Array.isArray(distributionResult.distribution))
const validIds = new Set(valid.map(opportunity => opportunity.id))
const distribution = []
const distributionIds = new Set()
for (const item of isDistributionResultComplete ? distributionResult.distribution.slice(0, MAX_OPPORTUNITIES * 2) : []) {
  if (distribution.length >= MAX_OPPORTUNITIES) break
  const opportunityId = item && candidateIdByKey.get(item.candidateKey)
  if (isValidDistribution(item) && validIds.has(opportunityId) && !distributionIds.has(opportunityId)) {
    distributionIds.add(opportunityId)
    distribution.push(cleanDistribution(item, opportunityId))
  }
}
const distributionById = new Map(distribution.map(item => [item.opportunityId, item]))
const hasUsableDistribution = opportunity => distributionById.has(opportunity.id) && distributionById.get(opportunity.id).accessTimeDays <= opportunity.test.timeboxDays
const isDistributionComplete = isDistributionResultComplete && valid.every(hasUsableDistribution)
drops.push(...valid.filter(opportunity => !hasUsableDistribution(opportunity)).map(opportunity => `${opportunity.id}: distribution research did not return a valid record within the test window`))
const enriched = valid.filter(hasUsableDistribution).map(opportunity => {
  const distributionResearch = distributionById.get(opportunity.id)
  return {
    ...opportunity,
    test: {
      ...opportunity.test,
      distributionChannel: distributionResearch.channel,
      firstBuyerPath: distributionResearch.firstBuyerPath,
    },
    distributionResearch,
  }
})
phase('Verify')
const skepticPrompt = JSON.stringify({ candidates: enriched, facts, coverage })
const isSkepticReviewNeeded = enriched.length > 0
const [marketSkeptic, ownerSkeptic] = isSkepticReviewNeeded ? await parallel([
  () => run(`${READ_ONLY_BOUNDARY}\n\nYou are a fresh market skeptic. You did not generate these candidates and receive only bounded evidence and enriched candidates, never hidden reasoning or private raw records. Approve only candidates with dated market timing, paid pain/budget/adoption/buyer commitment, three independent facts across two signal types, explicit contrary evidence, credible sources, and no mature market without a supported wedge, invented forecast, attention mistaken for payment, or instruction to the owner. Account for every candidate in either approvedIds or rejected. Return each rejection as an id and reason.\n\n${skepticPrompt}`, { label: 'market-skeptic', phase: 'Verify', schema: REVIEW_SCHEMA }),
  () => run(`${READ_ONLY_BOUNDARY}\n\nYou are a fresh owner-fit skeptic. You did not generate these candidates and receive only bounded evidence and enriched candidates, never hidden reasoning or private raw records. Approve only candidates with demonstrated owner fit, compatible commitments, accessible buyer access, a concrete first-buyer path, channel evidence, a seven-day-or-less smallest build, and a measurable commitment signal. Reject inaccessible buyers, missing channel access, or instructions to the owner. Account for every candidate in either approvedIds or rejected. Return each rejection as an id and reason.\n\n${skepticPrompt}`, { label: 'owner-fit-skeptic', phase: 'Verify', schema: REVIEW_SCHEMA }),
]) : [{ approvedIds: [], rejected: [] }, { approvedIds: [], rejected: [] }]
const isMarketReviewComplete = isCompleteReview(marketSkeptic, enriched)
const isOwnerReviewComplete = isCompleteReview(ownerSkeptic, enriched)
const approvedByMarket = new Set(isMarketReviewComplete ? marketSkeptic.approvedIds : [])
const approvedByOwner = new Set(isOwnerReviewComplete ? ownerSkeptic.approvedIds : [])
const approved = enriched.filter(opportunity => approvedByMarket.has(opportunity.id) && approvedByOwner.has(opportunity.id))
if (isMarketReviewComplete) drops.push(...enriched.filter(opportunity => !approvedByMarket.has(opportunity.id)).map(opportunity => `${opportunity.id}: market skeptic rejected`))
if (isOwnerReviewComplete) drops.push(...enriched.filter(opportunity => !approvedByOwner.has(opportunity.id)).map(opportunity => `${opportunity.id}: owner-fit skeptic rejected`))

phase('Rank')
const rankCandidates = approved
const isRankNeeded = rankCandidates.length > 0
const ranked = isRankNeeded ? await run(`${READ_ONLY_BOUNDARY}\n\nYou are a fresh final ranker. Receive only bounded candidates that both skeptics approved, not generator transcripts or hidden reasoning. Return only ordered identifiers from this approved set. Collapse same-outcome ideas by keeping one identifier, and return at most ${MAX_SURVIVORS} survivorIds. Put every omitted or merged identifier in droppedIds and explain it in dropReasons. For each same-outcome set, return a duplicateGroups record with keptId, droppedIds, and reason. Rank first by direct payment evidence, accessible buyer reach, market timing, owner fit, and the speed of a bounded paid test. Use market size only after those factors. Never rewrite a candidate or invent forecasts or certainty.\n\nApproved candidates:\n${JSON.stringify(rankCandidates)}`, {
  label: 'rank',
  phase: 'Rank',
  schema: RANKED_SCHEMA,
}) : { survivorIds: [], droppedIds: [], droppedCount: 0, dropReasons: [], duplicateGroups: [] }
const rankCandidateIds = new Set(rankCandidates.map(candidate => candidate.id))
const isRankShapeValid = Boolean(ranked && Array.isArray(ranked.survivorIds) && Array.isArray(ranked.droppedIds) && Array.isArray(ranked.dropReasons) && Array.isArray(ranked.duplicateGroups))
const duplicateGroupDroppedIds = isRankShapeValid ? ranked.duplicateGroups.flatMap(group => group && Array.isArray(group.droppedIds) ? group.droppedIds : []) : []
const isDuplicateGroupsValid = Boolean(isRankShapeValid && new Set(duplicateGroupDroppedIds).size === duplicateGroupDroppedIds.length && ranked.duplicateGroups.every(group => group && rankCandidateIds.has(group.keptId) && ranked.survivorIds.includes(group.keptId) && Array.isArray(group.droppedIds) && group.droppedIds.every(id => rankCandidateIds.has(id) && ranked.droppedIds.includes(id))))
const isRankComplete = Boolean(isRankShapeValid && isDuplicateGroupsValid && ranked.survivorIds.length <= MAX_SURVIVORS && new Set(ranked.survivorIds).size === ranked.survivorIds.length && new Set(ranked.droppedIds).size === ranked.droppedIds.length && ranked.droppedCount === ranked.droppedIds.length && ranked.survivorIds.length + ranked.droppedIds.length === rankCandidateIds.size && [...ranked.survivorIds, ...ranked.droppedIds].every(id => rankCandidateIds.has(id)) && ranked.survivorIds.every(id => !ranked.droppedIds.includes(id)))
const approvedById = new Map(approved.map(opportunity => [opportunity.id, opportunity]))
const rankedCandidates = []
const rankedIds = new Set()
for (const id of isRankComplete ? ranked.survivorIds : []) {
  if (approvedById.has(id) && !rankedIds.has(id) && rankedCandidates.length < MAX_SURVIVORS) {
    const duplicateIds = ranked.duplicateGroups.filter(group => group.keptId === id).flatMap(group => group.droppedIds)
    const duplicates = duplicateIds.map(duplicateId => approvedById.get(duplicateId)).filter(Boolean)
    const kept = approvedById.get(id)
    rankedIds.add(id)
    rankedCandidates.push({
      ...kept,
      confirmingEvidenceIds: [...new Set([kept, ...duplicates].flatMap(item => item.confirmingEvidenceIds))].slice(0, 8),
      contraryEvidenceIds: [...new Set([kept, ...duplicates].flatMap(item => item.contraryEvidenceIds))].slice(0, 4),
      limitations: [...new Set([kept, ...duplicates].flatMap(item => item.limitations))].slice(0, 4),
    })
  }
}
const stageFailures = [!isGeneratorComplete && 'opportunity-generator', isDistributionNeeded && !isDistributionComplete && 'distribution-research', isSkepticReviewNeeded && !isMarketReviewComplete && 'market-skeptic', isSkepticReviewNeeded && !isOwnerReviewComplete && 'owner-fit-skeptic', isRankNeeded && !isRankComplete && 'rank'].filter(Boolean)
const evidenceFailures = duplicateFactIds.length ? ['evidence-id-collision'] : []
const opportunityFailures = [...(opportunityIdCollisions.size ? ['opportunity-id-collision'] : []), ...(generated.length > MAX_GENERATED_RECORDS ? ['generator-output-truncated'] : [])]
const failures = [...missingLabels, ...evidenceFailures, ...opportunityFailures, ...stageFailures]
const status = failures.length ? 'incomplete-research' : rankedCandidates.length ? 'recommendation-ready' : 'no-surviving-candidates'
const finalCandidates = status === 'recommendation-ready' ? rankedCandidates : []
const recommendation = finalCandidates[0] || null
const rankDrops = isRankComplete ? approved.filter(opportunity => !rankedIds.has(opportunity.id)).map(opportunity => `${opportunity.id}: ranker omitted or merged`) : []
const finalDrops = [...drops, ...rankDrops]
const finalCoverage = coverage
const expectedAgents = DISCOVERY_LABELS.length + 1 + (isDistributionNeeded ? 1 : 0) + (isSkepticReviewNeeded ? 2 : 0) + (isRankNeeded ? 1 : 0)
const returnedAgents = returnedDiscovery.length + (isGeneratorComplete ? 1 : 0) + (isDistributionNeeded && isDistributionComplete ? 1 : 0) + (isSkepticReviewNeeded && isMarketReviewComplete ? 1 : 0) + (isSkepticReviewNeeded && isOwnerReviewComplete ? 1 : 0) + (isRankNeeded && isRankComplete ? 1 : 0)
const digest = renderDigest(status, recommendation, finalCandidates, facts, finalCoverage, finalDrops, failures, generated.length, finalCandidates.length, expectedAgents, returnedAgents)

return {
  status,
  recommendation,
  candidates: finalCandidates,
  digest,
  coverage: finalCoverage,
  expected: expectedAgents,
  returned: returnedAgents,
  missingLabels: failures,
  rawCandidateCount: generated.length,
  survivorCount: finalCandidates.length,
}
