# Eval harness template

Every artifact ai-author authors ships an `evals/` dir with the three files below. No
artifact carries its own logging section anymore — usage evidence is derived from real
Pi session transcripts by `tools/gepa-due`, never self-reported. Copy, fill, keep lean.

## cases.jsonl

One case per line:

```json
{"id":"c1","input":"<situation the artifact should handle>","expect":"<checkable success criterion>","holdout":false,"source":"seed"}
```

- ≥5 cases before a draft goes live; grow them from real transcript failures (found via
  `gepa-due`'s own scan at tuning time) and votes.
- Give every case a unique `id` and an explicit Boolean `holdout` value. The runner rejects
  a missing slice, a missing field, and a duplicate identifier before any dispatch.
- Mark ~20% (minimum 1) `"holdout": true` — never shown to the mutation-proposer.
- `source` is provenance: `seed` (authored), `log` (from a real use), `vote` (from a
  judge complaint).

## rubric.md

The judge's grading contract, single-sourced here (the blind post-use judge and the eval
judge use the same rubric):

```
Score 0-10. Grade harshly: expect met exactly, or say what's missing.
- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable): <this artifact's irreversible or false-pass failures>
```

List the artifact's catastrophic failures explicitly (false pass, wrong autonomous
action, hallucinated names). A catastrophic case can't be traded against a better mean.

## run.sh

Use this wrapper:

```zsh
#!/bin/zsh
set -euo pipefail
here=${0:A:h}
repo=$(git -C "$here" rev-parse --show-toplevel)
exec "$repo/tools/skill-eval/run.sh" --eval-dir "$here" "$@"
```

Convention: `./run.sh [candidate-file]` delegates to `tools/skill-eval/run.sh`. It grades
BOTH slices (non-holdout, then holdout) against the current artifact or candidate with
rubric.md. A full paired run splits work into `(arm, tier, slice, case, repeat)` units.
The runner emits stable JSON lines per paired case to stdout with an additive `arm` field
(`{"arm":"candidate","id":"c1","tier":"T3","repeat_scores":[7,8,7],"median":7}`). Each
case includes aggregate dispatch time and per-repeat generation and judge timing, requested tiers,
final models, and fallback attempts. It writes durable-unit progress and per-arm tier summaries to
stderr. It serializes `output-check.sh`.
The one final coordinator owns the selection, frontier write, and conditional acceptance after
all units complete.

Grading both slices in one pass supplies the conditional acceptance rule below. `--holdout`
is a lighter, frontier-write-free mode for a quick holdout recheck. Use
`--accept-if-winning <candidate>` for a real Decide. A plain candidate run is a dry comparison.

**Execution arm, not a prose judge.** Keep the per-artifact `run.sh` as a thin wrapper
around `tools/skill-eval/run.sh`. The shared runner does not send the artifact's own text
to a judge and ask whether an agent following it WOULD pass — that grades wording, and every
tier scores identically since no tier ever actually runs anything. It dispatches a REAL
run of the artifact via `tools/tier-dispatch`, starting with the tier's primary model.
When quota limits or availability stop a model, the dispatcher walks that tier's own
`fallbacks[]` list in `config/model-tiers.json`. The harness then grades
the ARTIFACT that run produced. `tier-dispatch --tier <T1..T5>` is called once per
repeat trial (`REPEATS`, default 3 — a single pass or fail is a sample of one) per case
per tier; the judge for a tier's output runs one tier up (`config/model-tiers.json`'s own
ordering), except the top tier, which grades itself since no tier exists above it.

**The runner attempts every configured tier on every full run.** It ignores the artifact's
`metadata.minimum-tier`. That floor is a hypothesis the scores can change, not an evaluation
filter. When a model is unavailable, `tools/tier-dispatch` walks the rest of that tier's
fallback chain. This includes quota limits and installed-client support errors. If the whole
chain is unavailable, the frontier keeps that tier's line with `null` scores. The visible gap
can never look like a tested win. `--tier T3` is a narrow authoring check and writes no
frontier data. The selected tier still uses the next configured tier as its judge.

**A graded dispatch keeps its tools but never sees the live repo.** Every Pi process
disables extension discovery and loads `pi-anthropic-auth` as the bare minimum extension.
A harness adds another extension only when a case requires it. `tools/tier-dispatch`
runs every dispatch — artifact and judge alike — with tools ON, inside a fresh throwaway
sandbox directory that is discarded after the attempt. Tools stay on because the harness
measures what a tier can actually DO; a dispatch stripped of tools is a different, easier
task whose score stops predicting real capability. The sandbox is confirmed necessary,
not theoretical: an early version that ran dispatches in the repo's own working directory
let a real dispatch of ai-author's own case `a1` ("every code comment should follow the
whitelist... apply it in every session") actually EDIT this repository's own tracked
`CLAUDE.md`, because the case's EXPECT correctly names CLAUDE.md as the right destination
and the model, holding write access to the live repo with no signal this was a graded
exercise, made the edit rather than stating the verdict. Reverted; the sandbox is the
fix. A harness whose cases need the dispatch to act on specific fixture files should
pre-seed them and grade the aftermath the way `agents/spec-tester/evals/run.sh` already
does (scratch fixture dir, checksums before and after) — never point a graded dispatch
at a working tree anyone cares about.

