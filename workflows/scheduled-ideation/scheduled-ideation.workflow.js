export const meta = {
  name: 'scheduled-ideation',
  description: 'Daily fan-out that hunts for the highest-impact levers to pull on this workspace and AI setup — mines 24h session activity plus external research, checks every finding against what the repo already has, ranks survivors by leverage, then synthesizes one markdown digest',
  whenToUse: "Triggered by the scheduled-ideation launchd job's seeded kickoff prompt inside a fresh daily Herdr worktree. Returns a digest; the caller writes it to .context/scheduled-ideation-digest.md — this workflow has no filesystem access of its own.",
  phases: [
    { title: 'Plan', detail: "one agent turns the fixed daily mission (highest-impact levers, not an exhaustive catalog) into self-contained mining + radar dispatches scoped to the last 24h, rotating tool-radar sources so runs don't repeat the same search every day" },
    { title: 'Generate', detail: 'one agent per angle: session-evidence mining (skills/agents), correction-mining (checkers), external-tool radar', model: 'sonnet' },
    { title: 'Filter', detail: 'fresh-context critic with repo read access checks every raw candidate against the current implementation (already built? already open as an issue?), drops what is already addressed or unmeasured, then ranks the rest by leverage/impact, highest first' },
    { title: 'Digest', detail: 'one agent writes the final impact-ranked markdown digest from survivors' },
  ],
}

// Mission is fixed day to day — no required args. An optional args.focus note lets a
// one-off manual run steer the tool-radar angle (e.g. "focus on Rust tooling this run");
// the daily 3pm trigger calls this with no args.
const FOCUS_NOTE = (args && typeof args === 'object' && args.focus) ? args.focus : null
const MAX_TOOL_RADAR = Math.min((args && args.max_tool_radar) || 3, 3)
const MAX_DIGEST_CANDIDATES = 10
const MAX_MINING = 3

