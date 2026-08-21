# Pi Harness Token-Dollar Efficiency
## Fable 5 + Opus 5 fallback, automated routing, phased steering, and retrofit of an existing engineering skill

**Research date:** 2026-08-20

## Executive recommendation

Do not replace the existing multi-phase engineering/UX workflow.

Keep its:
- phase semantics
- user steering points
- UX/design approval gates
- architecture decisions
- acceptance criteria
- verification requirements

Replace the inference layer underneath it.

Recommended architecture:

```text
                             USER
                              │
                    steering / approvals
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ EXISTING MULTI-PHASE ENGINEERING SKILL                      │
│                                                             │
│ same phases · same UX gates · same architecture checkpoints │
└────────────────────────────┬────────────────────────────────┘
                             │ PhaseContract
                             ▼
┌─────────────────────────────────────────────────────────────┐
│ CONTROL PLANE                                               │
│                                                             │
│ Decision Ledger     Context Compiler     Budget Manager      │
│ Phase State         Router               Health/Quota State  │
│ Acceptance Tests    Telemetry            Privacy Policy      │
└────────────────────────────┬────────────────────────────────┘
                             │
                   route(request / subtask)
                             │
        ┌────────────────────┼──────────────────────┐
        ▼                    ▼                      ▼
 deterministic          cheap workers          orchestrators
  tools / tests       free / Luna / Terra   Sonnet / Opus / Fable
        │                    │                      │
        └────────────────────┴──────────┬───────────┘
                                       ▼
                              verified artifact
```

### Primary policy

Use **Fable 5 selectively as the deepest orchestration tier**, not as the default model for every turn.

Use **Opus 5 as Fable's automatic fallback** for:
- Fable availability/capacity failure
- timeout/provider health degradation
- Fable budget ceiling reached
- router confidence that Fable's incremental value does not justify its price
- explicit user override
- eligible refusals where another model can safely complete the task

Use **T3 as the normal interactive engineering brain**.

Use **GPT-5.6 Luna / free OpenRouter models** for cheap, bounded, verifiable workers.

Use **GPT-5.6 Terra** as a strong cross-provider middle tier and recovery route.

Suggested hierarchy:

```text
T0  deterministic tools             $0
T1  OpenRouter free                 $0
T2  Haiku / GPT-5.6 Luna            ultra-cheap worker
T3  Codex-Spark / Sonnet / Terra    normal engineering
T4  Opus / Sol                      complex tasks/planning and hard engineering
T5  Fable or fallback to T4         intense and deeply complex orchestration, planning, and systems design.
```

Fable should be treated as a scarce resource.

---

# 1. Where Fable belongs

Fable is most useful for:
- ambitious coding projects
- large migrations
- long-running agentic work
- complex multi-stage work
- high-fidelity implementation against designs
- self-verification
- cross-phase synthesis

That maps well to the top of the routing tree.

It does not imply “use Fable for every user turn.”

Better:

```text
ordinary interactive engineering
        │
        ▼
       T3
        │
   hard / risky?
      /     \
    no       yes
    │         │
 continue     ▼
             T4
              │
     project-level uncertainty?
        /             \
      no               yes
      │                 │
   continue             ▼
                        T5
```

Use Fable when the task has large downstream branching cost.

Examples:
- choosing architecture for a large migration
- resolving contradictory constraints across several phases
- redesigning a subsystem with many invariants
- synthesizing competing implementation strategies
- repairing a workflow after repeated model failures
- reviewing whether a long implementation still matches the original UX intent
- deciding how to split a large engineering task into independently delegatable units

Bad Fable uses:
- rename symbol
- summarize logs
- write a small test
- inspect a compiler error
- edit a single view
- routine repo search
- format output
- run verification commands
- simple localized bug fixes

---

# 2. Fable primary + Opus fallback

Recommended fallback path:

```text
                     ┌────────────────┐
                     │ FABLE 5        │
                     │ deep primary   │
                     └───────┬────────┘
                             │
        ┌────────────────────┼─────────────────────┐
        │                    │                     │
      success             refusal              outage
        │                    │                     │
        ▼                    ▼                     ▼
     accept            policy allows?       provider unhealthy
                             │                     │
                        yes  ▼                     │
                         OPUS 5  ◄─────────────────┘
                             │
                   failure / low confidence
                             ▼
                    GPT-5.6 SOL / TERRA
```