An artifact can add an executable `evals/preflight.sh` for a deterministic check before
model dispatch. It can add `evals/output-check.sh` for a deterministic check of each
actual output. The shared runner caps a failed output check at 4. Both files must be
executable, or the runner stops with an error. The runner gives `preflight.sh` the selected
candidate as its first argument. It exports the absolute `CASES_FILE` path. These checks add
evidence and never replace tier execution.

A full candidate invocation evaluates the live incumbent and candidate as one paired comparison.
It prepares immutable arm-qualified prompts, runs bounded units, and saves completed units under
`evals/.skill-eval-state/<run-key>/`. The same command resumes an exact incomplete run,
including its completed timing records. A completed
`null` score remains complete on automatic resume. Use `--restart` to retry unavailable providers
after they recover. `--restart` reruns the current exact key. Use `--jobs N` or `SKILL_EVAL_JOBS`
to set one through 16 workers. The runner emits incumbent records before candidate records. Each arm
orders records by configured tier, non-holdout cases, then holdout cases. The runner uses
`evals/.skill-eval-state/run.lock` as one stable advisory lock per artifact. The lock permits one
paired coordinator while its workers remain concurrent. Only the final coordinator aggregates complete
units. After the final coordinator writes the frontier and live definition, a state cleanup failure is
a warning. The completed run keeps its success exit. It removes only `metadata.minimum-tier` from
model prompts, candidate identity, and the frontier snapshot; preflight receives the submitted candidate unchanged.
It appends one row per configured tier even when a tier is unavailable. A dry run records
its paired evidence only. `--accept-if-winning` applies a winner conditionally: accepted
is exit 0, a valid rejection is exit 1, and incomplete or execution failure is exit 2.

## frontier.jsonl

The per-candidate score archive GEPA loop step 2 (Propose) reads before choosing what to
mutate from. ONE LINE PER (candidate, tier) PAIR ever tested for this artifact, including
the incumbent itself the first time `run.sh` uses this format. The runner
attempts every configured tier and ignores `metadata.minimum-tier`. A runner that only judges
an artifact's prose would produce the same score on every tier. This execution arm gives the
tier axis real information:

```json
{"candidate_id":"<short hash of the normalized candidate text>","tested_against":"<prompt_version of the incumbent it competed with>","tier":"<T1..T5, the tier actually dispatched>","judge_tier":"<one tier above tier, or tier itself when tier is the top tier — no tier above the top exists>","model_ran":["<every distinct model id tools/tier-dispatch actually used this tier, after any same-tier fallback walk>"],"scores_nonholdout":[7,8,6],"scores_holdout":[7],"repeat_scores_nonholdout":{"<case id>":[7,8,7]},"repeat_scores_holdout":{"<case id>":[7]},"case_timings_nonholdout":{"<case id>":{"total_ms":1200,"repeats":[{"generation_ms":500,"judge_ms":700,"requested_tier":"T3","final_model":"<model id>","attempts":[],"judge_requested_tier":"T4","judge_final_model":"<judge model id>","judge_attempts":[]}]}},"case_timings_holdout":{},"incumbent_case_timings_nonholdout":{},"incumbent_case_timings_holdout":{},"mean_nonholdout":7.00,"accepted":false,"ts":"<local iso with offset>"}
```

- `candidate_id`: a short hash of the candidate text after removing only
  `metadata.minimum-tier`, so a floor-only update has one identity. Existing rows retain
  their earlier raw-text identifiers. The first retest writes the normalized identifier.
- `comparison_id`, `incumbent_id`, `incumbent_model_ran`, incumbent scores, and incumbent
  repeats identify the paired live incumbent evidence on every candidate tier row. Legacy
  rows remain readable but never authorize acceptance.
- `case_timings_nonholdout` / `case_timings_holdout` and their incumbent counterparts keep
  aggregate case dispatch time and per-repeat generation and judge measurements. Each repeat
  names the requested tier, final model, and every timed fallback attempt.
- `tier`: the configured tier `tools/tier-dispatch` attempted for this candidate. The
  artifact's declared minimum tier never filters this list. If the whole model chain is
  unavailable, its line keeps `null` scores instead of a guessed score or missing record.
- `judge_tier`: which tier graded this tier's output. One tier up by convention, so no model
  grades itself; the top tier is a named, accepted exception — it grades itself, since no
  tier above it exists.
