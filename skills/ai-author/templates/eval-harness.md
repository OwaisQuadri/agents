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

Convention: `./run.sh [candidate-file]` — runs every non-holdout case against the current
artifact (or the candidate, if given), grades each with rubric.md, emits one JSON line
per case to stdout (`{"id":"c1","score":7,"failure_mode":"<tag-or-null>"}`) and a
summary to stderr. `--holdout` runs the holdout slice instead.

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

At the end of a use, append ONE JSON(JavaScript Object Notation) line to this
artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"<name>","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md'`. A
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