### Fable → Opus fallback triggers

```ts
fableUnavailable
|| fableTimedOut
|| fableRateLimited
|| fableProviderHealth < 0.90
|| fableRefusedAndOpusEligible
|| projectedFableCost > request.budget
|| sessionFableBudgetRemaining <= reserve
```

Do not automatically fall back merely because Fable produced an answer you dislike.

Classify the failure:

```text
provider failure       → same task to Opus
policy-safe refusal    → Opus only if eligible
format/schema failure  → one constrained Fable retry, then Opus
test failure           → Opus reviewer or alternative strategy
bad product direction  → return to user steering gate
```

A UX/design disagreement is not a “retry until a model agrees.” It returns to the user-controlled decision point.

---

# 3. Fable activation gate

Fable should be activated when its expected reduction in downstream failure cost exceeds its price premium.

Conceptually:

```text
use Fable iff

P(Opus failure) × downstreamFailureCost
  - P(Fable failure) × downstreamFailureCost

> FableCost - OpusCost
```

Bootstrap scoring:

```text
scope                0–20
cross-phase coupling 0–20
ambiguity            0–15
irreversibility      0–15
UX/product impact    0–10
architecture impact  0–10
prior failures       0–10
--------------------------------
total                0–100
```

Initial policy:

```text
0–49   → never Fable
50–69  → Opus/Sonnet
70–84  → Fable eligible
85–100 → Fable preferred
```

Hard Fable candidates:
- multi-day/multi-phase migrations
- cross-cutting architecture decisions
- synthesis after competing plans
- final coherence review across a long project
- repeated failure after Opus/Sol
- explicit maximum-quality request

Hard Fable exclusions:
- deterministic tasks
- low-risk localized changes
- tasks where tests fully determine correctness
- routine code generation
- large raw-context summarization that a cheaper model can compress first

---

# 4. Do not constantly switch the top-level Pi model

The user-facing orchestrator should be sticky.

Bad:

```text
turn 1 Fable
turn 2 Luna
turn 3 free model
turn 4 Opus
turn 5 Sonnet
turn 6 Fable
```

This damages:
- prompt cache locality
- reasoning continuity
- design/UX consistency
- predictable steering
- cost accounting

Better:

```text
                     sticky orchestrator
                         Sonnet 5
                            │
             ┌──────────────┼──────────────┐
             ▼              ▼              ▼
           Luna          OR-free        Terra
           worker         worker         worker
             │              │              │
             └──────────────┴──────┬───────┘
                                   ▼
                              Sonnet decides
                                   │
                     hard phase / escalation
                              /          \
                           Opus          Fable
```

Switch the top-level model mainly at:
- explicit phase boundaries
- escalation gates
- user override
- provider failure
- high-value review checkpoints

Use cheap/free models as subcalls/workers.

---

# 5. Preserve steering through a Decision Ledger

Routing must never own product intent.

Create one canonical ledger:

```yaml
objective:
  ...

ux_principles:
  - ...

architecture_decisions:
  - id: ADR-004
    decision: ...
    rationale: ...
    status: accepted

rejected_options:
  - option: ...
    reason: ...

hard_constraints:
  - ...

open_questions:
  - ...

current_phase:
  implementation

acceptance_criteria:
  - ...

user_steering:
  pending: false
```

Models may propose changes:

```yaml
proposed_decision_change:
  decision_id: ADR-004
  proposed_value: ...
  reason: ...
```

They may not commit the change.

Only the existing steering/approval mechanism commits it.

```text
model proposes
      │
      ▼
existing decision gate
      │
   user decides
    /       \
 accept     reject
   │          │
ledger      preserve old
update      decision + reason
```

This protects engineering and UX direction across model switches.

---

# 6. Retrofit the existing multi-phase engineering skill

Migration rule:

> Preserve workflow semantics. Optimize inference mechanics.

Wrap each existing phase in a `PhaseContract`.

Example:

```yaml
phase: ux_design

goal:
  Resolve the intended user interaction before implementation.

required_inputs:
  - product_goal
  - accepted_requirements
  - existing_ui_constraints
  - relevant_decision_ledger_entries

hard_constraints:
  - preserve accepted navigation model
  - do not implement before approval

routing:
  minimum_tier: T3
  preferred: claude-sonnet-5
  fable_eligible: true

steering_required: true

output:
  - proposed_flow
  - alternatives
  - tradeoffs

completion:
  - user has chosen a direction

survive_to_next_phase:
  - chosen_flow
  - rejected_flows
  - rationale
```

