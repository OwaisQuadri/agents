export const meta = {
  name: 'research-sweep',
  description: 'Fan out web-research-summarizer agents over a research goal, gap-check with a fresh critic, fill gaps',
  whenToUse: 'A research question needing several external angles read, compared, and condensed into cited findings blocks. Returns the blocks; the caller synthesizes.',
  phases: [
    { title: 'Plan', detail: 'one agent turns the goal into self-contained dispatches' },
    { title: 'Research', detail: 'one web-research-summarizer per dispatch, in parallel', model: 'sonnet' },
    { title: 'Gap check', detail: 'fresh-context completeness critic over all findings blocks' },
    { title: 'Fill gaps', detail: 'researchers for angles the critic found missing', model: 'sonnet' },
  ],
}

const GOAL = typeof args === 'string' ? args : args && args.goal
if (!GOAL) return { error: 'missing input: goal' }
const MAX_PLANNED = Math.min((args && args.max_researchers) || 6, 6)
const MAX_FILL = 3

const DISPATCH_SCHEMA = {
  type: 'object',
  properties: {
    label: { type: 'string' },
    objective: { type: 'string' },
    boundaries: { type: 'string' },
    source_guidance: { type: 'string' },
    recency: { type: 'string' },
  },
  required: ['label', 'objective', 'boundaries', 'source_guidance', 'recency'],
}

phase('Plan')
const plan = await agent(
  `You are the planning node of a research sweep. The research goal, verbatim: "${GOAL}"

Design between 3 and ${MAX_PLANNED} research dispatches that jointly cover this goal from distinct angles — use different source modalities where the goal allows (official documentation, repositories and changelogs, community discussion, hands-on comparisons). Each dispatch goes to a fresh-context web researcher who sees NOTHING but the four content fields — not this conversation, not the goal as you phrase it here, not the sibling dispatches — so every objective must be verbose and self-contained, restating all context that researcher needs, including what decision the research serves. Fields per dispatch:
- label: short kebab-case name
- objective: the research question, specific enough that "answered" is checkable
- boundaries: what is out of scope
- source_guidance: starting URLs, preferred domains, or search venues
- recency: the freshness bound sources must meet, matched to how fast the topic moves (e.g. "within 12 months; flag anything older with a stale line")`,
  { label: 'plan', schema: { type: 'object', properties: { dispatches: { type: 'array', items: DISPATCH_SCHEMA } }, required: ['dispatches'] } })

if (!plan || !plan.dispatches || !plan.dispatches.length) return { goal: GOAL, error: 'plan node returned nothing; no researchers dispatched' }
const planned = plan.dispatches.slice(0, MAX_PLANNED)
log(`planned ${planned.length} dispatches: ${planned.map(d => d.label).join(', ')}`)

const runResearchers = (dispatches, phaseTitle) => parallel(dispatches.map(d => () =>
  agent(
    `objective: ${d.objective}\nboundaries: ${d.boundaries}\nsource_guidance: ${d.source_guidance}\nrecency: ${d.recency}`,
    { label: d.label, phase: phaseTitle, agentType: 'web-research-summarizer' })
    .then(text => (text ? { label: d.label, text } : null))))

const round1 = await runResearchers(planned, 'Research')
const round1Blocks = round1.filter(Boolean)
const round1Missing = planned.map(d => d.label).filter(l => !round1Blocks.some(b => b.label === l))
log(`research round: ${round1Blocks.length}/${planned.length} findings blocks returned${round1Missing.length ? `, missing: ${round1Missing.join(', ')}` : ''}`)

const critic = await agent(
  `You are a completeness critic for a research sweep. The research goal, verbatim: "${GOAL}"

Below are ${round1Blocks.length} cited findings blocks from parallel web researchers, each covering one angle.${round1Missing.length ? ` These planned angles returned nothing and may need re-dispatching: ${round1Missing.join(', ')}.` : ''} Your job: judge whether the blocks, taken together, answer the goal well enough to act on, and name what is MISSING — an angle nobody covered, a dimension nobody answered, or a contradiction between blocks that needs a tie-breaking source. Do NOT re-research anything yourself; only judge coverage.

Return isSufficient=true when the gaps are minor enough that further research would not change what the caller does with the answer. Otherwise return up to ${MAX_FILL} missing entries, each a complete research dispatch with: label (short kebab-case), objective (a specific research question, checkable as answered), boundaries (what is out of scope), source_guidance (where to look), recency (freshness bound). Write each objective verbose and self-contained — the researcher who receives it sees NOTHING else, not these blocks, not this conversation.

${round1Blocks.map(b => `=== findings block: ${b.label} ===\n${b.text}`).join('\n\n')}`,
  { phase: 'Gap check', schema: { type: 'object', properties: { isSufficient: { type: 'boolean' }, missing: { type: 'array', items: DISPATCH_SCHEMA }, notes: { type: 'string' } }, required: ['isSufficient', 'missing', 'notes'] } })

let round2Blocks = []
let round2Missing = []
if (critic && !critic.isSufficient && critic.missing.length) {
  const fill = critic.missing.slice(0, MAX_FILL)
  log(`critic found ${fill.length} gap(s): ${fill.map(m => m.label).join(', ')}`)
  const round2 = await runResearchers(fill, 'Fill gaps')
  round2Blocks = round2.filter(Boolean)
  round2Missing = fill.map(d => d.label).filter(l => !round2Blocks.some(b => b.label === l))
} else {
  log(critic ? 'critic judged coverage sufficient' : 'critic returned nothing; report carries round 1 only, unverified')
}

const expected = planned.length + round2Blocks.length + round2Missing.length
const blocks = [...round1Blocks, ...round2Blocks]
return {
  goal: GOAL,
  blocks,
  criticNotes: critic ? critic.notes : 'critic returned nothing; coverage unverified',
  expected,
  returned: blocks.length,
  missingLabels: [...round1Missing, ...round2Missing],
}