const DISPATCH_SCHEMA = {
  type: 'object',
  properties: {
    label: { type: 'string' },
    category: { type: 'string', enum: ['skill', 'agent', 'checker', 'tool'] },
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
// A dedicated impactNote field was tried and dropped: a live 2026-08-28 run showed the
// model omitting it from every single candidate across all 8 agent calls, even marked
// required — this harness's structured-output validation doesn't strictly enforce
// required fields nested inside an array item schema, so a field the model skips just
// silently vanishes instead of forcing a retry. Folding the leverage statement into the
// existing required `rationale` field (see dispatchPrompt below) is more reliable than a
// field the model can quietly skip.
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
  `You are the planning node of the daily scheduled-ideation workflow for a personal agents repo (Pi coding-agent CLI, Herdr terminal multiplexer, GitHub Issues for tracking). This workflow's fixed mission, every run: find the HIGHEST-IMPACT LEVERS the owner can pull right now to improve this actual workspace and AI setup — not an exhaustive catalog, a small ranked set of moves that would matter most, each checked against what the repo already has before it's proposed. It draws from four categories — SKILLS worth authoring, AGENTS worth authoring, CHECKERS worth building (deterministic rules for recurring agent mistakes, per GitHub issue #79), and existing PUBLIC TOOLS worth adopting or trying — but every candidate must earn its place by leverage: what breaks, costs, or stays slow without it, not just that it would be nice to have. Never file anything automatically; a later stage writes a markdown digest for manual review only.${FOCUS_NOTE ? `\n\nThis specific run has an operator-supplied focus note to steer the tool-radar angle: "${FOCUS_NOTE}"` : ''}

Design exactly 3 mining dispatches (codebase, read-only) plus between 1 and ${MAX_TOOL_RADAR} tool-radar dispatches (web), each self-contained: the agent receiving it sees NOTHING but the five fields below, not this conversation.

All three mining dispatches MUST include this exact reading-boundedness rule verbatim in their objective, because a Pi session transcript can run to many megabytes and an earlier live run aborted trying to read several in full: "Session transcripts live under ~/.pi/agent/sessions/ and can be multi-MB each. NEVER read a full session file. Use grep to find markers first (tool_use, error, cost, a skill/agent name), or read only the last ~200 lines (tail) of a session, or bounded offset/limit reads — never an unbounded whole-file read. The same per-file cap applies to any logs/usage.jsonl or run-history.jsonl found. Repetition evidence is NOT limited to artifact usage logs -- grep session transcripts for markers of repeated friction: a warning or error that recurs, a workaround or manual recovery step applied more than once, an operation that silently failed and needed re-doing. A marker that greps 2+ times across the window is measured evidence (you counted real occurrences), not an estimate -- cite each occurrence's rough marker text/timestamp as the evidence. Zero candidates stays a valid, honest result when nothing actually repeats."

All three mining dispatches MUST also include this exact window rule verbatim, overriding any fixed-count window a referenced procedure states: "Scope the session window to the last 24 hours of activity, not a fixed count of newest sessions: any parent Pi session transcript (exclude child/subagent transcripts) whose last message timestamp falls within the last 24 hours is in scope, however many that is. Zero sessions active in the last 24 hours is a valid, honest result — report it, don't substitute an older window."

Mining dispatch 1 (category "skill", label "skill-evidence-sweep"): objective MUST instruct the agent to run the "bounded session evidence sweep" procedure from skills/ai-author/SKILL.md in this repo (read that file first, then follow its numbered steps 1-6, substituting the 24-hour window rule above for that procedure's own step-1 window) and report the surviving candidates it proposes, each with its evidence and routing verdict. Step 5 of that procedure already routes each surviving candidate to whichever destination the evidence actually supports — a checker, a Pi extension, a skill, an agent, or a workflow — so the candidates returned are NOT limited to category "skill": instruct the agent explicitly to report each candidate's category as whatever that step concludes (skill|agent|workflow|checker|extension), never force-fit everything into "skill". source_guidance: "skills/ai-author/SKILL.md for the procedure; parent Pi session transcripts active in the last 24 hours, and named artifacts' logs/usage.jsonl or run-history.jsonl for evidence, per the window rule and the reading-boundedness rule above."

Mining dispatch 2 (category "agent", label "agent-candidate-scan"): objective MUST instruct the agent to read skills/ai-author/SKILL.md's "should it exist?" section (the same repo, same file) for the criteria that justify an AGENT specifically — a distinct model the parent shouldn't hold, a tool grant the parent must not hold, or isolation from noisy work — then apply the SAME 24-hour window and evidence discipline (measured repetition and cost only, never estimated, reject unmeasured shapes) but filtered to shapes that clear the agent bar specifically. When a shape doesn't clear the agent bar but the same step 5 routing concludes a different destination (checker, extension, skill, workflow) still fits, report that too rather than discarding it — instruct the agent to report each candidate's actual category (skill|agent|workflow|checker|extension), not force everything into "agent". source_guidance: same evidence sources, window rule, and reading-boundedness rule as dispatch 1.

Mining dispatch 3 (category "checker", label "correction-mining"): objective MUST instruct the agent to grep parent session transcripts in the 24-hour window for markers where the human user corrected the agent mid-task — pushback language such as "no,", "that's wrong", "don't", "stop", "undo", "revert", "not what I", "fix this", "why did you" inside role":"user" message blocks — then read a bounded window (offset/limit, never the whole file) around each hit to capture what the agent had just done and what exactly got corrected. Group hits into repeated SHAPES: the same kind of mistake recurring across 2+ independent sessions or tasks (e.g. "claimed a command succeeded without checking its exit code", "reported something done without verifying"). A shape needs 2+ independent occurrences to count; a single isolated correction is not a pattern — note it but do not report it as a candidate. For each surviving shape, report it as a candidate with category "checker" (this feeds GitHub issue #79, which wants deterministic Rust checkers under tools/ for exactly this class of recurring, cited, mechanizable mistake): name the shape, cite each occurrence (session path + a short quote of the correction), and state plainly whether it looks mechanizable (a program can catch it) or requires judgment (drop it — issue #79 explicitly wants only bounded, non-judgment rules). Zero repeated shapes in the window is a valid, honest result. source_guidance: "parent Pi session transcripts active in the last 24 hours only, per the window rule above; the reading-boundedness rule above governs every read."

Tool-radar dispatches (category "tool", 1-${MAX_TOOL_RADAR} of them): each covers a DISTINCT external-discovery angle relevant to this repo's actual stack (Pi/Claude Code and coding-agent tooling, TypeScript/Rust developer tooling, terminal multiplexer/Herdr-adjacent tooling, AI agent orchestration). Rotate which specific sources and angle you pick each run so consecutive daily runs don't repeat the same search — vary among GitHub Trending, Hacker News, package-registry release feeds, vendor/company engineering blogs, and ArXiv for anything genuinely research-adjacent. Each objective must ask the researcher to name concrete, named tools/projects with a URL and a one-paragraph rationale for why THIS repo's owner specifically would want it, not a generic "top N tools" listicle.

Fields per dispatch: label (short kebab-case), category (skill|agent|checker|tool), objective (specific enough that "answered" is checkable), boundaries (what's out of scope), source_guidance (files/dirs for mining, or URLs/domains/search venues for tool-radar), recency ("current repo state" for mining, a freshness bound for tool-radar).`,
  {
    label: 'plan',
    schema: {
      type: 'object',
      properties: { dispatches: { type: 'array', items: DISPATCH_SCHEMA } },
      required: ['dispatches'],
    },
  })

if (!plan || !plan.dispatches || !plan.dispatches.length) return { error: 'plan node returned nothing; no candidate generators dispatched' }

const mining = plan.dispatches.filter(d => d.category === 'skill' || d.category === 'agent' || d.category === 'checker').slice(0, MAX_MINING)
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

const dispatchPrompt = (d, extra) =>
  `objective: ${d.objective}\nboundaries: ${d.boundaries}\nsource_guidance: ${d.source_guidance}\nrecency: ${d.recency}${extra ? `\n\n${extra}` : ''}\n\nReturn a JSON array of candidates you found (may be empty — zero candidates is a valid, honest result). Each candidate needs: name, ${categoryInstruction(d)}, rationale (its FIRST SENTENCE must state the actual leverage this buys — time, tokens, dollars, or failures saved, measured or a stated honest estimate flagged as such; the rest of the sentence explains why this specific repo/owner would want it), evidence (the measured repetition/cost, or the named source finding — never an estimate), source (a file/log path for mining, a URL for tool-radar).`

phase('Generate')
// Mining ran on agentType 'Explore' (Haiku-tier) in the founding version; two real 2026-08-28
// live runs both aborted it mid-task on skill-evidence-sweep/agent-candidate-scan ("This
// operation was aborted") -- reproduced live twice, including on a run doing nothing but
// bounded directory-listing with no large reads at all, so it isn't a content-size problem
// prompt tweaks can fix. Dropped agentType entirely: mining now runs on the same default
// (session) model every other node here already uses successfully in every one of those
// same runs -- Plan, Filter, and Digest never once failed.
//
// Mining runs BEFORE tool-radar now, as a genuine barrier rather than a fake one: tool-radar
// needs mining's actual real-evidence content to ground its rationale in this repo's own
// observed usage rather than generic trending-tool descriptions (2026-08-28 live runs kept
// surfacing "seems useful for a terminal multiplexer" reasoning with no connection to any
// specific measured friction). This costs wall-clock versus one combined wave, which is the
// correct trade per workflow-author's own barrier rule: stage N here genuinely needs stage
// N-1's cross-item content, not just its own independent inputs.
const miningResults = await parallel(mining.map(d => () =>
  agent(dispatchPrompt(d), { label: d.label, phase: 'Generate', schema: { type: 'object', properties: { candidates: { type: 'array', items: CANDIDATE_SCHEMA } }, required: ['candidates'] } })
    .then(r => (r ? { label: d.label, candidates: r.candidates } : null))))

const minedCandidates = miningResults.filter(Boolean).flatMap(r => r.candidates || [])
const usageGrounding = minedCandidates.length
  ? `Real recurring friction measured in this repo's own session evidence this run:\n${minedCandidates.map(c => `- [${c.category}] ${c.name}: ${c.evidence}`).join('\n')}\n\nYour rationale for any tool candidate MUST explicitly name which of these it addresses, or state plainly "addresses no measured friction this run" and instead ground the fit in this repo's specific, real stack details (Pi coding-agent CLI, Herdr terminal multiplexer, TypeScript/Rust tooling, GitHub Issues tracking) rather than a generic "would be useful" claim. A rationale that ignores this entirely will be dropped in filtering.`
  : `Mining found zero measured recurring friction this run (a valid, honest result). Your rationale for any tool candidate must still ground fit in this repo's specific, real stack details (Pi coding-agent CLI, Herdr terminal multiplexer, TypeScript/Rust tooling, GitHub Issues tracking), never a generic "would be useful" claim with no connection to this repo's actual work.`

const toolResults = await parallel(toolRadar.map(d => () =>
  agent(dispatchPrompt(d, usageGrounding), { label: d.label, phase: 'Generate', agentType: 'web-research-summarizer', schema: { type: 'object', properties: { candidates: { type: 'array', items: CANDIDATE_SCHEMA } }, required: ['candidates'] } })
    .then(r => (r ? { label: d.label, candidates: r.candidates } : null))))

const generateResults = [...miningResults, ...toolResults].filter(Boolean)
const generateMissing = allDispatches.map(d => d.label).filter(l => !generateResults.some(r => r.label === l))
const rawCandidates = generateResults.flatMap(r => r.candidates || [])
log(`generate round: ${generateResults.length}/${allDispatches.length} dispatches returned, ${rawCandidates.length} raw candidate(s)${generateMissing.length ? `, missing: ${generateMissing.join(', ')}` : ''}`)

if (!rawCandidates.length) {
  return { candidates: [], digest: '## Scheduled ideation — no candidates today\n\nEvery dispatch returned zero candidates, or every dispatch failed to return. See missingLabels.', expected: allDispatches.length, returned: generateResults.length, missingLabels: generateMissing }
}

phase('Filter')
// This stage does two jobs the workflow-as-a-whole owns, not any one generating dispatch:
// (1) compare every raw candidate — mined evidence AND external tool-radar research alike
// — against what THIS repo's workspace already has, using real read/grep/bash access, so a
// candidate that's already built or already an open issue never reaches the digest; (2) rank
// what survives by actual leverage, not just evidence quality, so the digest leads with the
// highest-impact move rather than an arbitrary or alphabetical order.
const filtered = await agent(
  `You are a fresh-context filter and ranker for a daily candidate-ideation run. You were not involved in generating these candidates and have never seen the sessions or searches that produced them — judge only what's written here plus what you verify yourself in the repo.

Below are ${rawCandidates.length} raw candidates across six possible categories (skill/agent/workflow/checker/extension/tool), from a personal agents repo's daily ideation sweep whose fixed mission is finding the HIGHEST-IMPACT LEVERS to pull on this workspace and AI setup right now — not an exhaustive catalog.

STEP 1 — compare against the current implementation (use your read/grep/bash tools on this checked-out repo; use \`gh issue list --state all --search "<name>"\` to check GitHub Issues too): for EVERY candidate, actually look — does skills/, workflows/, tools/, pi/extensions/, or an existing open/closed GitHub issue already cover this? Drop any candidate that's already built or already fully tracked by an open issue (cite what you found in dropReason). Note a PARTIAL match (something related exists but this candidate is still a real gap) in the survivor's rationale rather than dropping it.

STEP 2 — evidence quality: for what's left, score on (a) evidence strength — is the rationale backed by a measured, cited fact (a real repetition count, a real cost, a real URL fetched this run) rather than a vibe or an estimate; (b) relevance — would this repo's owner plausibly act on it; (c) actionability — is there a concrete next step. For any candidate with category "tool": drop it unless its rationale explicitly names a specific measured friction item it addresses, or explicitly grounds fit in this repo's real stack details (Pi coding-agent CLI, Herdr terminal multiplexer, TypeScript/Rust tooling, GitHub Issues tracking). This guards against a documented 2026 failure mode: AI-generated noise overwhelming a human reviewer. When in doubt, drop it — a short, high-signal list beats a long noisy one.

STEP 3 — rank by leverage, not evidence alone: order every survivor by actual impact — how much recurring cost (time, tokens, dollars, failures, friction) this removes, weighted by how often the underlying friction actually recurs. A well-evidenced but low-stakes candidate ranks below a higher-stakes one even with slightly thinner evidence, as long as the higher-stakes one still clears STEP 2's bar. Put the single highest-leverage candidate first.

Return only the survivors, capped at ${MAX_DIGEST_CANDIDATES}, ranked highest-leverage first, plus how many you dropped and the single main reason (e.g. "6 dropped: 2 already implemented (found in tools/), 4 unmeasured/estimated evidence").

${rawCandidates.map((c, i) => `=== candidate ${i + 1} (${c.category}) ===\nname: ${c.name}\nrationale: ${c.rationale}\nevidence: ${c.evidence}\nsource: ${c.source}`).join('\n\n')}`,
  { phase: 'Filter', schema: { type: 'object', properties: { survivors: { type: 'array', items: CANDIDATE_SCHEMA }, droppedCount: { type: 'number' }, dropReason: { type: 'string' } }, required: ['survivors', 'droppedCount', 'dropReason'] } })

const survivors = filtered ? filtered.survivors.slice(0, MAX_DIGEST_CANDIDATES) : rawCandidates.slice(0, MAX_DIGEST_CANDIDATES)
log(filtered ? `filter: ${survivors.length} survived, ${filtered.droppedCount} dropped (${filtered.dropReason})` : 'filter node returned nothing; digest carries unfiltered candidates, flagged')

phase('Digest')
const digestAgent = await agent(
  `Write the final markdown digest for today's scheduled-ideation run. Group ALL survivors under headings in this exact order, one per category present: "## Skills worth authoring", "## Agents worth authoring", "## Workflows worth authoring", "## Checkers/linters worth building" (category "checker"), "## Pi extensions worth building" (category "extension"), "## Tools worth trying" (category "tool") (omit a heading entirely if it has zero candidates — never write an empty section). Candidates stay in the rank order given below within each section. Under each candidate: a bold name, then its rationale (its first sentence already states the leverage/impact), evidence, and source on separate lines. Start the file with a one-line dated header "# Scheduled ideation — <today's date if inferable from context, otherwise omit the date>" and end with a one-line footer stating how many raw candidates were generated, how many survived filtering, and any dispatch labels that returned nothing. Do NOT write your own top-level summary section — the caller prepends one from the #1-ranked survivor in code.

Survivors, in rank order (best/highest-leverage first):
${survivors.map((c, i) => `${i + 1}. [${c.category}] ${c.name} — ${c.rationale} | evidence: ${c.evidence} | source: ${c.source}`).join('\n')}

Raw generated: ${rawCandidates.length}. Survived filter: ${survivors.length}.${generateMissing.length ? ` Dispatches that returned nothing: ${generateMissing.join(', ')}.` : ''}`,
  { phase: 'Digest' })

// Built in code, not left to model compliance: a live 2026-08-28 run showed the Digest
// agent silently dropping the "lead with a Top lever section" instruction even when it was
// spelled out in the prompt. `survivors` is already Filter's own rank-ordered array, so the
// #1 lever is known data, not a judgment call — construct this section directly instead of
// hoping the model notices the instruction again next time.
const topLeverBlock = survivors.length
  ? `## Top lever today\n\n**[${survivors[0].category}] ${survivors[0].name}**\n${survivors[0].rationale}\n\n`
  : ''
const bodyDigest = digestAgent || `## Scheduled ideation\n\n(digest-writer node returned nothing; raw survivor list follows)\n\n${survivors.map(c => `- [${c.category}] ${c.name} — ${c.rationale}`).join('\n')}`
// Insert the top-lever block right after the digest's own dated header line (its first
// line), or at the very top if the digest text has no header line to key off of.
const headerMatch = bodyDigest.match(/^# .*\n/)
const finalDigest = headerMatch
  ? bodyDigest.slice(0, headerMatch[0].length) + '\n' + topLeverBlock + bodyDigest.slice(headerMatch[0].length)
  : topLeverBlock + bodyDigest

return {
  candidates: survivors,
  digest: finalDigest,
  expected: allDispatches.length,
  returned: generateResults.length,
  missingLabels: generateMissing,
  rawCandidateCount: rawCandidates.length,
  survivorCount: survivors.length,
}
