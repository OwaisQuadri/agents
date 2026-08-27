---
type: research
topics: [model routing, agent evaluation, benchmark design]
source_tickets: [AGNT-0032]
sources:
  - https://arxiv.org/abs/2602.12670
  - https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents
  - https://arxiv.org/abs/2602.07150
  - https://www.anthropic.com/engineering/infrastructure-noise
  - https://aclanthology.org/2026.acl-long.2045/
  - https://iclr.cc/virtual/2026/oral/10009494
  - https://arxiv.org/abs/2605.23904
  - https://arxiv.org/html/2601.21557v1
  - https://epoch.ai/frontiermath
  - https://arxiv.org/abs/2406.19314
  - https://crfm.stanford.edu/2025/06/04/reliable-and-efficient-evaluation.html
  - https://proceedings.mlr.press/v235/maia-polo24a.html
  - https://arxiv.org/html/2403.04132
  - https://arxiv.org/html/2411.12990
  - https://openai.com/index/why-we-no-longer-evaluate-swe-bench-verified/
researched: 2026-08-27
confidence: cited
---

## summary

Model routing needs one cumulative, nested benchmark rather than five disconnected pools. Research supports locked confirmation cases, calibrated difficulty, rolling case refresh, paired challenger comparisons, and separate capability tags. The repo has 235 cases, but declared tiers cover only T2-T4; T1, T5, and 78 untiered cases block a credible 30-case ladder.

## links

- [SkillsBench](https://arxiv.org/abs/2602.12670), updated 2026-06-14, runs matched skill and no-skill conditions with deterministic verifiers.
- [Anthropic agent-evaluation guidance](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents), published 2026-01-09, recommends repeated trials, outcome checks, and resource metrics.
- [On Randomness in Agentic Evals](https://arxiv.org/abs/2602.07150), published 2026-02-06, reports material single-run score variation.
- [Anthropic infrastructure-noise study](https://www.anthropic.com/engineering/infrastructure-noise), published 2026-02-05, shows execution resources can change scores.
- [MTRouter](https://aclanthology.org/2026.acl-long.2045/), published in 2026, learns routing from execution history.
- [GEPA](https://iclr.cc/virtual/2026/oral/10009494), published in 2026, tests prompt mutations against execution traces.
- [SkillOpt](https://arxiv.org/abs/2605.23904), updated 2026-05-25, accepts only strict held-out gains.
- [Meta Context Engineering](https://arxiv.org/html/2601.21557v1), published 2026-01-27, co-evolves context construction and produced context.
- [FrontierMath](https://epoch.ai/frontiermath), accessed 2026-08-27, uses unpublished expert-reviewed problems in explicit difficulty tiers.
- [LiveBench](https://arxiv.org/abs/2406.19314), revised 2025-04-18, adds recent objective questions and harder variants.
- [Stanford CRFM adaptive evaluation](https://crfm.stanford.edu/2025/06/04/reliable-and-efficient-evaluation.html), published 2025-06-04, uses calibrated item difficulty and adaptive selection.
- [tinyBenchmarks](https://proceedings.mlr.press/v235/maia-polo24a.html), published 2024-07-08, reuses item-level results for compact comparisons.
- [Chatbot Arena](https://arxiv.org/html/2403.04132), published 2024-03-07, uses blind paired prompts, ranking, and uncertainty.
- [BetterBench](https://arxiv.org/html/2411.12990), published 2024-11-20, recommends uncertainty, provenance, contamination checks, and explicit score scope.
- [OpenAI SWE-bench Verified retirement](https://openai.com/index/why-we-no-longer-evaluate-swe-bench-verified/), published 2026-08-26, documents retirement after contamination and scoring problems.

## codebase

The executed inventory at `.map/AGNT-0032/tier-corpus-inventory.tsv:1-30` finds 235 existing skill, agent, and workflow cases. Declared assignments cover 11 T2 cases, 81 T3 cases, and 65 T4 cases. No case source declares T1 or T5, and 78 cases have no tier. Fifty-five cases are holdouts and 192 carry explicit execution data.

The shared loader parses case identity, input, expected result, source, holdout, support files, execution drive, tools, checkpoints, and timeout (`tools/skill-eval/src/source.rs:640-749`). `CaseDefinition` has no tier, stratum, weight, critical flag, exposure state, or calibrated difficulty (`tools/skill-eval/src/model.rs:123-165`). A tier-suite manifest can reference existing artifact and case identities, but it must freeze source revisions and own those routing fields.

The pool engine stores exact model-thinking identities, thinking levels, per-stage evidence, catastrophic counts, usage, confidence intervals, ranked routes, and durable child state (`tools/skill-eval/src/model.rs:195-315`). The statistics layer validates complete repeated trial sets and confidence-adjusted acceptance (`tools/skill-eval/src/statistics.rs:1060-1224`). The store supports validated snapshot replacement (`tools/skill-eval/src/pool_store.rs:121-240`).

Current routing assigns evaluated agents only to T2-T4 (`config/model-tiers.json:42-54`). The pool shapes rank one tier at a time, and policy lacks the approved 1/3/5 schedule, weighted groups, critical-case identities, 85-percent aggregate, and 80-percent lower bound (`tools/skill-eval/src/model.rs:195-270`). The T1 command requires exactly five non-holdout cases (`tools/skill-eval/src/cli.rs:3187-3191`). Reports render one boolean matrix per tier (`tools/skill-eval/src/cli.rs:1278-1479`).

The benchmark must exist before another model call. It needs reviewed nested tier manifests, immutable provenance, a locked confirmation subset, cumulative progression state, baseline differences, capability tags, and one frontier-wide matrix. OpenRouter challengers remain later tickets, but their evidence fingerprints must be compatible with this first-party baseline.
