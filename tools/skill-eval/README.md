# Skill evaluation command-line interface

`skill-eval` qualifies skills, agents, workflows, and model pools through Pi. The command-line interface (CLI) records all evidence before an owner decision.

## Execution rules

Artifact qualification uses an adaptive tier staircase. It starts at `--start-tier`, probes a cheaper tier after a pass, and probes a stronger tier after a failure. The run stops when it finds a supported boundary or needs review. The reference tier supplies comparison evidence. A distinct T5 judge grades each candidate.

Model-pool qualification runs a separate Pi-native thinking staircase for each exact model. It starts at the plan's declared thinking level. An initial pass causes a cheaper probe. An initial failure causes a stronger probe. The pool selects the lowest proven passing level. When evidence supports two finalists, the pool promotes them to full qualification.

A pool candidate must match the plan's provider, model, tier, and thinking level. The resolver does not use a fallback or replace a direct provider with a proxy. Use a direct authenticated provider when one is available. Treat each proxy route as a separate host identity.

All candidate and judge model execution starts `pi` in JavaScript Object Notation (JSON) mode. The evaluator does not start `cld`, `claude`, or `codex`. The `openai-codex` value is a Pi provider identifier, not an executable name.

A judge must use a different provider-and-model pair from its candidate. The judge tier must be stronger than a T1 through T4 candidate. A T5 pool candidate uses a distinct T5 judge because no higher tier exists.

The tracked pool plan limits catalog age to 3,600 seconds. Refresh every `catalog_observed_at` value within one hour before a pool launch. `pool-qualify` rejects future observations and observations older than the plan's limit.

A paid pool plan needs a positive provider-enforced spending limit. Configure that limit at the provider before launch. The evaluator also tracks actual cost and pauses before the next child after it reaches the plan limit.

The free T1 control is read-only and unranked. The evaluator excludes it from candidates, judges, owner decisions, and publication evidence.

## Run roots and output

The default run root is `.map/skill-eval/runs`. Use `--runs-root PATH` to isolate a run. Ordinary runs store events under `PATH/RUN/events.jsonl`. Pool snapshots use `PATH/pools/POOL/state.json`, while pool children use ordinary run directories. T1 screening snapshots use `.map/skill-eval/t1-screening/SCREEN/state.json`. T1 campaign snapshots use `.map/skill-eval/t1-screening-campaigns/CAMPAIGN/state.json`.

Their child evidence uses the ordinary run root.

Use `--run-id-file PATH` on `qualify`, `pool-qualify`, or `t1-screen-start` to capture the created identifier. The command writes the file only after it creates the run.

Text output is the default. Add `--format text` to select it. Add `--format jsonl` for JSON Lines output. Start and resume commands can emit several progress lines before the final report. Report commands emit one complete report line.

Pool and T1 text reports start with this result matrix:

```text
| Model | off | minimal | low | medium | high | xhigh | max |
| --- | --- | --- | --- | --- | --- | --- | --- |
```

Each configured base provider and model has one row. A completed pass is `P`, and a completed failure is `F`. Other cells are blank.

Pool qualification evidence overrides calibration evidence for the same cell. T1 cells use completed attempt evidence only. Stage, pause, infrastructure, spending, and evidence details follow the matrix. JSON Lines output keeps the complete structured evidence and does not replace it with matrix text.

## Model capability catalog

`model-capabilities` saves the union of the normal all-extension model list and the no-extension Remote Procedure Call (RPC) metadata. It runs only `pi --list-models`, `pi --version`, and one `get_available_models` RPC request. It does not send a prompt or call a model.

```sh
skill-eval model-capabilities --output .scratch/skill-eval/model-capabilities.json
```

The output must be a new repository-relative path. The command rejects collisions and symbolic links, then publishes one atomic JSON snapshot. List-only and RPC-only models remain in the catalog. A moving alias remains marked and is not exact qualification evidence.

## T1 screening preview

`t1-screen-preview` reads a version 1 capability snapshot and classifies every row without a model call or file write. It keeps fixed zero-cost text models with exact evidence and valid Pi thinking levels. It reports every exclusion reason, preserves supported-level holes, labels fixed preview identities, and projects the exact five-case call count. The frozen snapshot projects 485 candidate calls and 485 judge calls.

```sh
skill-eval t1-screen-preview --capabilities research/model-routing/pi-model-capabilities.json --format text
```

Use `--format json` for one deterministic JSON report. The report projects candidate money at zero. The owner-approved judge-spending cap remains separate from the 485-call judge projection. Execution stays blocked until the owner approves this cap.

## T1 screening commands

### `t1-screen-campaign-create`