- `model_ran`: the exact model(s) that produced these scores, distinct from `tier` itself
  whenever a same-tier fallback fired (`tier` names what was REQUESTED; `model_ran` names
  what actually ran — the two are allowed to differ and both are always recorded, never
  only the requested one, per this repo's own rule against reporting success on anything
  other than the object actually acted on).
- `scores_nonholdout` / `scores_holdout`: the per-case score arrays `run.sh` already prints
  to stdout, in case order — the MEDIAN of that case's repeat trials at this tier (a single
  pass or fail is a sample of one, so a candidate/tier pair is graded on repeats, never one
  run), kept instead of discarded.
- `repeat_scores_nonholdout` / `repeat_scores_holdout`: the raw, un-averaged repeat scores
  behind each median above, keyed by case id — kept for auditability; the median in
  `scores_nonholdout`/`scores_holdout` is derived from these, never the other way around.
  A repeat entry is `null`, never `0`, when that specific repeat's dispatch or judge call
  could not produce a grade because the model chain exhausted, the dispatcher failed, or
  the judge returned invalid JSON. A numeric `0` would look like a real rubric failure.
  `null` entries do not enter the median or `mean_nonholdout`. A `null` repeat inside
  the selected suffix blocks acceptance. A gap does not exclude a tier when its case median
  survives. The score ranking selects the suffix before the completeness gate. A tier below
  that selected floor keeps its gap without blocking acceptance.
- A `cases.jsonl` case carrying a `files` list still gets those files' content appended to
  the text the dispatched run receives as its system prompt (skill text plus each listed
  file, in order) — the same behavior the harness had before the execution arm existed,
  now applied to what gets DISPATCHED rather than what gets judged as prose.
- The normalized candidate text (not a diff) lands in `evals/frontier/<candidate_id>.md`, so
  a later Propose step can load a non-incumbent frontier member and mutate from it directly.
- **Pruning**: cap `evals/frontier.jsonl` at the 20 newest unaccepted entries per artifact
  per tier. Accepted entries stay outside this cap. A candidate writes one line for every
  configured tier, so the cap applies within each `tier` value separately. A comparison with
  any accepted selected-suffix row stays outside the cap as one complete group, including its
  excluded lower rows. Keep the newly appended line. When over cap, drop the oldest prior
  incomplete entry first. Then drop the
  oldest prior *dominated* entry: an entry whose score vector another entry at the same tier
  beats or ties everywhere, with at least one strict win. If needed, drop the oldest prior
  unaccepted entry to enforce the cap. Delete the matching
  `evals/frontier/<candidate_id>.md` only once no tier's line for that candidate survives.
  The same candidate text is shared across its tier lines.
- **Tracked in git, not gitignored.** Unlike `votes/votes.jsonl` (gitignored because it
  carries real judge critique tied to specific transcript excerpts — personal usage
  content), `frontier.jsonl` and `frontier/` only ever hold scores against this
  artifact's own synthetic, already-tracked `cases.jsonl`, plus candidate prompt text —
  structurally just draft variants of the artifact's own already-tracked definition file.
  Neither touches live session content, so there is no sensitive-data reason to exclude it,
  and tracking it means a fresh clone can recompute the frontier.
- **Conditional acceptance.** Run `./run.sh --accept-if-winning <candidate>`. The runner
  ranks each scorable suffix ending at the highest configured tier by exact candidate
  non-holdout average, with equal tier weights; an exact tie picks the widest suffix. It then
  gates only that suffix: numeric scores and repeats for both arms and slices, no new score-zero
  case, and strict aggregate wins on non-holdout and holdout. It does not retry another suffix.
  Under the frontier lock it confirms the incumbent is unchanged and writes the submitted
  winner. It updates only a declared `metadata.minimum-tier` and preserves a deliberate absence. It
  restores the live definition if frontier persistence fails. Only rows in the selected suffix receive `accepted:true`. Lower excluded
  rows remain unaccepted and keep `decision:"excluded_below_floor"`. Pruning retains complete
  paired comparison groups.

## Holdout gating rule

The runner always executes every configured tier. It scores each tier independently, then every
contiguous suffix ending at the highest tier from raw per-case medians using exact rational
comparisons. It selects the candidate's highest non-holdout average, choosing the widest suffix
on an exact tie. Only selected-tier evidence gates acceptance; excluded lower tiers do not block.
A selected suffix needs complete numeric paired evidence, no new rubric score-zero case, and
strict candidate aggregate wins on both slices. There is no fallback suffix.

## Usage evidence (no logging section to paste)

No artifact carries a logging section. `tools/gepa-due` derives usage evidence directly
from real Pi session transcripts — a `read` tool_call whose path matches this artifact's
own definition file, filtered by the time cutoff in `skills/ai-author/SKILL.md`'s "usage
evidence" section. Nothing to paste here; the eval harness above is the only thing a new
artifact needs to ship complete.
