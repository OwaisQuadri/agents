# model routing

The one policy for which model gets which work, across Pi, Claude Code, and every
dispatch. Distilled from research/pi-harness-routing-research-fable-opus.md; that file holds the full
rationale.

Model ids live in ONE file: `config/model-tiers.json`. Prose and skills name tiers, never
models. To swap a model, edit that file and run install.sh. You can also use `/tiers`.
Each model in a tier's chain carries its OWN `thinking` level. That includes the primary
and each fallback.

`tools/tier-dispatch` also reads this file. It resolves a tier to its ordered model
chain for the ai-author evaluation harness. The primary model comes before its listed
fallbacks. A quota error moves the dispatch to the next model in that chain. Model
limits can differ within one provider, so the dispatch tests every configured fallback.
Exhaustion makes the complete tier unavailable.

The `--verify-registry` mode checks each configured tier entry against the model records in
Pi's local registry. It also reports stale overrides for available providers. The tool
looks for `models.json` beside the supplied tiers file. Use `--models-file` when the check
needs another file. The tool reports a missing default overrides file as a standard-error
advisory. It rejects a missing explicit file or a malformed file.

This mode does not dispatch a model or write a file. The tool never derives a model
identifier from another source.

Prices live in the model registry (`~/.pi/agent/models-store.json`). Re-check prices before
you lean on a price argument.

## reconcile tiers

Run the following command before a tier change and after a registry refresh:

```sh
cargo run --quiet --manifest-path tools/tier-dispatch/Cargo.toml -- \
  --verify-registry --tiers-file config/model-tiers.json
```

Exit 0 means every tier entry resolves in the registry. Exit 1 names each missing tier
entry. Exit 2 means that a supplied input is invalid, or that the tiers file or registry is
unavailable. The command reports missing default overrides and newer unreferenced family
members as advisories on standard error. Advisories never change its exit code.

## tiers

| tier | role |
|------|------|
| T0 | deterministic tools, scripts, grep, tests |
| T1 | disposable bounded workers (cheap, not necessarily free) |
| T2 | cheap summarization, classification, boilerplate |
| T3 | normal engineering: build, test, debug, verify |
| T4 | hard problems, planning, final review |
| T5 | project-level synthesis, deep architecture (falls back to T4) |

Each tier's `fallbacks` list crosses provider families on purpose. A provider outage or a
usage-limit stop then degrades one tier sideways instead of failing the run. The list is
ordered and each entry names its own model and thinking level.

## the four rules

1. **The orchestrator is sticky.** One session runs one top-level model. It uses the T3
   model by default. Switch only at a phase boundary, a user override, a provider failure,
   or an escalation gate. Per-turn model hopping breaks prompt-cache locality and reasoning
   continuity. Cache reads cost 10x less than fresh input on every tier.
2. **Escalate on evidence.** After two meaningful failures, a task moves up one tier, on
   the other provider family. Failed tests, a diff outside scope, or two
   turns with no progress count as failures. A disliked answer does not. A direction
   disagreement goes back to the human gate, never to a bigger model.
3. **Gate fable; never make it the default.** T5 takes decisions with large downstream
   branching cost. That means architecture for a large migration, synthesis after
   competing plans, the final coherence review of a long project, or recovery after
   repeated T4 failure. On outage, limit, or refusal, the T5 fallback takes over
   automatically. Never use T5 for a rename, a summary, a small test, or a localized fix.
4. **Cheap workers do the volume, but only bounded verifiable work.** T1/T2 take
   classification, log and doc summarization, boilerplate, and candidate tests. A T0
   check or a T3 reviewer must grade the output cheaply. They never take architecture,
   UX direction, security-sensitive design, migrations, or final acceptance review.
   A free model was not free when it causes one extra correction turn.

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

ONE definition per role lives in `agents/<name>/<name>.md`. The installer derives
everything else from it.

