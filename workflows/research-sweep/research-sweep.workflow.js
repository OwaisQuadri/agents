export const meta = {
  name: 'research-sweep',
  description: 'Fan out web + codebase research agents over a goal, gap-check with a fresh critic, fill gaps',
  whenToUse: 'A research question needing several external angles (web, academic, news, design/UX) plus repo context read, compared, and condensed into cited findings blocks. Returns the blocks; the caller synthesizes.',
  phases: [
    { title: 'Plan', detail: 'one agent turns the goal into self-contained web + codebase dispatches' },
    { title: 'Research', detail: 'one researcher per dispatch, in parallel', model: 'sonnet' },
    { title: 'Gap check', detail: 'fresh-context completeness critic over all findings blocks' },
    { title: 'Fill gaps', detail: 'researchers for angles the critic found missing', model: 'sonnet' },
  ],
}

const GOAL = typeof args === 'string' ? args : args && args.goal
if (!GOAL) return { error: 'missing input: goal' }
const MAX_PLANNED = Math.min((args && args.max_researchers) || 6, 6)
const MAX_CODEBASE = Math.min((args && args.max_codebase) || 2, 2)
const MAX_FILL = 3
const INCLUDE_CODEBASE = (args && args.includeCodebase) !== false

// `label` is declared but never reaches a dispatched researcher's prompt (see
// dispatchPrompt below) — it stays orchestration metadata for agent()'s own
// {label, phase, agentType} opts. All five fields are required here because this
// schema governs the PLAN NODE's own output, not the researcher's contract:
// agents/web-research-summarizer/web-research-summarizer.md marks only `objective`
// required, with its own defaults for the rest. The two required-lists differ on
// purpose, not drift.
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

// Checks a response against agents/web-research-summarizer/web-research-summarizer.md's
// output contract: one ```findings fence with objective:, findings:, a claim/source pair,
// gaps:, and sources: fetched=N cited=M. "nothing outside it" there isn't literal — that
// agent's logging section mandates a separate trailing ```log fence — so this only rejects
// a truncated block (missing a required part) or a second ```findings fence, never
// surrounding text. Scoped to web-research-summarizer: Explore (codebase dispatches) has
// no repo-owned contract to check against.
function isValidFindingsBlock(text) {
  const fenceStart = text.indexOf('```findings')
  if (fenceStart === -1) return false
  const fenceEnd = text.indexOf('```', fenceStart + '```findings'.length)
  if (fenceEnd === -1) return false
  const body = text.slice(fenceStart, fenceEnd)
  if (text.indexOf('```findings', fenceEnd) !== -1) return false
  return /^objective:/m.test(body)
    && /^findings:/m.test(body)
    && /^\s*-\s*claim:/m.test(body)
    && /^\s*source:/m.test(body)
    && /^gaps:/m.test(body)
    && /^sources:\s*fetched=\d+\s*cited=\d+/m.test(body)
}

