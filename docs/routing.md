# model routing

The one policy for which model gets which work, across Pi, Claude Code, and every
dispatch. Distilled from research/pi-harness-routing-research-fable-opus.md; that file holds the full
rationale.

Model ids live in ONE file: `config/model-tiers.json`. Prose and skills name tiers, never
models. To swap a model, edit that file and run install.sh. Each tier also carries a
`thinking` level, and the installer compiles it with the model. Prices live in the model
registry (`~/.pi/agent/models-store.json`); re-check them before you lean on a price
argument.

## tiers

| tier | role |
|------|------|
| T0 | deterministic tools, scripts, grep, tests |
| T1 | disposable bounded workers (free) |
| T2 | cheap summarization, classification, boilerplate |
| T3 | normal engineering: build, test, debug, verify |
| T4 | hard problems, planning, final review |
| T5 | project-level synthesis, deep architecture (falls back to T4) |

Each tier's `fallbacks` list crosses provider families on purpose. A provider outage or a
usage-limit stop then degrades one tier sideways instead of failing the run. The list is
ordered, so T3 leads with codex-spark for bounded engineering work, then sonnet, then
terra.

## the four rules

1. **The orchestrator is sticky.** One session runs one top-level model — the T3 model
   by default. Switch only at a phase boundary, a user override, a provider failure, or
   an escalation gate. Per-turn model hopping breaks prompt-cache locality and reasoning
   continuity. Cache reads cost 10x less than fresh input on every tier.
2. **Escalate on evidence.** After two meaningful failures, a task moves up one tier, on
   the other provider family. Failed tests, a diff outside scope, or two
   turns with no progress count as failures. A disliked answer does not. A direction
   disagreement goes back to the human gate, never to a bigger model.
3. **Gate fable; never make it the default.** T5 takes decisions with large downstream
   branching cost. That means architecture for a large migration, synthesis after
   competing plans, the final coherence review of a long project, or recovery after
   repeated T4 failure. On outage, limit, or refusal, the T5 fallback takes over
   automatically. A rename, a summary, a small test, a localized fix — never T5.
4. **Cheap workers do the volume, but only bounded verifiable work.** T1/T2 take
   classification, log and doc summarization, boilerplate, and candidate tests. A T0
   check or a T3 reviewer must grade the output cheaply. They never take architecture,
   UX direction, security-sensitive design, migrations, or final acceptance review.
   A free model that causes one extra correction turn was not free.

## fleet tiers

The dispatch layer is where routing lives. The `agents` map in `config/model-tiers.json`
assigns each fleet role its tier:

| agent | tier | owner |
|-------|------|-------|
| log-summarizer | T2 | repo |
| web-research-summarizer | T2 | repo |
| researcher | T2 | package |
| scout | T2 | package |
| debugger | T3 | repo |
| spec-tester | T3 | repo |
| maestro-tester | T3 | repo |
| anchor-verifier | T3 | repo |
| worker | T3 | package |
| reviewer | T3 | package |
| code-reviewer | T4 | repo |
| oracle | T4 | package |

Two package agents stay untiered on purpose, and the tier file records why. `delegate`
inherits the parent model by design, so a pin defeats the role. `gpt-pro` runs through the
external-job bridge, so this router never picks its model.

Thinking splits by owner. `agentOverrides.thinking` OVERWRITES an agent's own frontmatter,
so the installer emits the tier's thinking only for the agents in `pi/agents/`. A package
agent keeps the thinking its author tuned, and only its model follows the tier.

How the assignment reaches each harness:

<!-- TODO(AGNT-0063.T06): Update routing only after replacement agent behavior is verified. -->
ONE definition per role lives in `agents/<name>/<name>.md`. The installer derives
everything else from it.

- Pi: the installer GENERATES `~/.pi/agent/agents/<name>.md` from that definition. Same body, tool
  names mapped to pi's registry (`Glob` to `find`, `WebSearch` to `web_search`), and no
  model line at all. Subagents receive a tier's whole ordered `fallbacks` list through
  `subagents.agentOverrides`. The session map takes the first entry only, because the
  session walks one hop per limit and re-enters on the new model. An unmapped tool aborts
  the install rather than dropping a capability grant in silence.