`t1-screen-campaign-create` imports the exact stored T1 run set into one campaign. It hashes each raw state file and records its canonical path, creation time, status, judge spend, and zero candidate cost. Legacy environment states remain audit entries and cannot resume. The command makes no model call.

```sh
skill-eval t1-screen-campaign-create --campaign t1-owner-budget --judge-cap-millionths 20000000 --reason "Owner approved one total T1 judge budget" --run RUN-1 --run RUN-2 --format json
```

The campaign starts with an approved total of 20,000,000 millionths. A restart or run-local cap cannot change that total. Only `t1-screen-campaign-extend-cap` can raise it through an explicit owner approval.

### `t1-screen-campaign-extend-cap`

`t1-screen-campaign-extend-cap` appends a larger approved aggregate total to a paused or exhausted campaign that has no active run. The record keeps the approval timestamp, exact previous total, new total, and owner reason. The command validates the complete chain from the initial 20,000,000 total, reopens the campaign, and saves the state before output. It preserves all run entries and aggregate spend. It makes no candidate, judge, model, or Pi call.

```sh
skill-eval t1-screen-campaign-extend-cap --campaign t1-owner-budget --judge-cap-millionths 66038087 --reason "Owner approved the aggregate campaign total" --format json
```

Each extension must name a total above the current campaign total and include a nonblank reason. Text output shows the exact total, spent amount, remaining amount, and extension count. JSON output returns the saved campaign state.

### `t1-screen-campaign-retire-run`

`t1-screen-campaign-retire-run` retires the exact active paused run without a candidate, judge, model, provider, or Pi call. It requires a paused campaign, a matching resumable run whose stored state is also paused, and a nonblank owner reason. The command verifies the unchanged raw run bytes and audit fields, then saves the campaign before output.

```sh
skill-eval t1-screen-campaign-retire-run --campaign t1-owner-budget --run t1-screen-run-1 --reason "Owner retired the paused run" --format json
```

Retirement preserves the run state file, evidence, status, costs, campaign spend, approved total, and prior history. It appends one timestamped retirement, makes only that run non-resumable with the exact owner reason, clears the active run, and reopens the campaign. Reconciliation keeps retired entries inactive while it still detects later run-file drift. A later campaign cap extension or retirement must have a timestamp later than all prior authority records. Text output names the run and shows the exact total, spent amount, and remaining amount. JSON output returns the saved campaign state.

### `t1-screen-fail-route`

`t1-screen-fail-route` records one exact route that cannot continue after an infrastructure pause. It requires the active parent run, its one paused child, and a nonblank owner reason. It verifies the saved child pause and exact model identity. It then saves the parent and reopens the same campaign run before it writes output.

```sh
skill-eval t1-screen-fail-route --run t1-screen-run-1 --child t1-child-0042 --reason "Owner accepted this exact route failure" --format json
```

The record keeps the local timestamp, child identifier, exact model identity, and owner reason. It also keeps the lowercase Secure Hash Algorithm 256-bit digest, or SHA-256 digest, of the saved pause message.

The failed child becomes `failed`. Only its later thinking siblings become `skipped`. Earlier completed attempts, usage, caps, event files, campaign spend, and other model states do not change. The model outcome is `infrastructure_failed`, not `exhausted`. A later resume starts the next base model and does not retry the failed child.

Text output names the failed route and shows the campaign total, spent amount, and remaining amount. JSON output returns the saved report. The command makes no provider, candidate, judge, model, or Pi call.

### `t1-screen-start`

`t1-screen-start` rebuilds the preview from the named tracked capability snapshot. It freezes the campaign identifier, exact five-case exam, external configured judge, candidate environment, local caps, and all child identifiers. It rejects changed snapshot bytes, output collisions, missing eligible models, any other case count, and a provider cap above the owner cap. It also rejects an active, paused, exhausted, awaiting-owner, or underfunded campaign. The parent snapshot and active campaign entry exist before the first candidate or judge call.

```sh
skill-eval t1-screen-start --campaign t1-owner-budget --capabilities research/model-routing/pi-model-capabilities.json --exam tools/skill-eval/tests/fixtures/model-calibration --judge-cap-millionths 20000000 --provider-cap-millionths 20000000 --run-id-file .scratch/skill-eval/t1-screen-id --format text
```

Candidate execution uses the deployed Pi runner with its discovered extensions and complete tool inventory. The blind judge stays locked to its evidence packet and configured judge tier. The command starts no provider-specific executable.

### `t1-screen-resume`

`t1-screen-resume` continues the first paused or interrupted child from its frozen identifier. It requires the state's named environment manifest and matching active campaign entry. It reconciles campaign spend before it projects another judge call. It reloads no replacement classification or route.