phase('Plan')
const plan = await agent(
  `You are the planning node of a research sweep. The research goal, verbatim: "${GOAL}"

Design between 3 and ${MAX_PLANNED} WEB research dispatches that jointly cover this goal from distinct angles — use different source modalities where the goal allows: official documentation, repositories and changelogs, community discussion, hands-on comparisons, academic/research papers, news coverage, and design/UX(user experience)/UI(user interface) critique or prior art. Pick whichever modalities actually apply to this goal; never force a modality that doesn't fit. Each dispatch goes to a fresh-context web researcher who sees NOTHING but the four content fields — not this conversation, not the goal as you phrase it here, not the sibling dispatches — so every objective must be verbose and self-contained, restating all context that researcher needs, including what decision the research serves.

${INCLUDE_CODEBASE ? `Also decide whether this goal needs CODEBASE context (what exists in the repo today, how something is currently implemented, what a change would touch). If it does, design 1-${MAX_CODEBASE} codebase dispatches using the SAME field shape — source_guidance names the directories/file patterns to look in, recency is "current repo state". If the goal is purely external (nothing about this specific codebase), return an empty codebase_dispatches array — never invent a codebase angle a goal doesn't need.` : 'codebase dispatches are disabled for this run; return an empty codebase_dispatches array.'}

Fields per dispatch (web or codebase):
- label: short kebab-case name
- objective: the research question, specific enough that "answered" is checkable
- boundaries: what is out of scope
- source_guidance: starting URLs/domains/search venues (web) or directories/file patterns (codebase)
- recency: the freshness bound sources must meet (web), or "current repo state" (codebase)`,
  {
    label: 'plan',
    schema: {
      type: 'object',
      properties: {
        dispatches: { type: 'array', items: DISPATCH_SCHEMA },
        codebase_dispatches: { type: 'array', items: DISPATCH_SCHEMA },
      },
      required: ['dispatches', 'codebase_dispatches'],
    },
  })

if (!plan || !plan.dispatches || !plan.dispatches.length) return { goal: GOAL, error: 'plan node returned nothing; no researchers dispatched' }
const planned = plan.dispatches.slice(0, MAX_PLANNED)
const plannedCodebase = INCLUDE_CODEBASE ? (plan.codebase_dispatches || []).slice(0, MAX_CODEBASE) : []
log(`planned ${planned.length} web dispatch(es): ${planned.map(d => d.label).join(', ')}${plannedCodebase.length ? `; ${plannedCodebase.length} codebase dispatch(es): ${plannedCodebase.map(d => d.label).join(', ')}` : ''}`)

const dispatchPrompt = d =>
  `objective: ${d.objective}\nboundaries: ${d.boundaries}\nsource_guidance: ${d.source_guidance}\nrecency: ${d.recency}`

// A malformed response is corrected in place via `resume` on the SAME child — never a
// fresh dispatch through the gap-check/fill-gap round, which stays reserved for an angle
// nobody covered at all. `resume` can't combine with agentType/model/schema, so the
// correction call omits agentType. Still-malformed after one correction is treated as
// missing, same as no response.
async function dispatchAndCheck(d, phaseTitle, agentType) {
  const text = await agent(dispatchPrompt(d), { label: d.label, phase: phaseTitle, agentType })
  if (!text) return null
  if (agentType !== 'web-research-summarizer' || isValidFindingsBlock(text)) return { label: d.label, text }
  const corrected = await agent(
    'Your last response did not match the required output contract: exactly one ```findings fenced block, complete, with objective:, findings: (at least one claim: paired with a source: line), gaps:, and sources: fetched=N cited=M. Resend the complete, corrected findings block — your closing ```log block still goes after it as usual, per your own logging contract.',
    { label: d.label, phase: phaseTitle, resume: d.label })
  return (corrected && isValidFindingsBlock(corrected)) ? { label: d.label, text: corrected } : null
}

const runResearchers = (dispatches, phaseTitle, agentType) => parallel(dispatches.map(d => () => dispatchAndCheck(d, phaseTitle, agentType)))

// Web and codebase round-1 dispatches share nothing — a real fan-out, not a false edge —
// so they run inside one combined wave rather than two sequential barriers.
const [webRound1, codebaseRound1] = await Promise.all([
  runResearchers(planned, 'Research', 'web-research-summarizer'),
  plannedCodebase.length ? runResearchers(plannedCodebase, 'Research', 'Explore') : Promise.resolve([]),
])
const round1Blocks = [...webRound1, ...codebaseRound1].filter(Boolean)
const round1Planned = [...planned, ...plannedCodebase]
const round1Missing = round1Planned.map(d => d.label).filter(l => !round1Blocks.some(b => b.label === l))
log(`research round: ${round1Blocks.length}/${round1Planned.length} findings blocks returned${round1Missing.length ? `, missing: ${round1Missing.join(', ')}` : ''}`)

const critic = await agent(
  `You are a completeness critic for a research sweep. The research goal, verbatim: "${GOAL}"

Below are ${round1Blocks.length} cited findings blocks from parallel researchers (web and, where relevant, codebase), each covering one angle.${round1Missing.length ? ` These planned angles returned nothing and may need re-dispatching: ${round1Missing.join(', ')}.` : ''} Your job: judge whether the blocks, taken together, answer the goal well enough to act on, and name what is MISSING — an angle nobody covered, a dimension nobody answered, or a contradiction between blocks that needs a tie-breaking source. Do NOT re-research anything yourself; only judge coverage.

Return isSufficient=true when the gaps are minor enough that further research would not change what the caller does with the answer. Otherwise return up to ${MAX_FILL} missing entries, each a complete research dispatch with: label (short kebab-case), objective (a specific research question, checkable as answered), boundaries (what is out of scope), source_guidance (where to look — web or codebase), recency (freshness bound). Write each objective verbose and self-contained — the researcher who receives it sees NOTHING else, not these blocks, not this conversation.

${round1Blocks.map(b => `=== findings block: ${b.label} ===\n${b.text}`).join('\n\n')}`,
  { phase: 'Gap check', schema: { type: 'object', properties: { isSufficient: { type: 'boolean' }, missing: { type: 'array', items: DISPATCH_SCHEMA }, notes: { type: 'string' } }, required: ['isSufficient', 'missing', 'notes'] } })

let round2Blocks = []
let round2Missing = []
if (critic && !critic.isSufficient && critic.missing.length) {
  const fill = critic.missing.slice(0, MAX_FILL)
  log(`critic found ${fill.length} gap(s): ${fill.map(m => m.label).join(', ')}`)
  // Fill dispatches route to web-research-summarizer by default: the critic names gaps
  // in claims, and a codebase gap almost always surfaces as a missing web angle already
  // covered by round 1's codebase pass. Fill stays single-modality to keep the loop simple.
  const round2 = await runResearchers(fill, 'Fill gaps', 'web-research-summarizer')
  round2Blocks = round2.filter(Boolean)
  round2Missing = fill.map(d => d.label).filter(l => !round2Blocks.some(b => b.label === l))
} else {
  log(critic ? 'critic judged coverage sufficient' : 'critic returned nothing; report carries round 1 only, unverified')
}

const expected = round1Planned.length + round2Blocks.length + round2Missing.length
const blocks = [...round1Blocks, ...round2Blocks]
return {
  goal: GOAL,
  blocks,
  criticNotes: critic ? critic.notes : 'critic returned nothing; coverage unverified',
  expected,
  returned: blocks.length,
  missingLabels: [...round1Missing, ...round2Missing],
}
