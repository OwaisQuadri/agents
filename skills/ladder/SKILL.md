---
name: ladder
description: Use when a technology has to be learned deeply rather than skimmed — a language, framework, or stack where the goal is mastery and the user will be producing, not consuming summaries. Ladders it: honest per-dimension placement against what he already knows, the red cells that are the whole curriculum, one checkable artifact per rung, ending in a project he maintains. Skip for a single how-do-I question, for shipping a feature where learning is incidental, and for running an existing ladder (the ladder file is already the instructions).
metadata:
  short-description: Build a rung ladder for learning a technology to mastery
---

# ladder

JOB: turn one named technology into a rung ladder — dimension placement, red cells, one artifact per rung ending in a maintained project, and a seeded interference log
IN: the technology; what the user already knows (languages, stacks, shipped work); any plan, deadline, or repo already committed to
OUT: `LADDER.md` written into the learning repo, sections fixed by step 8

Core inversion, and the reason every step below produces an artifact rather than an
explanation: summaries mean the model produces and he consumes, which retains nothing.
Mastery means he produces and the model attacks.

## 1. Find the plan that already exists

`rag search "<technology> plan|learning|ladder" --json`, and check the learning repo for
a prior file. He usually has already agreed something — merging into it beats a fresh
plan he'll have to reconcile himself.

Note that the store does NOT cover claude.ai web chats. If the plan is remembered but
absent, say so and ask him to paste it rather than reconstructing it.

Done when: the prior plan is in hand, or its absence has been stated as a fact rather
than assumed.

## 2. Settle the time horizon

Two modes, and the wrong one produces a ladder he can't use:

- **No deadline mentioned → full mastery.** The default. No ceiling, the whole rung set,
  measured in months. Don't invent urgency he didn't state.
- **A deadline mentioned → the deadline dominates everything.** An interview on Friday, a
  demo next month, a job starting in three weeks. Rung selection now answers one
  question: what is reachable *inside the box*, ordered by what the deadline actually
  tests.

A deadline never deletes the far rungs — it splits the ladder. Rungs past the box are
kept, in order, under an explicit "After the deadline" heading. They are not scope; they
are the reason the ladder is still a ladder and not a cram sheet.

Done when: the mode is stated in one line, and if there's a deadline, the box is written
as a concrete date or duration.

## 3. Place him per dimension

Break the technology into 8-12 dimensions and score each R0-R5 against what he already
ships. Emit a table: dimension, level, one-line note naming the transfer.

The point is to NOT restudy what transfers. Value semantics, pattern matching, async —
these arrive paid for from Swift and Dart, and a ladder that re-teaches them wastes the
weeks that matter.

Done when: every dimension carries a level and a stated reason, and **the red cells are
named explicitly** — the one or two R0 dimensions that are the entire curriculum.
Over-scoring a genuinely new dimension is the expensive error; it hides the red cell.

## 4. Adapt the loop to this technology

Five steps, each rewritten for what this technology actually gives you:

1. **Primary source first.** Name the specific ones: the spec, the reference, the RFC,
   the crate's own source. Never a blog post, never an "introduction to". The model
   translates one paragraph he's stuck on; it never narrates the chapter.
2. **Predict before you run.** Name this technology's ground-truth oracle — the compiler,
   the type checker, the test runner, the profiler. He writes the prediction, then runs.
   The delta is the curriculum. When he misses, the question is never "explain the
   error": it is *which mental model produced my prediction, and what else will that
   broken model make me get wrong.* Errors cluster around one bad model.
3. **Reimplement from scratch, no reference.**
4. **Inverted Feynman.** He lectures, the model cross-examines: no praise, no summarizing
   him back, a concrete example demanded for every abstraction, stop when it finds
   something he faked. Dispatch this fresh — an examiner that watched him build the
   ladder will fill his gaps for him.

   *No examiner agent exists yet.* Decided 2026-08-07: author it via `/ai-author` the
   first time this rung is actually reached, not before. `grill-me` is referenced in his
   source material but is not installed on this machine — don't send him to it.
5. **Free recall.** Blank page, write the rules from memory, diff against the source.
   Recognition drills (rustlings, quizzes, multiple choice) are volume, never the check.

Done when: step 2 names a concrete oracle for this technology and step 1 names at least
two real primary sources that exist.

## 5. Lay the rungs

One row per rung: number, artifact, dimension it moves, and where it came from if it
maps onto an existing plan.

- Every rung's artifact is checkable by someone who isn't him — a thing that builds,
  runs, renders, or merges. "Understands X" is not a rung.
- Scaffolding rungs (toolchain, project setup, CI) are marked as such. They don't move
  the red cells and must not be scored as progress.
- Include a drill rung: small variations under time, cold, no reference. This is where
  the model breaks under pressure and it is the rung most plans omit.
- Include a foreign-code rung before the top: a merged patch to a dependency. Merged,
  not opened — review is the grading step. Writing his own code and patching someone
  else's are different skills.
- **The top rung is always a long-term project he must keep maintaining.**

Under a deadline (step 2), split the same table: rungs inside the box first, then an
"After the deadline" heading carrying the rest — including the foreign-code rung and the
maintained project, which almost never fit a short box. Never drop them to make the box
look complete, and never claim a deadline slice is the whole ladder.

Done when: every rung has an artifact a stranger could verify, the top rung is a
maintained project, and under a deadline every rung sits on the correct side of the split.

## 6. Seed the interference log

A `wrong.md` in the learning repo, one line per prediction miss, reread monthly.

Seed it before he starts, from the habits his current stacks trained: a two-column table
of his habit against this technology's reality. Then name the single row that fails
*silently* — the one that looks like a different problem entirely. Everything else
announces itself; that one costs days.

Done when: the table has ≥8 rows and one is called out as the silent one.

## 7. Set cadence

Per week: 3h primary source with no model, 4h build, 2h adversarial, 1h free recall of
*older* material. Scale the hours to what he actually has; keep the ratio.

Under a deadline, switch the unit from weeks to whatever the box is made of — days for an
interview, sessions for a demo — and keep the ratio.

## 8. Write LADDER.md

Into the learning repo, sections in this order: provenance (what this merges, dated),
time horizon, dimension table, red cells, the adapted loop, rung table with per-rung
notes (split by the deadline if there is one), cadence, interference log seed, and the
adversary rule verbatim:

> The model explains, grades, and attacks. It never authors.

State any deviation from this skill out loud in the file — an extra rung, a renumber, a
dropped step. Silent deviations are how a plan stops being auditable.

Done when: `LADDER.md` exists at the stated path and every section above is present.

## evals

`evals/run.sh` grades each case in `cases.jsonl` against `rubric.md` with a fresh
`claude -p` judge, one JSON line per case. `./run.sh` runs the non-holdout slice;
`./run.sh --holdout` runs the held-out slice.

## logging

At the end of a use, append ONE JSON(JavaScript Object Notation) line to this
artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-08-07T14:05:09-0400>","artifact":"ladder","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `ts` is the machine's current local timezone with offset
  (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user
  analyzes these against their own day.
- The excerpt is the relevant transcript parts only — the trigger, the key outputs,
  any human correction. Never the full transcript; cap ~2KB per line.

## changes

- **2026-08-07 — step 2, time horizon.** Accepted on a harness win: non-holdout mean
  8.20 → 8.60, the target case (3-day React Native interview) 6 → 8, holdout 9.00 → 9.00
  (non-regression, one case). Seed cases assumed open-ended mastery, so a deadline forced
  the mandatory maintained-project and merged-patch rungs into a box they can't fit.
  Splitting rather than truncating keeps both the deadline and the invariant.
