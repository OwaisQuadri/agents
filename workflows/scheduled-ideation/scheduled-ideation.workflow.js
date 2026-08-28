export const meta = {
  name: 'scheduled-ideation',
  description: 'Daily fan-out that proposes candidate skills, agents, and external tools to adopt, filters with a fresh critic, then synthesizes one markdown digest',
  whenToUse: "Triggered by the scheduled-ideation launchd job's seeded kickoff prompt inside a fresh daily Herdr worktree. Returns a digest; the caller writes it to .context/scheduled-ideation-digest.md — this workflow has no filesystem access of its own.",
  phases: [
    { title: 'Plan', detail: "one agent turns the fixed daily mission into self-contained mining + radar dispatches, rotating tool-radar sources so runs don't repeat the same search every day" },
    { title: 'Generate', detail: 'one agent per angle: session-evidence mining (skills), agent-candidate scan, external-tool radar', model: 'sonnet' },
    { title: 'Filter', detail: 'fresh-context critic scores every raw candidate against an evidence rubric, drops noise' },
    { title: 'Digest', detail: 'one agent writes the final categorized markdown digest from survivors' },
  ],
}

// Mission is fixed day to day — no required args. An optional args.focus note lets a
// one-off manual run steer the tool-radar angle (e.g. "focus on Rust tooling this run");
// the daily 3pm trigger calls this with no args.
const FOCUS_NOTE = (args && typeof args === 'object' && args.focus) ? args.focus : null
const MAX_TOOL_RADAR = Math.min((args && args.max_tool_radar) || 3, 3)
const MAX_DIGEST_CANDIDATES = 10

const DISPATCH_SCHEMA = {
  type: 'object',
  properties: {
    label: { type: 'string' },
    category: { type: 'string', enum: ['skill', 'agent', 'tool'] },
    objective: { type: 'string' },
    boundaries: { type: 'string' },
    source_guidance: { type: 'string' },
    recency: { type: 'string' },
  },
  required: ['label', 'category', 'objective', 'boundaries', 'source_guidance', 'recency'],
}

// The candidate's own destination category is ai-author's actual type-tree outcome, not
// the coarser skill/agent/tool bucket a dispatch was routed under (a skill-evidence-sweep
// dispatch can legitimately conclude "checker" or "workflow" once it applies the "should
// it exist?" test) — this is what lets the digest literally answer "which linters, checks,
// skills, agents, workflows, or other", instead of force-fitting everything non-agent into
// a single generic "skill" or "tool" bucket.
const CANDIDATE_SCHEMA = {
  type: 'object',
  properties: {
    name: { type: 'string' },
    category: { type: 'string', enum: ['skill', 'agent', 'workflow', 'checker', 'extension', 'tool'] },
    rationale: { type: 'string' },
    evidence: { type: 'string' },
    source: { type: 'string' },
  },
  required: ['name', 'category', 'rationale', 'evidence', 'source'],
}