Suggested skill structure:

```text
engineering/
├── SKILL.md
├── references/
│   ├── discovery.md
│   ├── product-design.md
│   ├── ux-design.md
│   ├── architecture.md
│   ├── planning.md
│   ├── implementation.md
│   ├── verification.md
│   └── final-review.md
├── schemas/
│   ├── decision-ledger.json
│   ├── phase-contract.json
│   └── route-trace.json
└── scripts/
    ├── route.ts
    ├── context.ts
    ├── health.ts
    ├── budget.ts
    └── metrics.ts
```

Keep `SKILL.md` small.

Load detailed phase instructions only when active.

---

# 7. Recommended phase routing

| Phase | Main model | Workers | Fable? | User steering |
|---|---|---|---|---|
| Intake | Sonnet | Luna | rare | when ambiguity changes scope |
| Repo discovery | Sonnet sticky | T0/free/Luna | no | no |
| Product/UX design | Sonnet or Opus | Luna/Terra research | eligible | yes |
| Architecture | Sonnet/Opus | Luna/Terra | eligible/preferred on large systems | yes |
| Plan synthesis | Sonnet/Opus | Luna | eligible | existing gate |
| Implementation | Sonnet | Luna/free/Terra | almost never | continuously steerable |
| Debugging | Sonnet → Opus | T0/Luna/Terra | after repeated hard failure | only if direction changes |
| Verification | Sonnet | T0/Luna | no | only on unresolved problems |
| UX review | Sonnet/Opus | vision/tool workers | eligible on large changes | yes |
| Final coherence review | Opus | T0/Terra | preferred for major projects | final gate |

---

# 8. Automated router

Hard-filter first.

Score second.

```ts
interface RouteRequest {
  phase: Phase
  task: TaskClass

  contextTokens: number
  expectedOutputTokens: number

  complexity: number
  risk: number
  ambiguity: number
  crossPhaseCoupling: number
  uxImpact: number
  architectureImpact: number
  priorFailures: number

  privacy: "public" | "internal" | "sensitive"

  requiresTools: boolean
  requiresVision: boolean
  requiresStructuredOutput: boolean

  qualityFloor: number
  maxCostUSD?: number
  maxLatencyMs?: number

  currentModel?: ModelID
}
```

Core policy:

```ts
function route(req: RouteRequest): Candidate {
  const feasible = registry.models
    .filter(m => fitsContext(req, m))
    .filter(m => supportsCapabilities(req, m))
    .filter(m => privacyAllowed(req, m))
    .filter(m => providerHealthy(m.provider))
    .filter(m => estimatedCost(req, m) <= budgetFor(req))

  const ranked = feasible
    .map(model => ({
      model,
      pSuccess: successEstimator.predict(req, model),
      expectedCost: expectedAcceptedArtifactCost(req, model)
    }))
    .filter(x =>
      lowerConfidenceBound(x.pSuccess) >= req.qualityFloor
    )
    .sort((a, b) => a.expectedCost - b.expectedCost)

  return applySessionStickiness(ranked, req.currentModel)
      ?? strongest(feasible)
}
```

Goal:

```text
cheapest model confidently capable of producing
an accepted artifact
```

Not “cheapest tokens.”

---

# 9. Optimize expected cost to accepted artifact

Actual economic objective:

```text
ExpectedCost =
    inferenceCost
  + retryCost
  + escalationCost
  + cacheSwitchPenalty
  + quotaShadowCost
  + latencyPenalty
  + expectedUserCorrectionCost
  + expectedTestFailureCost
```

A free worker can still be expensive if it creates:
- compile failures
- retries
- architecture drift
- extra correction turns
- expensive context rereads

---

# 10. Bootstrap routing tiers

Before sufficient telemetry:

```text
complexity 0–24    → T1/T2
complexity 25–49   → T2
complexity 50–69   → T3
complexity 70–84   → T4
complexity 85–100  → T4/T5
```

Forced floors:

| Situation | Minimum |
|---|---|
| summarize compiler output | T1 |
| rename / boilerplate | T1/T2 |
| isolated implementation | T2 |
| multi-file debugging | T3 |
| cross-module refactor | T3 |
| product/UX decision | T3 |
| subtle concurrency/state problem | T3/T4 |
| security/data migration/high blast radius | T4 |
| two failed serious attempts | cross-provider T4 |
| major project-wide synthesis | T5 eligible |
| major final coherence review | T5 preferred |

Initial policy quality floors:

```yaml
worker_low_risk: 0.90
normal_engineering: 0.95
architecture_or_ux: 0.97
critical_change: 0.995
```

These are policy thresholds to tune from observed traces.

---

# 11. Escalation state machine

```text
T0 deterministic
      │ insufficient
      ▼
T1 free
      │ failure / low confidence
      ▼
T2 Luna
      │ failure / scope rises
      ▼
T3 Sonnet/Terra
      │ hard / risky
      ▼
T4 Opus/Sol
      │ cross-phase uncertainty / repeated failure
      ▼
T5 Fable
```

Recovery should often cross provider families.

```ts
if (transientProviderError) {
  retrySameTierDifferentEndpoint()
}

if (schemaFailure || toolProtocolFailure) {
  retrySameModelOnceWithStricterContract()
}

if (
  testsFailed ||
  compileFailed ||
  diffEscapedScope ||
  repeatedToolLoop ||
  noProgressForTwoTurns
) {
  escalateOneTier()
}

if (meaningfulFailures >= 2) {
  switchProviderFamily()
}

if (userRejectsDirection) {
  returnToSteeringGate()
}

if (fableProviderFailure) {
  routeTo("claude-opus-5")
}
```

Never repeat the same failing model/strategy indefinitely.

---

# 12. Context compiler

The routing layer should decide what deserves to reach a model.

Worker context:

```text
immutable rules
+
relevant Decision Ledger slice
+
current PhaseContract
+
relevant code/file excerpts
+
task
+
acceptance test
```

Avoid:

```text
whole conversation
+
all old tool output
+
all rejected alternatives
+
whole repo
```

Stable prefix order:

```text
1. immutable system/workflow rules
2. orchestration contract
3. decision ledger
4. phase contract
5. stable relevant artifacts
6. volatile task context
```

This improves cache reuse.

Fable especially benefits because high-value calls should receive compact, high-quality context rather than raw exploration.

---

# 13. OpenRouter free policy

Use free models as replaceable workers.

Good:
- classification
- log summarization
- compiler-error explanation
- repo-map summaries
- candidate tests
- boilerplate
- public documentation summaries
- simple candidate patches

Avoid for:
- final architecture
- UX direction
- security-sensitive design
- migrations
- irreversible decisions
- proprietary high-value context unless privacy policy is explicit
- final acceptance review

Useful provider restrictions:

```json
{
  "provider": {
    "data_collection": "deny",
    "zdr": true,
    "require_parameters": true,
    "allow_fallbacks": true
  }
}
```

Use capability checks, live health, fallbacks, privacy settings, latency, and cost ceilings.

---

# 14. OpenRouter Auto Router

Useful as:
- benchmark against custom router
- low-risk worker fallback
- exploration mechanism for new models
- shadow-routing baseline

Not ideal as the authoritative engineering router because it does not know:
- your phase semantics
- accepted UX decisions
- Decision Ledger
- user correction cost
- subscription quota scarcity
- repository-specific success rates

Your router should eventually beat it on your own workload.

---

# 15. Subscription-aware pricing

Subscription-backed calls are not economically free.

Use:

```text
effectiveCost =
    meteredDollarCost
  + quotaShadowPrice
```

Example:

```ts
function quotaShadowPrice(q: Quota): number {
  const scarcity = 1 - q.remaining / q.total
  const untilReset = q.hoursUntilReset / q.periodHours

  return BASE_SHADOW_PRICE
    * scarcity ** 2
    * untilReset
}
```

When quota is scarce far from reset:
- premium routes become expensive

When reset is close and much quota remains:
- shadow cost drops
- router can consume it more aggressively

---

# 16. Pi implementation

Recommended Pi extension surface:

```ts
routedWorker({
  task,
  context,
  phase,
  risk,
  privacy,
  qualityFloor,
  acceptance,
})
```

And:

```ts
deepReview({
  artifact,
  decisionLedger,
  phaseHistory,
  preferred: "claude-fable-5",
  fallback: "claude-opus-5",
})
```

Pseudo-implementation:

