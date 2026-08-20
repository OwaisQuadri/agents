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

Each tier's `fallback` crosses provider families on purpose. A provider outage or a
usage-limit stop then degrades one tier sideways instead of failing the run.

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

| agent | tier |
|-------|------|
| web-research-summarizer | T2 |
| debugger | T3 |
| spec-tester | T3 |
| maestro-tester | T3 |
| anchor-verifier | T3 |
| code-reviewer | T4 |

How the assignment reaches each harness:

- Pi: the `pi/agents/` frontmatter carries NO model. The installer compiles the tier file
  into `subagents.agentOverrides` (model, cross-provider fallback, thinking) plus
  `subagents.defaultModel` and `subagents.defaultThinking` in `~/.pi/agent/settings.json`.
  Frontmatter without a model falls through to those overrides.
- Claude Code: no override layer exists, so `agents/*/` frontmatter must carry the tier's
  floating alias, named by the tier file's `claude` field. That line belongs to the
  installer, which rewrites it when it drifts from the tier file. Never edit it by hand.

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
  A model outside the tier file keeps the old behavior, a scheduled resume at reset time.
- `pi/agents/` is the versioned fleet; the installer links `~/.pi/agent/agents` to it.
- OpenRouter calls keep `data_collection: deny` + `zdr` when they carry repo content.

## what this deliberately skips

The research proposes a control plane: decision ledger, context compiler, learned router,
telemetry-driven thresholds. It stays unbuilt until a routing failure appears that this
policy cannot express. The engineer skill's gates already own steering; its phase files
already own per-phase context. Static pins plus rule 2 are the whole router.
