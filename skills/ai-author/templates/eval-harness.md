# Eval harness + logging template

Every artifact ai-author authors ships BOTH pieces of this template: an `evals/` dir
with the three files below, AND the `## logging` section pasted at the end of its
definition (SKILL.md, `<agent>.md`, or the workflow's SKILL.md). Copy, fill, keep lean.

## cases.jsonl

One case per line:

```json
{"id":"c1","input":"<situation the artifact should handle>","expect":"<checkable success criterion>","holdout":false,"source":"seed"}
```

- ≥5 cases before a draft goes live; grow them from `logs/usage.jsonl` failures and votes.
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

Convention: `./run.sh [candidate-file]` — grades BOTH slices (non-holdout, then holdout)
against the current artifact (or the candidate, if given) with rubric.md, emitting one
JSON line per case to stdout (`{"id":"c1","score":7,"failure_mode":"<tag-or-null>"}`)
and a mean-per-slice summary to stderr. Grading both slices in one pass matches what the
Holdout gating rule below already needs together ("the win holds on the holdout slice").
`--holdout` is a lighter, frontier-write-free mode for a quick recheck of the holdout
slice alone — use the plain (no-flag) form for anything feeding a real Decide.

After grading BOTH slices in the plain (no-flag) form (candidate or incumbent, accepted
or rejected), append one line to `evals/frontier.jsonl` and write the full candidate
text to `evals/frontier/<candidate_id>.md` — see "frontier.jsonl" below. This runs every
time, not only on acceptance: a rejected candidate's score vector is exactly what a later
Pareto-frontier selection (GEPA loop step 2) needs, and today's harness threw it away the
moment `run.sh` exited. `accepted` defaults to `false` unless the caller sets
`ACCEPTED=true` in the environment, since `run.sh` cannot know the Decide-step verdict at
grading time — nothing ships without an explicit Decide anyway.

## frontier.jsonl

The per-candidate score archive GEPA loop step 2 (Propose) reads before choosing what to
mutate from. One line per candidate ever tested for this artifact, including the
incumbent itself the first time `run.sh` runs after this section is adopted:

```json
{"candidate_id":"<short hash of the candidate's full text>","tested_against":"<prompt_version of the incumbent it competed with>","scores_nonholdout":[7,8,6],"scores_holdout":[7],"mean_nonholdout":7.00,"accepted":false,"ts":"<local iso with offset>"}
```

- `candidate_id`: a short hash (e.g. `sha1sum | cut -c1-8`) of the candidate's exact text —
  stable identity independent of whether it shipped.
- `scores_nonholdout` / `scores_holdout`: the per-case score arrays `run.sh` already prints
  to stdout, in case order — same numbers, just kept instead of discarded.
- The candidate's full text (not a diff) lands in `evals/frontier/<candidate_id>.md`, so a
  later Propose step can load a non-incumbent frontier member and mutate from it directly.
- **Pruning**: cap `evals/frontier.jsonl` at the 20 newest entries per artifact. When over
  cap, drop the oldest *dominated* entry first — an entry whose score vector is beaten or
  tied everywhere by some other entry's vector, with at least one strict loss. Never prune a
  non-dominated (frontier) member for being old; dominance status is the only pruning signal.
  Delete the matching `evals/frontier/<candidate_id>.md` in the same step its jsonl line is
  pruned, so the two files never drift out of sync.
- **Tracked in git, not gitignored.** Unlike `logs/usage.jsonl` and `votes/votes.jsonl`
  (gitignored because they carry real session excerpts and correction notes — personal
  usage content), `frontier.jsonl` and `frontier/` only ever hold scores against this
  artifact's own synthetic, already-tracked `cases.jsonl`, plus candidate prompt text —
  structurally just draft variants of the artifact's own already-tracked definition file.
  Neither touches live session content, so there is no sensitive-data reason to exclude it,
  and tracking it means a fresh clone can recompute the frontier — unlike the reproducibility
  gap already flagged for `logs/`/`votes/` in artifacts' own `TUNING.md` deferred lists.
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
  `candidate_id`'s line gets `accepted` flipped.
  If a
  specific artifact's eval cases start encoding something sensitive, gitignore that one
  artifact's frontier paths — never a repo-wide default change.

## Holdout gating rule

A candidate replaces the incumbent only when, on the same cases:

1. no case is graded catastrophic that wasn't before — hard reject, regardless of mean
2. mean score is higher — tie goes to the incumbent, no churn on noise
3. the win holds on the holdout slice — otherwise it's overfitting, reject
4. two candidates both pass 1–3 → the one adding fewer conditions ships (weakest wins)

## The `## logging` section (paste into the artifact's definition)

An artifact with an eval section but no logging section is not done — same rule as a
missing harness. Paste this at the end of the definition, fill `<name>`:

````markdown
## logging

At the end of a use, append ONE JSON(JavaScript Object Notation) line to
`<repo-root>/<artifact-dir>/logs/usage.jsonl`, where `<repo-root>` is the output of
`git rev-parse --show-toplevel` run from inside this repo — never a path relative to
the caller's own working directory, which may not be the repo root:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"<name>","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md' ':(exclude)**/logs/**' ':(exclude)**/votes/**'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
- `ts` is the machine's current local timezone with offset
  (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user
  analyzes these against their own day.
- The excerpt is the relevant transcript parts only — the trigger, the key outputs,
  any human correction. Never the full transcript; cap ~2KB per line.
- `outcome` grades THIS RUN'S EXECUTION OF THE ROLE, never the deliverable and never the
  code under test. `success` covers a correct refusal, an invalid-dispatch, a `blocked`
  verdict naming its precondition, a repro that did not reproduce, and an evidenced
  `fail`. `failure` is the role misfiring: improvising past a missing input, grading on a
  self-report, editing a file its contract bars. `partial` is a run cut short. A run that
  grades itself `success` while holding a known coverage hole is `partial`, and names the
  hole. Eight blind-judge votes were spent on this one distinction.
````
