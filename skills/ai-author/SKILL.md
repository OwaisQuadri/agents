---
name: ai-author
description: Use when deciding whether a skill, agent, or workflow should be authored at all, when authoring one, or when tuning one from its accumulated logs and votes. The umbrella authoring skill. Owns the should-it-exist decision tree, the authoring contract (every artifact ships evals/ + logs/ + votes/), the blind fresh-context judge, and the GEPA(Genetic-Pareto prompt evolution) loop. Hands type-specific craft to skill-author, agent-author, or workflow-author. Skip for a one-off task with no reusable capability, or when the type is already decided and only craft depth is needed (go straight to the sibling author skill).
metadata:
  short-description: Umbrella author + GEPA-tune for skills, agents, workflows
---

# ai-author

The umbrella for authoring skills, agents, and workflows. No artifact exists without a
reason to fire, a way to measure it, and a loop that improves it. GEPA(Genetic-Pareto
prompt evolution) is built in: every authored artifact ships an eval harness, logs its
uses, gets blind-judged, and mutates only on measured wins.

## should it exist?

Decide in order, stop at the first match:
1. An existing artifact already owns the capability → update it. Never author a sibling.
2. Genuine one-off, pure Q&A(question and answer), or unrepeatable → author nothing.
3. Linear recipe a single agent follows → **skill** (prose SKILL.md).
4. A distinct role needing its own context, tools, and judgment, dispatched fresh → **agent**.
5. Fans out over ≥2 agents, loops over items, or has a generate→judge shape → **workflow**.

One-sentence test: if the artifact can't justify its existence in one sentence, don't author it.

## type depth: the sibling authors

ai-author decides WHAT to author and enforces the contract below. HOW each type is
crafted lives in one sibling skill per type. Once the tree picks a type, dispatch to it:

- **skill-author**: how to author a skill (the SKILL.md craft itself).
- **agent-author**: how to author an agent definition.
- **workflow-author**: how to author a workflow.

Each sibling owns its type's specifics in full. Nothing type-specific is duplicated here.

## the authoring contract

Every artifact this skill authors ships:

```
<artifact>/
  SKILL.md | <agent>.md | *.workflow.js   # ends with its own "## logging" section
  evals/             # the GEPA harness — copy templates/eval-harness.md
    cases.jsonl
    rubric.md
    run.sh
  logs/usage.jsonl   # appended per the artifact's "## logging" section
  votes/votes.jsonl  # blind judge votes; written ONLY by scripts/submit_vote.py
```

No harness = not done. No `## logging` section = not done. `templates/eval-harness.md`
carries both the harness files and the paste-ready logging section. A draft goes live
only when every non-holdout case passes and the holdout slice holds (rule in the template).

## logging

Every authored skill and agent carries a short `## logging` section (paste-ready text in
`templates/eval-harness.md`). This is ai-author's own, and the model for all of them:

At the end of a use, append ONE JSON(JavaScript Object Notation) line to
`skills/ai-author/logs/usage.jsonl`:

```json
{"ts":"2026-07-31T14:05:09-0400","artifact":"ai-author","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `ts` is the machine's current local timezone with offset
  (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user
  analyzes these against their own day.
- Bounded: the excerpt is the relevant parts only — the trigger, the key outputs, any
  human correction. Never the full transcript; cap ~2KB per line.

These lines are the GEPA reflective dataset, not a dump nobody reads.

## judge protocol (blind by construction)

After logging, dispatch a fresh-context judge:

- Fresh context: the judge receives the artifact's source and the just-logged usage line
  ONLY. It must NOT read `votes/`, prior `logs/` history, or any other vote.
- It grades harshly, strictly, critically, constructively: a grade (letter or 0-10) plus
  an open-ended vote on where to adjust the artifact and what it is lacking.
- It submits ONLY via the script — never by editing files:

```sh
echo "<open-ended vote text>" | \
  python3 skills/ai-author/scripts/submit_vote.py --artifact <name> --grade <grade>
```

The script is append-only and never returns existing votes, so blindness holds by
construction. Aggregation across votes is a separate later pass; no judge ever sees it.

## GEPA loop

Run per artifact, on demand or once logs/votes accumulate:

1. **Reflect**: read `logs/usage.jsonl` + `votes/votes.jsonl`. Build a failure histogram —
   which criteria fail most, which complaints repeat.
2. **Propose**: targeted mutations aimed at the top failure modes (sharpen the trigger,
   add a skip-when, tighten a step — or widen a trigger the logs show never firing).
   Small, named, one concern each. A narrowing mutation names the logged false positive
   it answers; none logged → don't narrow.
3. **Test**: run `evals/run.sh` — incumbent vs candidate on the same cases.
4. **Decide**: accept ONLY on a harness win — no new catastrophic failure, higher mean
   score, and the win holds on the holdout slice. Ties go to the incumbent; two
   candidates tying each other → the one adding fewer conditions ships (weakest wins).
   No churn on noise.
5. **Record**: note the accepted mutation and its rationale in the artifact so history is
   auditable and reversible.

Fence: the mutation-proposer never writes `evals/` cases, the rubric, or `votes/`. The
exam stays out of the student's hands.
