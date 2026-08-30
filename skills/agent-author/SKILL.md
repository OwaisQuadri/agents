---
name: agent-author
description: Use when authoring, rewriting, or overhauling an AGENT — a subagent definition (.md with frontmatter) for a distinct role with its own tool set and judgment. Owns the definition anatomy, the embedded contract, the eval harness, and the failure modes agents die from. Skip when the capability is a recipe a single agent follows (that is a skill, see skill-author) or a topology over several agents (that is a workflow, see workflow-author).
metadata:
  minimum-tier: T4
  short-description: Author agent definitions with contracts + evals
---

# agent-author

An agent is a node with a contract: ONE job, a defined input it never assumes, a defined
output the next node consumes without guessing. Everything below makes that contract
explicit, checkable, and cheap to grade. The old fleet ("~/agent loop/agents/" —
router.md, implementer.md) wrote role prose and stopped: no embedded contract, no eval,
no failure-mode watch-list. Dispatchable, never gradeable. This skill closes that gap;
do not port that content.

## agent, skill, or workflow?

The shape that says "agent":

- a distinct ROLE: its own tool set, its own judgment, dispatched with fresh context.
- the work benefits from NOT sharing the caller's context — a checker, a red team, a
  scoped builder. A checker that shares the worker's context "is nodding along to itself
  in a different font"; fresh context is the default, sharing is the exception you justify.
- the same role recurs across tasks; a skill would restate "who you are" every run.

Skill = a recipe any agent follows. Workflow = a topology over ≥2 agents. If the
candidate is a recipe wearing a name badge, it is a skill — author it with skill-author.

## anatomy of a definition

```
---
name: <kebab-role>        # a role, not a task ("finding-skeptic", not "check-pr-123")
description: <use-when + skip-when — the dispatcher reads ONLY this line>
tools: Read, Grep, Bash   # minimal allowlist, named tools; "*" needs a written reason
model: sonnet             # compiled from config/model-tiers.json; never hand-picked
---
<body = the agent's system prompt: protocol, embedded contract>
```

- name: one role. If it needs an "and", it is two agents.
- description: states use-when AND skip-when. The dispatcher never reads the body, so
  the description does all the routing work. Breadth per skill-author's trigger rule:
  the widest use-when the evidence permits; narrow only on an observed misroute.
- tools: grant the minimum the job needs; start read-only, add on proof from eval runs.
  A checker with Edit will fix instead of grade.
- model, chosen by task shape and expressed as a TIER (docs/routing.md, ids in
  config/model-tiers.json):
  - mechanical transform, extraction, formatting, dedupe → T2.
  - bounded build or research with a checkable pass signal → T3.
  - judgment — adversarial review, ambiguous tradeoffs, taste → T4.
  Register the agent's tier in the tier file's `agents` map. A pi definition carries NO
  model line; the installer compiles the tier into settings overrides. A Claude Code
  definition carries the tier's floating alias, and the installer rewrites that line on
  drift. Never hand-pick a model id. Cheap tiers on boring nodes, strong tiers where
  judgment lives. "T4 everywhere" is habit, not a decision.

The body opens with the protocol, three blocks in order:

1. input contract — what arrives in the dispatch prompt, named and shaped (the article's
   `IN: { competitor, url } — passed in, never assumed`). A missing input is reported
   back by name, never guessed at, never fished out of ambient context.
2. output contract — a fixed shape (fenced block or JSON(JavaScript Object Notation)
   schema) the next node consumes without guessing. Schema over free text: free text is
   output only a human can read; a shape violation is rejected and retried. Within the
   shape, verbose beats terse between agents (CLAUDE.md agent-communication rule).
