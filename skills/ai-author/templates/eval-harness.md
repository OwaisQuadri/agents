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
rubric.md. The runner emits one
JSON line per (case, tier) to stdout
(`{"id":"c1","tier":"T3","repeat_scores":[7,8,7],"median":7}`) and a mean-per-tier,
per-slice summary to stderr. Grading both slices in one pass matches what the Holdout
gating rule below already needs together ("the win holds on the holdout slice").
`--holdout` is a lighter, frontier-write-free mode for a quick recheck of the holdout
slice alone — use the plain (no-flag) form for anything feeding a real Decide.

**Execution arm, not a prose judge.** Keep the per-artifact `run.sh` as a thin wrapper
around `tools/skill-eval/run.sh`. The shared runner does not send the artifact's own text
to a judge and ask whether an agent following it WOULD pass — that grades wording, and every
tier scores identically since no tier ever actually runs anything. It dispatches a REAL
run of the artifact via `tools/tier-dispatch` (tier's own primary model, walking that
tier's own `fallbacks[]` list in `config/model-tiers.json` on a quota error), then grades
the ARTIFACT that run produced. `tier-dispatch --tier <T1..T5>` is called once per
repeat trial (`REPEATS`, default 3 — a single pass or fail is a sample of one) per case
per tier; the judge for a tier's output runs one tier up (`config/model-tiers.json`'s own
ordering), except the top tier, which grades itself since no tier exists above it.

**Every available tier is tested, every run — exhaustive, not walk-up-and-stop.** A tier
whose own fallback chain is fully exhausted on a quota error is skipped entirely for that
run; no score is guessed for it and no frontier line is written for it. `--tier T3` restricts
a run to exactly one tier — a cheap iteration escape hatch while authoring, never the
default; the restricted tier is still judged by whichever tier is one above it in the FULL
tier order, never by itself, purely because it happens to be the only tier in that run's
own sweep.

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

After grading BOTH slices in the plain (no-flag) form (candidate or incumbent, accepted
or rejected), append one line PER TIER TESTED to `evals/frontier.jsonl` and write the
full candidate text to `evals/frontier/<candidate_id>.md` — see "frontier.jsonl" below.
This runs every time, not only on acceptance: a rejected candidate's score vector is
exactly what a later Pareto-frontier selection (GEPA loop step 2) needs, and today's
harness threw it away the moment `run.sh` exited. `accepted` defaults to `false` unless
the caller sets `ACCEPTED=true` in the environment, since `run.sh` cannot know the
Decide-step verdict at grading time — nothing ships without an explicit Decide anyway.

## frontier.jsonl

The per-candidate score archive GEPA loop step 2 (Propose) reads before choosing what to
mutate from. ONE LINE PER (candidate, tier) PAIR ever tested for this artifact, including
the incumbent itself the first time `run.sh` runs after this section is adopted. Every
available tier is tested every run (exhaustive, not a walk-up-and-stop sweep — see
`tools/tier-dispatch` and `evals/run.sh`'s own comments for the execution arm that makes
this possible: a run.sh that only judges an artifact's PROSE, never runs it, produces the
same score on every tier and a tier axis on this file would carry no information):

```json
{"candidate_id":"<short hash of the candidate's full text>","tested_against":"<prompt_version of the incumbent it competed with>","tier":"<T1..T5, the tier actually dispatched>","judge_tier":"<one tier above tier, or tier itself when tier is the top tier — no tier above the top exists>","model_ran":["<every distinct model id tools/tier-dispatch actually used this tier, after any same-tier fallback walk>"],"scores_nonholdout":[7,8,6],"scores_holdout":[7],"repeat_scores_nonholdout":{"<case id>":[7,8,7]},"repeat_scores_holdout":{"<case id>":[7]},"mean_nonholdout":7.00,"accepted":false,"ts":"<local iso with offset>"}
```

- `candidate_id`: a short hash (e.g. `sha1sum | cut -c1-8`) of the candidate's exact text —
  stable identity independent of whether it shipped.