- Pi: the installer GENERATES `~/.pi/agent/agents/<name>.md` from that definition. Same body, tool
  names mapped to pi's registry (`Glob` to `find`, `WebSearch` to `web_search`), and no
  model line at all. Subagents receive a tier's whole ordered `fallbacks` list through
  `subagents.agentOverrides`. The session map (`modelTierFallbacks`) holds each model in
  that ordered chain. It keys each model to the model that comes next. The session walks one
  hop per limit, re-enters on the new model, then looks that model up on its own next limit.
  The installer nests the map by tier. A shared model keeps a distinct next hop in each
  tier. An unmapped tool aborts the install instead of dropping a capability grant in
  silence.
- Claude Code: no override layer exists, so the frontmatter must carry a model alias. The
  installer DERIVES that alias instead of reading a declared one. It walks the tier's
  chain for the first Anthropic model and takes the family word out of the id. A chain
  holding none climbs to the next tier in tier-name order until one does. Never edit that
  line by hand.
- New: `/tiers`, a Pi command (`pi/extensions/tier-settings.ts`) for editing tiers and their
  models interactively. Browse T1-T5. Drill into a tier's primary and ordered fallbacks.
  Edit one model or thinking level. Confirm to write the file and re-run install.sh.

The anchor-verifier seat runs per builder wave and per break panel, the highest volume of
any reviewer. It grades on executed evidence, not judgment, so it rides T3. The
code-reviewer seat gives the final coherence verdict and stays T4.

## pi session defaults

- The session `defaultModel` follows the `orchestrator` tier (T3). The installer does not
  enforce it, so a deliberate `/model` choice survives a pull. Escalate a session by hand
  at a real escalation point; drop back after.
- The SESSION model falls back on a usage limit, the same way a dispatch does. Pi core has
  no session fallback, so `pi/extensions/usage-limit-continue.ts` does it. On a usage limit
  it reads the tier-nested `modelTierFallbacks` map that the installer compiles into pi
  settings. It calls `setModel` on the tier's backup. It then calls `setThinkingLevel` on
  that backup's own thinking level. A session resolves its tier once, at its first hop,
  from `tierPrimaries`. Every later hop reuses the resolved tier. It does not re-derive the
  tier from a model id that could belong to more than one tier's chain.
- The chain ends by itself. Every model in a tier's own chain is a key in that tier's own map.
  Each key points at the next model and its thinking level. A session walks one hop per
  limit. It keeps walking as each new model hits its own limit. It stops on a peer that has
  no entry within that tier. When a tier's own chain runs out entirely, `climbOnExhaustion`
  names the next tier.
  T1 rises to T2. T5 drops to T4. The next tier starts from its own primary. The settings UI
  does not depend on model ids that happen to overlap at chain boundaries.
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
run on nearly every reply. A floor on any of them would floor the whole session, not only
one task. `ste-check` already grades their output, which is the cheap catch
that makes the floor unnecessary.

Every floor is a hypothesis. They rest on judgment today, and the bottom-up sweep in
AGNT-0018 is what can prove one wrong.

## what T1 cost, measured

T1 has no agent assigned, and four runs on 2026-08-20 are why. Those runs used
`openrouter/openrouter/free`, T1's primary at the time. The free model answered a
file-count question correctly in 10 tool calls and 92 seconds, five of them rewriting one
file. It broke its output shape on a one-word reply.

A read-only grant and a three-call budget then cut it to one call on a log. It still
numbered every quoted line from 1, instead of using the file's own numbers. On the next
run it returned nothing at all. The T1 fallback carried that work to T2, which finished in
one turn for a fifth of a cent.

One clean run in four. That result is why T1's primary later moved off the free model.
A per-model comparison picked `gpt-5.6-luna@low` instead. Read the original four runs as a
measurement of that one model. Never read them as a verdict on free models in general.

`log-summarizer` still sits at T2, and T1 still has no agent assigned. The fallback held
every time the free model failed, and the cost of trying it was latency, not money.

## what this deliberately skips

The research proposes a control plane: decision ledger, context compiler, learned router,
telemetry-driven thresholds. It stays unbuilt until a routing failure appears that this
policy cannot express. The engineer skill's gates already own steering; its phase files
already own per-phase context. Static pins plus rule 2 are the whole router.
