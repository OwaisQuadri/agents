---
name: lateral-syntactic-drift
description: Use when the user wants novel ideas, alternative framings, or a deliberate escape from an obvious solution. Reframes the problem through distinct lenses, then ranks concrete ideas. Skip when the user needs execution, factual research, or a conventional plan.
---

# Lateral Syntactic Drift

JOB: Generate grounded novel ideas by changing the language and frame of one problem.
IN: A problem statement, constraints, and any known non-negotiables.
OUT: Five reframes, ranked ideas, assumptions, and one recommended experiment.

1. Name the active domain before reframing. A container, such as a footer slot, is not the domain. Its current data, goal, and metaphors are the domain.
2. Restate the problem through five lenses. Change actor, object, constraint, timescale, incentive, or metaphor in each lens.
3. When the user asks for unrelated ideas, reject candidates that reuse the active domain's data, goal, or metaphor. Generate ideas from at least three other domains. Treat user examples as boundaries or hints, not ideas to repeat, unless the user asks to use one.
4. Generate at least one concrete idea from each lens. Keep the original problem visible and name every assumption.
5. Rank the ideas by novelty and feasibility. State one low-cost experiment for the best idea.
6. Stop before implementation. Ask the user to select an idea when the choice changes the next action.

## output

```text
Problem:
Non-negotiables:
Active domain:
Escaped domains:
Reframes:
1. <lens>: <restatement>
Ideas:
1. <idea> | novelty: high|medium|low | feasibility: high|medium|low | assumption: <fact>
Recommended experiment:
```

## evals

Run `evals/run.sh` for non-holdout cases and `evals/run.sh --holdout` for the holdout case.

## logging

At the end of a use, append one bounded JSON line to
`<repo-root>/skills/lateral-syntactic-drift/logs/usage.jsonl`, where `<repo-root>` is
the output of `git rev-parse --show-toplevel` — never a path relative to the caller's
own working directory:

```json
{"ts":"<local ISO timestamp with offset>","artifact":"lateral-syntactic-drift","trigger":"<what fired it>","excerpt":"<problem and selected idea>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```