- Claude Code: no override layer exists, so the frontmatter must carry a model alias. The
  installer DERIVES that alias rather than reading a declared one. It walks the tier's
  chain for the first Anthropic model and takes the family word out of the id. A chain
  holding none climbs to the next tier, which is how the free tier resolves to haiku
  there. Never edit that line by hand.

The anchor-verifier seat runs per builder wave and per break panel, the highest volume of
any reviewer. It grades on executed evidence, not judgment, so it rides T3. The
code-reviewer seat gives the final coherence verdict and stays T4.

## pi session defaults

- The session `defaultModel` follows the `orchestrator` tier (T3). The installer does not
  enforce it, so a deliberate `/model` choice survives a pull. Escalate a session by hand
  at a real escalation point; drop back after.
- The SESSION model falls back on a usage limit, the same way a dispatch does. Pi core has
  no session fallback, so `pi/extensions/usage-limit-continue.ts` does it. On a usage limit
  it reads the flat `modelTierFallbacks` map that the installer compiles into pi settings.
  It then calls `setModel` on the tier's backup, and the session continues.
- The chain ends by itself. Only tier primaries are keys in that map, so a session walks
  one hop per limit and stops on a peer that has no entry. Every hop holds its tier except
  two. T1 has no free peer and rises to T2, and T5 has no peer and drops to T4.
- When the chain runs out, the session waits on the model that RETURNS FIRST, not the one
  that failed last. The extension records each abandoned model with its reset, schedules
  the resume at the soonest of them, and sets the session back to that model. A model
  outside the tier file keeps the old behavior, a scheduled resume at its own reset.
- `pi/agents/` is the versioned fleet; the installer links `~/.pi/agent/agents` to it.
- OpenRouter calls keep `data_collection: deny` + `zdr` when they carry repo content.

## skill floors

A skill runs on the session model, and it cannot change that. So a skill whose work needs
capability declares `metadata.minimum-tier`, and AGENTS.md tells the runner to flag a
session sitting below it.

A floor goes on only where a cheaper model fails in a way the user cannot cheaply catch.
That test, not seniority, decides:

- T4 for judgment, taste, and ambiguity, where a wrong answer is expensive and no command
  proves it wrong. The four authoring skills, `agent-config-reset`, `byline`, `ladder`.
- T3 for bounded work carrying real blast radius or structure a small model loses.
  `engineer`, `git-sync`, `hq`, `task-graph`, `footprint`, `vocabulary`.
- No floor where the work is mechanical, or where a checker already grades the output.
  `create-pr`, `rust-style`, `session-stats`, `volley`.

The register skills carry no floor ON PURPOSE. `mouthpiece`, `bro`, and `computah-voice`
run on nearly every reply. A floor on any of them would floor the whole session, rather
than one task. `ste-check` already grades their output, which is the cheap catch
that makes the floor unnecessary.

Every floor is a hypothesis. They rest on judgment today, and the bottom-up sweep in
AGNT-0018 is what can prove one wrong.

## what T1 cost, measured

T1 has no agent assigned, and four runs on 2026-08-20 are why. The free model answered a
file-count question correctly in 10 tool calls and 92 seconds, five of them rewriting one
file. It broke its output shape on a one-word reply.

A read-only grant and a three-call budget then cut it to one call on a log. It still
numbered every quoted line from 1, instead of using the file's own numbers. On the next
run it returned nothing at all. The T1 fallback carried that work to T2, which finished in
one turn for a fifth of a cent.

One clean run in four. So `log-summarizer` sits at T2, and T1 stays defined and unused
until some task shape earns it. Read that as measurement, never as a verdict on free
models. The fallback held every time, and trying cost latency rather than money.

## what this deliberately skips

The research proposes a control plane: decision ledger, context compiler, learned router,
telemetry-driven thresholds. It stays unbuilt until a routing failure appears that this
policy cannot express. The engineer skill's gates already own steering; its phase files
already own per-phase context. Static pins plus rule 2 are the whole router.