phase('Plan')
const plan = await agent(
  `You are the planning node of the daily scheduled-ideation workflow for a personal agents repo (Pi coding-agent CLI, Herdr terminal multiplexer, GitHub Issues for tracking). This workflow's fixed mission, every run: propose a batch of candidates across three categories a human reviews later — SKILLS worth authoring, AGENTS worth authoring, and existing PUBLIC TOOLS worth adopting or trying. Never file anything automatically; a later stage writes a markdown digest for manual review only.${FOCUS_NOTE ? `\n\nThis specific run has an operator-supplied focus note to steer the tool-radar angle: "${FOCUS_NOTE}"` : ''}

Design exactly 2 mining dispatches (codebase, read-only) plus between 1 and ${MAX_TOOL_RADAR} tool-radar dispatches (web), each self-contained: the agent receiving it sees NOTHING but the five fields below, not this conversation.

Both mining dispatches MUST include this exact reading-boundedness rule verbatim in their objective, because a Pi session transcript can run to many megabytes and an earlier live run aborted trying to read several in full: "Session transcripts live under ~/.pi/agent/sessions/ and can be multi-MB each. NEVER read a full session file. Use grep to find markers first (tool_use, error, cost, a skill/agent name), or read only the last ~200 lines (tail) of a session, or bounded offset/limit reads — never an unbounded whole-file read. The same per-file cap applies to any logs/usage.jsonl or run-history.jsonl found."

Mining dispatch 1 (category "skill", label "skill-evidence-sweep"): objective MUST instruct the agent to run the exact "bounded session evidence sweep" procedure from skills/ai-author/SKILL.md in this repo (read that file first, then follow its numbered steps 1-6 verbatim) and report the surviving candidates it proposes, each with its evidence and routing verdict. Step 5 of that procedure already routes each surviving candidate to whichever destination the evidence actually supports — a checker, a Pi extension, a skill, an agent, or a workflow — so the candidates returned are NOT limited to category "skill": instruct the agent explicitly to report each candidate's category as whatever that step concludes (skill|agent|workflow|checker|extension), never force-fit everything into "skill". source_guidance: "skills/ai-author/SKILL.md for the procedure; the ten newest parent Pi session transcripts and named artifacts' logs/usage.jsonl or run-history.jsonl for evidence, per that procedure's own window rule and the reading-boundedness rule above."

Mining dispatch 2 (category "agent", label "agent-candidate-scan"): objective MUST instruct the agent to read skills/ai-author/SKILL.md's "should it exist?" section (the same repo, same file) for the criteria that justify an AGENT specifically — a distinct model the parent shouldn't hold, a tool grant the parent must not hold, or isolation from noisy work — then apply the SAME bounded evidence sweep window and evidence discipline (measured repetition and cost only, never estimated, reject unmeasured shapes) but filtered to shapes that clear the agent bar specifically. When a shape doesn't clear the agent bar but the same step 5 routing concludes a different destination (checker, extension, skill, workflow) still fits, report that too rather than discarding it — instruct the agent to report each candidate's actual category (skill|agent|workflow|checker|extension), not force everything into "agent". source_guidance: same evidence sources and reading-boundedness rule as dispatch 1.

Tool-radar dispatches (category "tool", 1-${MAX_TOOL_RADAR} of them): each covers a DISTINCT external-discovery angle relevant to this repo's actual stack (Pi/Claude Code and coding-agent tooling, TypeScript/Rust developer tooling, terminal multiplexer/Herdr-adjacent tooling, AI agent orchestration). Rotate which specific sources and angle you pick each run so consecutive daily runs don't repeat the same search — vary among GitHub Trending, Hacker News, package-registry release feeds, vendor/company engineering blogs, and ArXiv for anything genuinely research-adjacent. Each objective must ask the researcher to name concrete, named tools/projects with a URL and a one-paragraph rationale for why THIS repo's owner specifically would want it, not a generic "top N tools" listicle.

Fields per dispatch: label (short kebab-case), category (skill|agent|tool), objective (specific enough that "answered" is checkable), boundaries (what's out of scope), source_guidance (files/dirs for mining, or URLs/domains/search venues for tool-radar), recency ("current repo state" for mining, a freshness bound for tool-radar).`,
  {
    label: 'plan',
    schema: {
      type: 'object',
      properties: { dispatches: { type: 'array', items: DISPATCH_SCHEMA } },
      required: ['dispatches'],
    },
  })

if (!plan || !plan.dispatches || !plan.dispatches.length) return { error: 'plan node returned nothing; no candidate generators dispatched' }

const mining = plan.dispatches.filter(d => d.category === 'skill' || d.category === 'agent').slice(0, 2)
const toolRadar = plan.dispatches.filter(d => d.category === 'tool').slice(0, MAX_TOOL_RADAR)
const allDispatches = [...mining, ...toolRadar]
log(`planned ${mining.length} mining dispatch(es) + ${toolRadar.length} tool-radar dispatch(es): ${allDispatches.map(d => d.label).join(', ')}`)

// Mining dispatches (skill/agent) report whatever destination ai-author's own type-tree
// concludes, not the coarse bucket they were routed under; tool-radar stays fixed at "tool"
// since it never runs that routing step.
const categoryInstruction = d =>
  d.category === 'tool'
    ? `category ("tool")`
    : `category (one of skill|agent|workflow|checker|extension — whichever your routing verdict actually concludes; do not force-fit into "${d.category}")`

const dispatchPrompt = d =>
  `objective: ${d.objective}\nboundaries: ${d.boundaries}\nsource_guidance: ${d.source_guidance}\nrecency: ${d.recency}\n\nReturn a JSON array of candidates you found (may be empty — zero candidates is a valid, honest result). Each candidate needs: name, ${categoryInstruction(d)}, rationale (why this specific repo/owner would want it), evidence (the measured repetition/cost, or the named source finding — never an estimate), source (a file/log path for mining, a URL for tool-radar).`

phase('Generate')
// Mining ran on agentType 'Explore' (Haiku-tier) in the founding version; two real 2026-08-28
// live runs both aborted it mid-task on skill-evidence-sweep/agent-candidate-scan ("This
// operation was aborted") -- reproduced live twice, including on a run doing nothing but
// bounded directory-listing with no large reads at all, so it isn't a content-size problem
// prompt tweaks can fix. Dropped agentType entirely: mining now runs on the same default
// (session) model every other node here already uses successfully in every one of those
// same runs -- Plan, Filter, and Digest never once failed.
const [miningResults, toolResults] = await Promise.all([
  parallel(mining.map(d => () =>
    agent(dispatchPrompt(d), { label: d.label, phase: 'Generate', schema: { type: 'object', properties: { candidates: { type: 'array', items: CANDIDATE_SCHEMA } }, required: ['candidates'] } })
      .then(r => (r ? { label: d.label, candidates: r.candidates } : null)))),
  parallel(toolRadar.map(d => () =>
    agent(dispatchPrompt(d), { label: d.label, phase: 'Generate', agentType: 'web-research-summarizer', schema: { type: 'object', properties: { candidates: { type: 'array', items: CANDIDATE_SCHEMA } }, required: ['candidates'] } })
      .then(r => (r ? { label: d.label, candidates: r.candidates } : null)))),
])

