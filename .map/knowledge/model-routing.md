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
  - https://arxiv.org/html/2409.18433
  - https://arxiv.org/html/2503.14499
  - https://openai.com/index/introducing-swe-bench-verified/
  - https://arxiv.org/html/2211.09110
  - https://pmc.ncbi.nlm.nih.gov/articles/PMC4978781/
  - https://arxiv.org/html/2406.01574
researched: 2026-08-27
confidence: cited
---

## summary

Evidence supports validating and calibrating the full item bank before assigning disjoint difficulty bands. Cumulative scores should union unique tier inventories; reused anchors do not satisfy capacity. If a band lacks valid items, fail closed, expand and recalibrate the corpus, merge bands, or publish fewer tiers. Thirty cases and five bands remain project policy.

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
- [Easy2Hard-Bench](https://arxiv.org/html/2409.18433), published 2024-09-27, supports continuous full-bank difficulty calibration before band assignment.
- [METR task-horizon study](https://arxiv.org/html/2503.14499), published 2025-03-18, combines repeated agent runs with skilled-human baselines and shows domain effects.
- [SWE-bench Verified construction](https://openai.com/index/introducing-swe-bench-verified/), published 2024-08-13, rejected 68.3 percent of reviewed items before selection.
- [HELM](https://arxiv.org/html/2211.09110), published 2022-11-16, supports standardized, disaggregated scoring. It is foundational and predates the recency target.
- [Item-response-theory linking](https://pmc.ncbi.nlm.nih.gov/articles/PMC4978781/), published 2016, supports limited common anchors. It is foundational and stale for agent evaluation.
- [MMLU-Pro](https://arxiv.org/html/2406.01574), published 2024-06-03, adds harder sources and removes trivial or noisy items when earlier coverage saturates.

## codebase

The raw inventory found 235 skill, agent, and workflow cases, but raw count did not establish disjoint difficulty capacity. The executed capacity snapshot freezes accepted T1-T3 at 33, 32, and 32 cases (`research/model-routing/frontier-corpus-capacity.json:11-17`). After those assignments, 90 non-lower executable candidates remain; only eight are honestly unique above T3, versus 30 required (`research/model-routing/frontier-corpus-capacity.json:19-27`).

The proposed T4 entry is invalid. It reaches 30 only by reusing 16 exact T2 references and contains 14 structurally unique entries (`research/model-routing/frontier-corpus-capacity.json:25-27`). Independent fixture review accepted eight above-T3 cases (`research/model-routing/frontier-corpus-capacity.json:29-37`) and rejected exact lower-tier duplicates, response-only work, routing-only work, missing inputs, prose-only work, blocked work, and cases at or below T3 (`research/model-routing/frontier-corpus-capacity.json:57-60`). T4 has a 22-case shortfall under the no-reuse floor. No T5 suite exists.

The shared loader parses case identity, input, expected result, source, holdout, support files, execution drive, tools, checkpoints, and timeout (`tools/skill-eval/src/source.rs:640-749`). `CaseDefinition` has no calibrated difficulty or discrimination (`tools/skill-eval/src/model.rs:123-165`). Sequential review therefore let T2 consume later-band cases before full-bank calibration.

The pool engine stores exact model-thinking identities, per-stage evidence, catastrophic counts, usage, confidence intervals, ranked routes, and durable child state (`tools/skill-eval/src/model.rs:195-315`). The benchmark must exist before another model call, but construction must now fail closed. Reused calibration anchors cannot count toward a later tier's floor. Validate and calibrate the complete bank first, assign disjoint bands second, and expand the executable corpus until each retained band has capacity. If five bands still cannot meet the floor, merge bands or publish fewer tiers.