- `tier`: the tier `tools/tier-dispatch` actually dispatched this candidate at. A tier whose
  own fallback chain was fully exhausted on a quota error is SKIPPED for the run — no line
  is written for it, never a guessed score standing in for a tier that never ran.
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
  hard-failed for a real, non-quota reason (`tools/tier-dispatch` exit 1, or the judge's
  own JSON reply was unparseable) — folding an infrastructure failure into a numeric `0`
  would be indistinguishable from the candidate genuinely failing the rubric, which a code
  review of this mechanism's first version caught as a real defect. `null` entries are
  excluded from the median and from `mean_nonholdout`, never averaged in as zero.
- A `cases.jsonl` case carrying a `files` list still gets those files' content appended to
  the text the dispatched run receives as its system prompt (skill text plus each listed
  file, in order) — the same behavior the harness had before the execution arm existed,
  now applied to what gets DISPATCHED rather than what gets judged as prose.
- The candidate's full text (not a diff) lands in `evals/frontier/<candidate_id>.md`, so a
  later Propose step can load a non-incumbent frontier member and mutate from it directly.
- **Pruning**: cap `evals/frontier.jsonl` at the 20 newest entries per artifact PER TIER —
  a candidate now writes up to 5 lines (one per tier tested, fewer if a tier was skipped),
  so the cap applies within each `tier` value separately, never across tiers pooled
  together (a candidate could legitimately dominate at T3 and lose everywhere at T1; those
  are two different comparisons, not one). When over cap within a tier, drop the oldest
  *dominated* entry first — an entry whose score vector is beaten or tied everywhere by
  some other entry's vector AT THE SAME TIER, with at least one strict loss. Never prune a
  non-dominated (frontier) member for being old; dominance status is the only pruning signal.
  Delete the matching `evals/frontier/<candidate_id>.md` only once no tier's line for that
  candidate_id survives — the same candidate text is shared across its tier lines, so the
  file stays as long as any one of them does.
- **Tracked in git, not gitignored.** Unlike `votes/votes.jsonl` (gitignored because it
  carries real judge critique tied to specific transcript excerpts — personal usage
  content), `frontier.jsonl` and `frontier/` only ever hold scores against this
  artifact's own synthetic, already-tracked `cases.jsonl`, plus candidate prompt text —
  structurally just draft variants of the artifact's own already-tracked definition file.
  Neither touches live session content, so there is no sensitive-data reason to exclude it,
  and tracking it means a fresh clone can recompute the frontier.
- **Marking a candidate accepted, after Decide, without a re-grade.** `run.sh` writes
  `accepted:false` by default because it can't know the Decide verdict at grading time.
  When Decide (GEPA loop step 4) later accepts that candidate, flip its line in place
  instead of re-running `run.sh` (which would re-grade every case just to change one
  boolean):
  ```sh
  jq --arg id "<candidate_id>" -c 'if .candidate_id == $id then .accepted = true else . end' \
    evals/frontier.jsonl > /tmp/frontier.jsonl.$$ && mv /tmp/frontier.jsonl.$$ evals/frontier.jsonl
  ```
  `jq` (no `-s`) reads one JSON value per line and emits one line per input line, so
  every non-matching line passes through byte-identical, in order; only the matching
  `candidate_id`'s line gets `accepted` flipped. Matching on `candidate_id` alone is
  intentional even though a candidate now spans up to 5 lines, one per tier — acceptance
  is a property of the candidate's TEXT, not of any one tier it was tested at, so every
  tier line for an accepted candidate flips together, in one pass.
  If a
  specific artifact's eval cases start encoding something sensitive, gitignore that one
  artifact's frontier paths — never a repo-wide default change.

## Holdout gating rule

A candidate replaces the incumbent only when, on the same cases:

1. no case is graded catastrophic that wasn't before — hard reject, regardless of mean
2. mean score is higher — tie goes to the incumbent, no churn on noise
3. the win holds on the holdout slice — otherwise it's overfitting, reject
4. two candidates both pass 1–3 → the one adding fewer conditions ships (weakest wins)

## Usage evidence (no logging section to paste)

No artifact carries a logging section. `tools/gepa-due` derives usage evidence directly
from real Pi session transcripts — a `read` tool_call whose path matches this artifact's
own definition file, filtered by the time cutoff in `skills/ai-author/SKILL.md`'s "usage
evidence" section. Nothing to paste here; the eval harness above is the only thing a new
artifact needs to ship complete.