```ts
async function run(req: RouteRequest) {
  const route = router.choose(req)

  try {
    return await invoke(route.primary, compileContext(req))
  } catch (error) {
    health.recordFailure(route.primary, error)

    const fallback = router.fallback(req, route, error)

    if (!fallback) throw error

    return await invoke(
      fallback,
      compileContext(req, { preserveSemanticState: true })
    )
  }
}
```

Context migration should be semantic, not transcript copying.

When moving Fable → Opus, pass:
- phase contract
- Decision Ledger
- task
- relevant evidence
- failure reason
- acceptance criteria

Do not blindly resend the whole Fable conversation.

---

# 17. Pi/Fable ecosystem precedent

A public Pi harness already demonstrates a Fable-oriented model router with:
- Fable for planning/orchestration
- Sonnet for code-writing tools
- cheaper models for search
- Opus as heavy reasoning
- automatic switching based on tool use

That validates the extension architecture.

For token-dollar efficiency, improve the pattern:

```text
naive:
every new user message → Fable
tool decides route

recommended:
each phase → sticky normal orchestrator
request classifier + phase contract decides route
Fable only after activation gate
Opus automatic fallback
workers remain cheap
```

Tool-trigger routing alone is too coarse.

A `write_file` call may represent:
- boilerplate
- dangerous migration
- subtle concurrency fix
- UX architecture change

The router needs phase/risk/decision context too.

---

# 18. Telemetry

Record:

```ts
interface RouteTrace {
  sessionId: string
  phase: string
  taskClass: string

  selectedModel: string
  provider: string
  tier: number
  routeReason: string

  inputTokens: number
  cachedTokens: number
  outputTokens: number

  dollars: number
  quotaShadowDollars: number
  latencyMs: number

  retries: number
  escalation?: string
  fallback?: string

  compilePassed?: boolean
  testsPassed?: boolean
  scopePassed?: boolean

  userAccepted?: boolean
  userCorrected?: boolean

  steeringViolations: number
}
```

Optimize:
- $/accepted task
- $/accepted implementation
- $/phase
- Fable activation %
- Fable incremental win rate
- Opus fallback rate
- free-tier success %
- Luna success %
- escalation rate
- missed-escalation rate
- user correction turns
- test failure rate by model/tier
- cache miss dollars
- decision/UX drift rate

Do not optimize only:
- tokens/day
- average model price
- percentage of free calls

---

# 19. Learned router

After enough representative traces, fit:

```text
P(acceptedArtifact | taskFeatures, model)
```

Then:

```text
ExpectedAcceptedCost(model) =
  immediateCost(model)
  / P(success | model, task)
  + expectedFallbackCost
  + expectedCorrectionCost
```

Choose:

```text
argmin ExpectedAcceptedCost
```

subject to:
- quality floor
- privacy constraints
- capability requirements
- steering invariants
- phase minimum tier
- model/provider health
- session budget

Let traces determine whether Fable pays for itself on specific classes of tasks.

---

# 20. Migration plan

## Stage 0 — baseline

No behavior change.

Measure:
- tokens
- dollar cost
- model calls
- corrections
- test failures
- phase length
- steering points
- acceptance rate

## Stage 1 — Decision Ledger

Introduce canonical product/architecture/UX state.

No model changes.

Regression requirement:
- same steering behavior

## Stage 2 — Phase Contracts

Explicitly describe:
- inputs
- outputs
- invariants
- acceptance
- steering requirement

## Stage 3 — Context Compiler

Same model.

Reduce unnecessary context.

Measure:
- token savings
- cache reuse
- decision drift

## Stage 4 — Shadow Router

Router chooses models but does not execute them.

Compare router choice to actual outcome.

## Stage 5 — Cheap Workers

Enable:
- T0
- OpenRouter free
- Luna

Only on bounded/verifiable work.

## Stage 6 — T3/T4 Automatic Escalation

Enable:
- Sonnet/Terra
- Opus/Sol
- cross-provider recovery

## Stage 7 — Fable Gate

Enable Fable for:
- very high-value phase decisions
- repeated hard failures
- project-wide synthesis
- final coherence review

Opus 5 is automatic fallback.

## Stage 8 — Learned Policy

Replace most static thresholds with empirical success estimates.

## Stage 9 — Continuous Eval

Run old workflow and new router against a golden corpus.

---

# 21. Golden traces and regression tests

Before modifying the current engineering skill, save representative sessions:

```text
small bug
new feature
UX-heavy feature
architecture change
multi-file refactor
migration
hard debugging case
failed implementation recovery
final review
```

For each, capture:
- user inputs
- phase transitions
- steering prompts
- accepted decisions
- rejected decisions
- expected artifacts
- verification evidence

Regression checks:

```text
same mandatory steering gates?
same hard constraints?
same accepted UX direction?
same architectural invariants?
same or better test result?
same or fewer user corrections?
lower cost?
```

Routing may change.

Engineering semantics must not.

---

# 22. Recommended v1 policy

```yaml
orchestrator:
  default:
    model: claude-sonnet-5
    sticky: phase

  hard:
    model: claude-opus-5

  deep:
    model: claude-fable-5
    fallback:
      - claude-opus-5
      - gpt-5.6-sol
    activation_score: 70
    preferred_score: 85

workers:
  deterministic:
    tier: T0

  disposable:
    primary: openrouter-free
    fallback: gpt-5.6-luna

  normal:
    primary: gpt-5.6-luna
    fallback: claude-sonnet-5

  balanced:
    primary: gpt-5.6-terra
    fallback: claude-sonnet-5

recovery:
  claude_provider_failure:
    - gpt-5.6-terra
    - gpt-5.6-sol

  openai_provider_failure:
    - claude-sonnet-5
    - claude-opus-5

privacy:
  openrouter:
    data_collection: deny
    zdr: true
    require_parameters: true

steering:
  decision_ledger_mutation: user_gate_only
  ux_direction_changes: user_gate_only
  architecture_direction_changes: user_gate_only

budget:
  objective: expected_cost_per_accepted_artifact
  include_quota_shadow_price: true
```

---

# 23. Recommended end state

```text
                         USER
                          │
                  user-controlled intent
                          │
                          ▼
                  ┌───────────────┐
                  │ Decision      │
                  │ Ledger        │
                  └───────┬───────┘
                          │
              ┌───────────▼───────────┐
              │ Existing Phase Engine │
              └───────────┬───────────┘
                          │
                   PhaseContract
                          │
              ┌───────────▼───────────┐
              │ Context + Route Plane │
              └───────────┬───────────┘
                          │
        ┌─────────────────┼──────────────────┐
        │                 │                  │
        ▼                 ▼                  ▼
       T0             T1/T2/T3            T4/T5
 deterministic      cheap workers     Opus / Fable
        │                 │                  │
        └─────────────────┼──────────────────┘
                          ▼
                       verify
                          │
                    accepted?
                    /        \
                  yes         no
                   │           │
                   ▼           ▼
              next phase    escalate /
                            steering gate
```

The engineering skill remains the product.

The router is infrastructure.

Fable is a scarce deep-reasoning resource.

Opus is the reliable Fable fallback.

Sonnet is the normal interactive brain.

Luna/free models do the volume.

The Decision Ledger protects engineering and UX intent across every model switch.

---

# Sources

## Anthropic
- https://www.anthropic.com/claude/fable
- https://www.anthropic.com/news/claude-fable-5-mythos-5
- https://www.anthropic.com/claude/opus
- https://www.anthropic.com/news/claude-opus-5
- https://www.anthropic.com/news/claude-sonnet-5
- https://www.anthropic.com/claude/sonnet

## OpenAI
- https://developers.openai.com/api/docs/models/gpt-5.6-luna
- https://developers.openai.com/api/docs/models/gpt-5.6-terra
- https://developers.openai.com/api/docs/models/gpt-5.6-sol
- https://developers.openai.com/api/docs/guides/latest-model

## OpenRouter
- https://openrouter.ai/docs/faq
- https://openrouter.ai/docs/guides/routing/provider-selection
- https://openrouter.ai/blog/announcements/introducing-the-new-auto-router/
- https://openrouter.ai/blog/announcements/february-release-spotlight/

## Pi ecosystem
- https://github.com/elilourens/pi-agent-harness
- https://github.com/ysenko/pi-by-fable
- https://github.com/FableFatale/pi-coding-agent

---

## Caveats

- Model pricing and availability change quickly. Re-evaluate before hard-coding prices.
- Free OpenRouter inventory is dynamic. Route by capability and health rather than depending on one permanent free model.
- Bootstrap thresholds are policy defaults. Replace them with empirical estimates from your own engineering traces.