```sh
skill-eval t1-screen-resume --run t1-screen-run-1 --format json
```

Resume cannot bypass a judge-cap pause unless the owner appends a larger cap with `t1-screen-extend-cap`.

### `t1-screen-extend-cap`

`t1-screen-extend-cap` appends new owner and provider totals to a paused judge-cap run. It keeps the base configuration, child identifiers, completed candidate work, completed judge work, evidence, usage, and spend unchanged. It writes the parent snapshot before it prints the updated report. It does not resume the run or call a candidate, judge, or model runtime.

```sh
skill-eval t1-screen-extend-cap --run t1-screen-run-1 --judge-cap-millionths 20000000 --provider-cap-millionths 20000000 --reason "Owner approved the remaining judge work" --format json
```

Each extension must name totals above the current effective totals. The provider total cannot exceed the owner total. The command accepts only a paused judge-cap run. Run `t1-screen-resume` separately to continue the same active child.

### `t1-screen-report`

`t1-screen-report` reads the parent snapshot and child event logs without creating a model runtime or writing a file.

```sh
skill-eval t1-screen-report --run t1-screen-run-1 --format text
```

The report retains the campaign identifier, approved total, aggregate spend, remaining amount, ordered run entries, and active run. It also retains inventory classifications, exclusion reasons, call projections, local caps, cap extension history, spend, and active or paused state. It retains ordered thinking attempts, five per-case verdicts and checks, separate usage, latency, failures, and terminal outcomes. JSON format writes one strict JSON object.

A terminal report ranks passing selected routes by candidate cost, candidate latency, candidate failure rate, then exact provider, model, and thinking identity. Judge cost and latency do not affect rank. Candidate cost must remain zero.

The first three routes are recommendations. The report places all remaining passing routes in ordered alternates. Fewer than three passing routes produce no recommendation and an explicit shortage. Pending, running, paused, excluded, and exhausted routes do not rank.

Every recommendation requires owner approval. These commands do not record a decision, evaluate a publication gate, write a tier, edit routing configuration, or accept a result.

## Artifact qualification commands

### `qualify`

Use `--skill PATH` or `--artifact PATH` once per artifact. Use `--all-skills` instead of explicit paths. `--dry-run` records discovery without model calls.

```sh
skill-eval qualify --skill skills/create-pr --dry-run --start-tier T2 --reference-tier T4 --run-id-file .scratch/skill-eval/run-id --runs-root .scratch/skill-eval/runs --format jsonl
```

The defaults are three trials, score 8, margin 1.0, confidence 0.95, start tier T2, reference tier T4, and judge tier T5. Override them with `--trials`, `--minimum-score`, `--noninferiority-margin`, and `--confidence`.

A changed-artifact run needs all four change arguments. The own-evaluation evidence must identify the candidate revision.

```sh
skill-eval qualify --artifact skills/create-pr --change-artifact skills/create-pr --incumbent-revision incumbent-v1 --candidate-revision candidate-v2 --own-eval skills/create-pr/evals/result.json --runs-root .scratch/skill-eval/runs --format jsonl
```

### `report`

`report` reads the reduced qualification state and does not change the run.

```sh
skill-eval report --run run-1 --runs-root .scratch/skill-eval/runs --format text
```

### `inspect`

`inspect` reads one completed trial. The artifact argument is the artifact name, not its path.

```sh
skill-eval inspect --run run-1 --artifact create-pr --tier T2 --case c1 --trial 2 --runs-root .scratch/skill-eval/runs --format jsonl
```

`--skill` aliases `--artifact`, and `--attempt` aliases `--trial`.

### `resume`

`resume` continues a paused qualification from its persisted checkpoint.

```sh
skill-eval resume --run run-1 --runs-root .scratch/skill-eval/runs --format jsonl
```

A quota pause keeps completed work and any candidate checkpoint. Run `resume` after quota returns. The evaluator does not repeat completed candidate work.

### `decide`

The owner must record either acceptance or rejection. The CLI never infers this choice.

Acceptance needs one exact assignment for every required destination.

```sh
skill-eval decide --run run-1 --artifact create-pr --accept --assign skill_minimum=T2 --assign skill_target=T2 --runs-root .scratch/skill-eval/runs --format jsonl
```

Rejection needs a reason and accepts no assignment.

```sh
skill-eval decide --run run-1 --artifact create-pr --reject --reason "owner keeps the incumbent" --runs-root .scratch/skill-eval/runs --format jsonl
```

The accepted tier must equal the supported boundary tier. Destination names depend on the artifact kind:

