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

## Usage evidence (no logging section to paste)

No artifact carries a logging section. `tools/gepa-due` derives usage evidence directly
from real Pi session transcripts — a `read` tool_call whose path matches this artifact's
own definition file, filtered by the time cutoff in `skills/ai-author/SKILL.md`'s "usage
evidence" section. Nothing to paste here; the eval harness above is the only thing a new
artifact needs to ship complete.
