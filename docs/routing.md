# model routing

The one policy for which model gets which work, across Pi, Claude Code, and every
dispatch. Distilled from docs/routing-research-2026-08-20.md; that file holds the full
rationale. Prices are $/Mtok input/output from the live model registry, 2026-08-20 —
re-check them before you lean on a price argument.

## tiers

| tier | role | model | price | fallback |
|------|------|-------|-------|----------|
| T0 | deterministic tools, scripts, grep, tests | — | $0 | — |
| T1 | disposable bounded workers | openrouter/free | $0 | gpt-5.6-luna |
| T2 | cheap summarization, classification, boilerplate | openai-codex/gpt-5.6-luna | 0.2/1.2 | anthropic/claude-haiku-4-5 (1/5) |
| T3 | normal engineering: build, test, debug, verify | anthropic/claude-sonnet-5 | 2/10 | openai-codex/gpt-5.6-terra (2/12) |
| T4 | hard problems, planning, final review | anthropic/claude-opus-5 | 5/25 | openai-codex/gpt-5.6-sol (5/30) |
| T5 | project-level synthesis, deep architecture | anthropic/claude-fable-5 | 10/50 | claude-opus-5, then gpt-5.6-sol |

Every fallback crosses provider families on purpose: a provider outage or usage-limit
stop degrades one tier sideways instead of failing the run.

## the four rules

1. **The orchestrator is sticky.** One session runs one top-level model — claude-sonnet-5
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
   repeated T4 failure. On outage, limit, or refusal, claude-opus-5 takes over
   automatically. A rename, a summary, a small test, a localized fix — never T5.
4. **Cheap workers do the volume, but only bounded verifiable work.** T1/T2 take
   classification, log and doc summarization, boilerplate, and candidate tests. A T0
   check or a T3 reviewer must grade the output cheaply. They never take architecture,
   UX direction, security-sensitive design, migrations, or final acceptance review.
   A free model that causes one extra correction turn was not free.

## fleet pins

The dispatch layer is where routing lives; the fleet definitions carry the pins.

| agent | tier | pi (`pi/agents/`) | claude code (`agents/`) |
|-------|------|-------------------|--------------------------|
| web-research-summarizer | T2 | gpt-5.6-luna → haiku-4-5 | haiku |
| debugger | T3 | sonnet-5 → terra | sonnet |
| spec-tester | T3 | sonnet-5 → terra | sonnet |
| maestro-tester | T3 | sonnet-5 → terra | sonnet |
| anchor-verifier | T3 | sonnet-5 → terra | sonnet |
| code-reviewer | T4 | opus-5 → sol | opus |

The anchor-verifier seat runs per builder wave and per break panel, the highest volume of
any reviewer. It grades on executed evidence, not judgment, so it rides T3. The
code-reviewer seat gives the final coherence verdict and stays T4.

## pi session defaults

- `defaultModel: claude-sonnet-5` in `~/.pi/agent/settings.json`. Escalate a session by
  hand with `/model` at a real escalation point; drop back after.
- `pi/agents/` is the versioned fleet; install.sh links `~/.pi/agent/agents` to it.
- OpenRouter calls keep `data_collection: deny` + `zdr` when they carry repo content.

## what this deliberately skips

The research proposes a control plane: decision ledger, context compiler, learned router,
telemetry-driven thresholds. It stays unbuilt until a routing failure appears that this
policy cannot express. The engineer skill's gates already own steering; its phase files
already own per-phase context. Static pins plus rule 2 are the whole router.
