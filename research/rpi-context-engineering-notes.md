# RPI(Research, Plan, Implement) and intentional compaction — talk notes, elaborated

Source: "No Vibes Allowed: Solving Hard Problems in Complex Codebases" — Dex Horthy, HumanLayer (YouTube).
Purpose: elaborate the raw notes and map each idea onto the current workflow in this repo (the `engineer` skill and the dispatch fleet).

## The core claim

The talk names one mechanism: an agent's output quality tracks how small and how curated
its context window is. RPI is a label for a loop that keeps the window small by design:

1. **Research** — compress the codebase truth into a document.
2. **Plan** — compress intent into exact steps with file names, line numbers, snippets, and a test story.
3. **Implement** — a fresh, small context executes the plan.

Each phase writes a file. The file is the compaction. The next phase reads the file, never
the previous transcript. Horthy's own caveat: RPI is a temporary name. The durable idea is
context engineering — every phase boundary is an intentional compaction point.

The "Memento" reference makes the model concrete: the protagonist has no long-term memory,
so he survives on externalized, curated notes (tattoos, photos, captions). An agent is the
same. What is not written into the artifact does not exist for the next phase. And a wrong
caption propagates forever — which is why the human signs the research and the plan.

## The 40% "smart zone"

Keep the working context under ~40% of the window. Past that, retrieval quality and
instruction-following degrade before the window is "full." Practical readings:

- Budget per phase, not per session. Each phase starts near zero and must finish under the line.
- Compaction is intentional and frequent — you compact by writing the artifact and starting
  fresh, not by letting the harness auto-compact at 90% (auto-compaction is unreviewed lossy
  summarization at the worst time).
- "The dumbest model in the world couldn't mess it up" is the acceptance test for a plan:
  if implementation requires judgment, the plan is not done. Corollary from the raw notes:
  a plan that passes this test is exactly what makes small local models viable for the
  implement phase. The plan quality, not the implement-model quality, is the ceiling.

## Phase details worth keeping

**Research**
- Goal: how the system works, which files matter. Output is a snapshot of truth grounded in
  code, with paths and line references.
- Stay objective — no bug hunting, no fixing. A research phase that starts diagnosing
  contaminates the artifact with speculation.
- Preferred shape: on-demand compressed context. Give steering ("this feature touches SCM
  providers, Jira, Linear — start over here"), fan sub-agents down vertical slices, merge
  into one research.md. A human reads it and signs off.
- Anti-pattern: static per-directory onboarding docs. They rot. Compress on demand from the
  code itself instead; the code is the only source that cannot be stale.

**Plan**
- Exact steps, file names, line numbers, code snippets.
- Explicit test story: what gets tested automatically, what gets tested by hand.
- The plan doubles as the code-review artifact: "here's the plan, here's what I did,
  here's how it was tested."

**Implement**
- If the plan is right, this phase is boring. Surprise during implement is a planning
  failure — walk back, do not improvise forward.

## Guardrails from the talk

- **Do not outsource thinking.** The agent amplifies the thinking already done; it cannot
  substitute for it. The human sign-offs on research and plan are where the thinking lives.
- **Not spec-driven development.** That term suffered semantic diffusion (Fowler, 2006 —
  a term spreads until nobody agrees what it means). RPI artifacts are compressed context,
  not upfront specs; they are cheap, regenerated per task, and disposable.
- **Dopamine flywheels.** Tools that emit stacks of markdown with no human gate produce
  slop that feels like progress. The gate — a human actually reading research.md and the
  plan — is the difference between compression and noise.

## Mapping onto the current workflow (`skills/engineer`)

Already covered — the 23-phase map is RPI with more joints:

| Talk idea | Where it already lives |
|---|---|
| Research phase, vertical-slice sub-agents | Phase 02 + web-research-summarizer / research-sweep dispatches |
| Plan with files/lines/tests | Phases 04–12 (testability, structures, interfaces, DAG, test-cases, todos) |
| Fresh small-context implement | Phase 13 fresh builders; "every dispatch carries compiled context only, never the transcript" |
| Plan as review artifact | Phase 23 fresh code-reviewer reads the .map run record |
| Surprise → replan, not improvise | The walk-back rule; "deviations disclosed, never absorbed" |
| On-demand compression over static onboarding | The dispatch rule (compiled context per phase) and the RAG store |
| Human gates against slop | GATE A/B/C/D/E/F |

Two terminology traps found on inspection of `phases/02-research.md`:

- **"Research" names two different activities.** The talk's research is CODEBASE research —
  vertical slices through the code, compressed into a truth snapshot. Phase 02's step 1 is
  WEB research (web-research-summarizer, explicitly banned from repo questions); its step 2
  is codebase research, but only a light Explore survey into `## project-state`. The deep
  code understanding the talk compresses into research.md instead accretes in the
  orchestrator's own transcript across phases 04–07 — exactly the uncompacted state the
  talk warns against. Any future doc or skill edit must say "web research" or "codebase
  research", never bare "research".
- **Restart-from-artifact is stated for dispatches only.** "Every dispatch carries compiled
  context only" binds sub-agents. Nothing states the complementary rule: after a phase
  writes its artifact, the NEXT consumer starts from the artifact alone. The orchestrator
  never restarts, and phase 13 does not say builders read only the plan files. The
  compaction discipline exists for the fleet and is absent for the spine.

Gaps — the ideas the current setup does not state:

1. **No explicit context budget.** Nothing names the 40% line. The orchestrator can bloat
   inside a phase with nothing tripping. Candidate: a one-line rule in the engineer skill —
   an orchestrator crossing ~40% mid-phase writes its phase artifact and re-enters fresh,
   the walk-back machinery already covering the mechanics. (Related roadmap note already
   filed: canary-per-message compaction signal.)
2. **Research has no human gate.** Phase 02 output flows into GATE B at phase 11 — the
   human first sees compressed research only after nine phases built on it. The talk's
   strongest claim is that a wrong research.md poisons everything downstream (Memento's
   wrong caption). Candidate: extend GATE A, or add a cheap sign-off at the end of 02.
3. **Codebase research is shallow and unlabeled.** Candidate: rename phase 02's outputs to
   web-research vs codebase-research, and let step 2 fan Explore agents down the vertical
   slices the ticket names, merging into a snapshot with file:line anchors.
4. **No spine-level restart rule.** Candidate: one line in the engineer skill — a phase's
   consumer (next phase, or any dispatched worker) opens with the named artifacts in
   context and nothing else; an orchestrator crossing the budget writes the current phase
   artifact and re-enters fresh from the .map run record.
5. **Manual-test story is implicit.** Phase 20 produces the sign-off checklist, but the
   plan (11) does not force the auto/manual split the talk wants visible at review time.
   Candidate: one line in the phase-10/11 contract naming the manual cases.

Status: applied on owner's instruction (this session), not left as candidates.

- `phases/02-research.md` — rewritten. Names web research vs codebase research explicitly,
  fans one Explore agent per vertical slice, merges into a file:line-anchored `## codebase`
  snapshot, and ends in HUMAN GATE R.
- `SKILL.md` — phase-index row 02 updated; GATE R added to the gates section (seven
  lettered gates); routing section gains the spine compaction rule: artifacts are the only
  carrier between phases, and the orchestrator re-enters fresh via resume past ~40% of the
  window.
- `phases/10-test-cases.md` — every case now carries `mode: auto` or `mode: manual`; the
  manual set is visible at GATE B.
- `phases/20-signoff.md` — the checklist pool now draws the `mode: manual` cases, making
  the human list their executor.
- `phases/13-implement.md` — unchanged; it already banned the planning transcript for
  builders.