- A skill requires `skill_minimum`. It also requires `skill_target` when the skill declares that destination.
- An agent requires `agent`.
- A workflow requires `workflow_orchestrator` and every `workflow_node:NAME` destination.

The parser also accepts the shorter `minimum`, `target`, and `orchestrator` destination names.

### `apply`

`apply` evaluates the publication gate and writes only approved tier destinations.

```sh
skill-eval apply --run run-1 --artifact create-pr --runs-root .scratch/skill-eval/runs --format jsonl
```

A skill write updates its minimum and optional target metadata. An agent write updates its tracked agent tier. A workflow write updates its orchestrator tier and every named node tier.

### `audit-briefs`

`audit-briefs` runs blind incumbent cases and writes failure briefs to a new output tree.

```sh
skill-eval audit-briefs --artifact skills/create-pr --out .scratch/skill-eval/audits --runs-root .scratch/skill-eval/runs --format jsonl
```

Use repeatable `--skill` or `--artifact` arguments, or use `--all-skills`. The command rejects an existing candidate mutation and does not expose holdouts, prior votes, model identity, or candidate text.

### `judge`

`judge` sends one prompt to the configured external judge. Use either inline text or a file.

```sh
skill-eval judge --prompt "grade this" --timeout 30 --runs-root .scratch/skill-eval/runs --format jsonl
```

```sh
skill-eval judge --prompt-file .scratch/skill-eval/prompt.txt --timeout 30 --runs-root .scratch/skill-eval/runs --format text
```

Use `--prompt-file -` to read standard input. Text format prints the response. JSON Lines format includes the judge identity, response, and usage.

## Model-pool commands

### `pool-qualify`

A pool plan is a repository-relative JSON file. Each selected tier must contain exactly three exact entrants. Repeat `--artifact` and `--tiers` as needed. Omitting `--tiers` selects T1 through T5.

```sh
skill-eval pool-qualify --plan .map/AGNT-0032/model-pool-plan.json --artifact tools/skill-eval/tests/fixtures/model-calibration --tiers T2 --dry-run --run-id-file .scratch/skill-eval/pool-id --runs-root .scratch/skill-eval/runs --format jsonl
```

A dry run freezes artifacts and preallocates child identifiers without model calls. A live run performs one child at a time and saves each parent state before it emits progress.

### `pool-report`

`pool-report` reads the pool snapshot. It reports exact hosts, thinking attempts, judge identities, failures, usage, spending, promotions, rankings, and owner state.

```sh
skill-eval pool-report --run pool-1 --runs-root .scratch/skill-eval/runs --format text
```

### `pool-resume`

`pool-resume` continues one saved child and preserves stable child identifiers.

```sh
skill-eval pool-resume --run pool-1 --runs-root .scratch/skill-eval/runs --format jsonl
```

Use it after a quota pause. `pool-resume` preserves a spending-limit pause, so the owner cannot bypass the cap. Identity, artifact, harness, judge, or plan drift stops the resume.

### `pool-replacement`

`pool-replacement` runs full qualification for one owner-approved, passing calibration entrant that the no-backfill rule skipped. It requires an awaiting-decision parent with a failed finalist. It does not change the parent pool or erase the failed evidence.

```sh
skill-eval pool-replacement --run pool-1 --entrant-index 2 --run-id-file .scratch/skill-eval/replacement-id --runs-root .scratch/skill-eval/runs --format jsonl
```

The command freezes the parent artifact, exact selected thinking level, full qualification repeat count, score floor, judge tier, and environment identity into a separate run.

## Mandatory authoring publication gate

`ai-author` (artificial intelligence author) must use this gate for every new artifact and every accepted GEPA (Genetic-Pareto prompt evolution) mutation. The candidate's own evaluation must pass at the same revision. The qualification staircase must produce supported current evidence. The owner must accept the result and assign every type-owned destination at the boundary tier.

Only `apply` can make the gate ready and write the approved tier destinations. A failed, paused, stale, undecided, rejected, or review-required result keeps the incumbent artifact and tiers unchanged. The authoring flow must not publish the candidate before the gate is ready.

## Paid Pi integration test

Cargo ignores the real Pi test by default. It uses an isolated home and requires an OpenRouter key. It can spend provider credit.

```sh
SKILL_EVAL_H4_REAL_PI=1 OPENROUTER_API_KEY="$OPENROUTER_API_KEY" cargo test --manifest-path tools/skill-eval/Cargo.toml --test pi_integration -- --ignored ordinary_skill_trial
```

The test skips when either opt-in value is absent or when its exact candidate and judge are unavailable.
