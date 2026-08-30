---
name: ideate
description: >-
  Use to brainstorm and file new work — nothing existing fits what's needed, pick-task
  hands off here, or you want to deliberately think past the obvious next ticket.
  Researches the space, reframes it through distinct lenses, grills you toward a
  concrete direction, then files the result behind a human gate. Skip when a task is
  already picked (engineer owns execution) and skip for selecting among existing work
  (pick-task owns that).
metadata:
  minimum-tier: T3
  short-description: Research, reframe, grill, file — brainstorm new work
---

# ideate

JOB: turn an open space ("what should we build next", "nothing fits, what now") into
     concrete, filed candidates
IN:  a prompt — an explicit ask to brainstorm, or a hand-off from pick-task when nothing
     in the backlog matched what the user wanted
OUT: candidate ideas filed into the task backend (GitHub Issues/Projects, Linear, or
     root `roadmap.json`) after a human gate, plus `inspiration.md`'s adopted-reference
     list updated when a candidate cites new inspiration

## steps

1. **Ground it.** State the space in one sentence — what's actually open here (a
   product area, a pain point, "anything"). Read `inspiration.md` for prior taste: what
   has previously earned a "yes" (adopted references), and the standing stance (prefer
   what deletes future work, cite the bleeding edge only when something shipped proves
   it, one ambitious "iron man" candidate per round).

2. **Research the space.** Dispatch the research workflow (`workflows/research-sweep/`)
   with a goal built from step 1 — it fans out web, academic/news/design, and codebase
   angles and returns cited findings blocks. Read the summary before reframing; a
   direction with no grounding is a guess, not a candidate.

3. **Reframe.** Apply `lateral-syntactic-drift`'s lens technique to the researched
   space: restate the problem through several lenses (actor, constraint, timescale,
   metaphor), generate at least one concrete idea per lens, and keep the obvious
   default candidate visible alongside the escapes — never let a reframe replace it
   silently.

4. **Grill toward a direction.** Same interrogation spirit as `pick-task`: don't
   present a list and wait — ask which candidates land, which don't, what's missing.
   Steer based on the answers rather than taking the first "sure, that one."

5. **Land 1-4 candidates**, each with: the idea, why it fits the researched space, its
   lineage (which lens or research finding produced it), and rough size (small/medium/
   large — bigger tasks are fine here).

6. **File — behind a human gate, always.** Show the candidates plainly, wait for a yes
   before writing anything. On approval: check for a connected task system first (`gh
   issue list`/`gh project`, Linear MCP tools) the same way pick-task does; file there
   if one exists, otherwise append to root `roadmap.json` with a fresh id. Filing into
   GitHub Issues means `gh issue create` with `status:todo` and `priority:<p>` labels
   and `--blocked-by <ids>` for any dependency — the same conventions
   `skills/task-graph/SKILL.md`'s "GitHub Issues backend" section documents, so
   `next-issue.sh` picks the new item up correctly. A candidate citing a new adopted
   reference gets that reference appended to `inspiration.md`. No manifest or snapshot
   machinery — a plain log line recording what was shown and what was approved is the
   record.

## evals

`evals/run.sh` grades every non-holdout case in `evals/cases.jsonl` against this file or
a candidate, using `evals/rubric.md`; `--holdout` runs the held-out slice.