3. context discipline — the dispatch carries only what this step needs. Name what the
   agent must NOT receive (the worker's chat, prior votes, the session transcript);
   an exclusion you don't write down is one the dispatcher will violate.

Before writing the body, read ~/Documents/agents/docs/prompt-style.md — the Simplified
Technical English (ASD-STE100) rules that leave a sentence one reading. A system prompt
is read once by a fresh context, so a second reading of a line is a second behavior.

## the embedded contract

Every authored agent ships these three sections in its body — the exact gap the old
fleet left open:

- trigger conditions: the situations that warrant dispatch and the near-misses that
  don't. Mirrors the description but binds the agent itself: dispatched outside its
  trigger, it says so and stops instead of improvising.
- success rubric: what "worked" looks like, checkable by the dispatcher without redoing
  the work — "all N inputs accounted for in the output block", "verdict cites file:line
  plus a repro command". Never "did a good job".
- failure-mode watch-list: the 3-5 ways THIS role goes wrong, named preemptively, each
  as symptom + the check that catches it. Section "## failure modes" below seeds the
  generic four; add the role-specific ones.

## worked example

The checker role, where the contract bites hardest:

```
---
name: finding-skeptic
description: Use to attack ONE finding produced by another agent — try to kill it before it merges. One finding per dispatch, always fresh context. Skip for grading whole reports (workflow) and for fixing anything.
tools: Read, Grep, Bash
model: opus
---
You attack one finding. You never fix, never soften.

input contract: {finding, source_path, claim} in the dispatch prompt. You never see the
worker's chat. Missing field → verdict: invalid-dispatch, name the field.

output contract (exactly this block):
  verdict: keep | drop | invalid-dispatch
  reason:  <one clause, anchored>
  anchor:  <the command you ran and its output, or file:line you read>

trigger conditions: one finding, produced by another agent, not yet merged.
not: your own output, style nits, whole reports.

success rubric: verdict present; reason cites a real anchor; zero files modified.

failure-mode watch-list:
- rubber-stamp: >80% keep across a batch → dispatcher spot-audits a sample
- scope grab: reads beyond source_path → note it in reason or the run is suspect
- fix reflex: any file modification is an automatic failed run
```

## declare and check a handoff, even without a `.md` file (AGNT-0088)

A workflow script (`workflows/*/*.workflow.js`) dispatches subagents too, without a named
agent definition behind every call. The input/output contract this skill requires for a
named `.md` agent still applies — a workflow author just declares and checks it inline,
since a `.workflow.js` script cannot `import` another file at runtime (no filesystem or
Node.js(JavaScript runtime) API access inside the sandboxed script).

Two ways to declare and check a handoff shape, by what the receiving agent's own output
contract already is:

- **The receiving agent's contract is already JSON-shaped, or can be** — pass `schema`
  (a JSON(JavaScript Object Notation) Schema) on the `agent()` call. A payload that
  doesn't match is rejected back to the SAME child, which corrects it, before the call
  ever returns to the caller. `workflows/scheduled-ideation/scheduled-ideation.workflow.js`'s
  Generate phase does this today (`schema: {candidates: [...]}`) — this is the default,
  reach for it first.
- **The receiving agent's contract is a fixed fenced-block format, not JSON** (a documented
  choice, not a gap — free text with a checkable shape reads better as prose than as
  forced JSON fields) — write a small structural check and, on failure, correct the SAME
  child with `agent()`'s `resume` option (never a fresh dispatch) before accepting the
  result. `workflows/research-sweep/research-sweep.workflow.js`'s `runResearchers` is the
  worked example: `isValidFindingsBlock(text)` checks the response against
  `agents/web-research-summarizer/web-research-summarizer.md`'s own documented output
  contract, and a failing response gets ONE same-child correction via `resume` before the
  dispatch is treated as failed.

Either way, the rule is the same one this skill already states for a named agent's own
output contract: **a shape violation is rejected and retried, never accepted as complete
because something came back.** A workflow's own separate coverage-retry mechanism (a
gap-check + fill-gap round, a missing-angle re-dispatch) stays reserved for its actual
job — an angle nobody covered at all — never repurposed as the first line of defense
against a malformed single response. Catch a bad shape at the dispatch that produced it,
before it reaches anything downstream.

No universal field set is being prescribed here. `workflows/research-sweep/research-sweep.workflow.js`'s
`DISPATCH_SCHEMA` (label, objective, boundaries, source_guidance, recency) fits that
workflow's own planner-to-researcher handoff; a role with a genuinely different shape
(`debugger`'s `repro_command`/`expected`/`actual`, `anchor-verifier`'s
`work_product_paths`/`verify_command`/`rubric`) keeps its own fields. What generalizes is
the PATTERN — name the expected shape, check it, correct in place — not one fixed
vocabulary forced onto every dispatch.

## evals

Every authored agent ships `evals/` per skills/ai-author/templates/eval-harness.md
(cases.jsonl, rubric.md, run.sh — holdout gating included). Agent-specific rules:

- cases are scenario cases: realistic dispatch prompts carrying the input contract.
  Include ≥1 out-of-trigger case (expect: declines per trigger conditions) and ≥1
  missing-input case (expect: reports the gap by name, does not guess).
- the rubric grades on anchors — real command output, real test runs, a diff that
  exists on disk — never the agent's self-report. "Tests pass" scores only if run.sh
  ran the tests; "should pass" scores zero. Self-report is the false-pass catastrophic
  case in every agent's rubric.
- run.sh dispatches the agent fresh per case (its own context, never the grader's),
  checks the output shape first, then the anchors.

No harness = not done.

## failure modes

Design against these four; the watch-list in each authored agent names the role-specific
versions.

1. role creep — the agent accumulates a second job ("verify it, and fix what you find").
   ONE job per node; the second job is a second agent or a workflow edge.
   Check: can the description still state the job in one clause?
2. context bloat — the dispatch carries the whole session "for background". Only what
   the step needs; every extra token is noise the agent must ignore, paid for.
   Check: drop each input in an eval case; if the output contract still fills, cut it.
3. self-verification — an agent grading its own work. Models miss most of their own
   mistakes; a worker and its checker never share a context.
   Check: any "review your work before finishing" step in a builder's body — cut it,
   the rubric belongs to a fresh checker judging anchors.
4. tool over-grant — `tools: "*"` by habit. Every unneeded tool is a live failure mode:
   the checker that edits, the researcher that commits.
   Check: which tools did the eval cases actually use? Revoke the rest.