const generateResults = [...miningResults, ...toolResults].filter(Boolean)
const generateMissing = allDispatches.map(d => d.label).filter(l => !generateResults.some(r => r.label === l))
const rawCandidates = generateResults.flatMap(r => r.candidates || [])
log(`generate round: ${generateResults.length}/${allDispatches.length} dispatches returned, ${rawCandidates.length} raw candidate(s)${generateMissing.length ? `, missing: ${generateMissing.join(', ')}` : ''}`)

if (!rawCandidates.length) {
  return { candidates: [], digest: '## Scheduled ideation — no candidates today\n\nEvery dispatch returned zero candidates, or every dispatch failed to return. See missingLabels.', expected: allDispatches.length, returned: generateResults.length, missingLabels: generateMissing }
}

phase('Filter')
const filtered = await agent(
  `You are a fresh-context filter for a daily candidate-ideation run. You were not involved in generating these candidates and have never seen the sessions or searches that produced them — judge only what's written here.

Below are ${rawCandidates.length} raw candidates across six possible categories (skill/agent/workflow/checker/extension/tool), from a personal agents repo's daily ideation sweep. Score each on: (1) evidence strength — is the rationale backed by a measured, cited fact (a real repetition count, a real cost, a real URL fetched this run) rather than a vibe or an estimate; (2) relevance — would this repo's owner plausibly act on it; (3) actionability — is there a concrete next step, not just a vague observation. This directly guards against a documented 2026 failure mode: AI-generated noise overwhelming a human reviewer. When in doubt, drop it — a short, high-signal list beats a long noisy one.

Return only the survivors, capped at ${MAX_DIGEST_CANDIDATES}, ranked best first, plus how many you dropped and the single main reason (e.g. "6 dropped: unmeasured/estimated evidence").

${rawCandidates.map((c, i) => `=== candidate ${i + 1} (${c.category}) ===\nname: ${c.name}\nrationale: ${c.rationale}\nevidence: ${c.evidence}\nsource: ${c.source}`).join('\n\n')}`,
  { phase: 'Filter', schema: { type: 'object', properties: { survivors: { type: 'array', items: CANDIDATE_SCHEMA }, droppedCount: { type: 'number' }, dropReason: { type: 'string' } }, required: ['survivors', 'droppedCount', 'dropReason'] } })

const survivors = filtered ? filtered.survivors.slice(0, MAX_DIGEST_CANDIDATES) : rawCandidates.slice(0, MAX_DIGEST_CANDIDATES)
log(filtered ? `filter: ${survivors.length} survived, ${filtered.droppedCount} dropped (${filtered.dropReason})` : 'filter node returned nothing; digest carries unfiltered candidates, flagged')

phase('Digest')
const digestAgent = await agent(
  `Write the final markdown digest for today's scheduled-ideation run. Group these survivor candidates under headings in this exact order, one per category present: "## Skills worth authoring", "## Agents worth authoring", "## Workflows worth authoring", "## Checkers/linters worth building" (category "checker"), "## Pi extensions worth building" (category "extension"), "## Tools worth trying" (category "tool") (omit a heading entirely if it has zero candidates — never write an empty section). Under each candidate: a bold name, then its rationale, evidence, and source on separate lines. Start the file with a one-line dated header "# Scheduled ideation — <today's date if inferable from context, otherwise omit the date>" and end with a one-line footer stating how many raw candidates were generated, how many survived filtering, and any dispatch labels that returned nothing.

Survivors:
${survivors.map(c => `- [${c.category}] ${c.name} — ${c.rationale} | evidence: ${c.evidence} | source: ${c.source}`).join('\n')}

Raw generated: ${rawCandidates.length}. Survived filter: ${survivors.length}.${generateMissing.length ? ` Dispatches that returned nothing: ${generateMissing.join(', ')}.` : ''}`,
  { phase: 'Digest' })

return {
  candidates: survivors,
  digest: digestAgent || `## Scheduled ideation\n\n(digest-writer node returned nothing; raw survivor list follows)\n\n${survivors.map(c => `- [${c.category}] ${c.name} — ${c.rationale}`).join('\n')}`,
  expected: allDispatches.length,
  returned: generateResults.length,
  missingLabels: generateMissing,
  rawCandidateCount: rawCandidates.length,
  survivorCount: survivors.length,
}
