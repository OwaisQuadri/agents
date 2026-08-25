---
name: skill-author
description: Use when authoring a new skill or rewriting an existing one — its trigger description, body recipe, disclosure files, evals, or logging. Skip for agents (agent-author) and workflows (workflow-author) — different anatomy — and for merely running a skill.
metadata:
  minimum-tier: T4
  short-description: The deep craft of authoring skills
---

# skill-author

A skill is a reusable node: one bounded job, a defined input (the situation that fires it
plus what the run arrives with), a defined output shape. A skill whose output is a wall of
free text is a skill only a human can read; a fixed output shape is what lets the next
agent consume the result without guessing. Everything below serves one virtue:
predictability — the agent takes the same PROCESS every run, not the same output.

## 0. Dedupe first

Read every existing skill's name + description. If one already owns the capability,
sharpen THAT skill (trigger, step, gotcha) — never a sibling.

## 1. Write the node contract first

Three lines before any prose. Can't write them → you don't have a skill yet.

```
JOB: <one bounded job, one sentence. two sentences means two skills>
IN:  <the triggering situation + what the run arrives with: files, args, state>
OUT: <the output shape: files written, report fields, a verdict enum. never "a summary">
```

The contract is the spine: the description sells IN, the body produces OUT, the evals
grade OUT against IN.

## 2. Anatomy

### Description — the trigger contract

The one part loaded into context every turn, so it pays rent every turn: prune hardest.

- Lead with "Use when <situation>" — the situation, not the topic. "Use when a build
  fails with X" fires; "helps with builds" never does.
- Front-load the leading word (section 3) — invocation is where it earns most.
- One trigger per branch. Synonyms restating one branch are duplication; collapse them.
- Always a "Skip when …" clause fencing off the nearest false positive.
- Widest "Use when" the evidence permits. Narrowing an existing trigger costs an
  observed false fire from the logs, never an imagined one — the Skip-when fence for a
  sibling is routing, not narrowing.
- Hand-only skill, never fired by the model or another skill? Set
  `disable-model-invocation: true` — the description becomes a human one-liner and costs
  zero context.

### Body — the imperative recipe

- Ordered steps, imperative mood. Code > prose: exact commands, exact paths.
- Every step ends on a checkable completion criterion — the agent can tell done from
  not-done ("every modified model accounted for", never "produce a change list"). Where
  it matters, make it exhaustive too; a vague bound invites quitting early.
- State the OUT shape from the node contract explicitly — a template, a
  JSON(JavaScript Object Notation) schema, a file layout. The next node reads OUT, not
  your steps.

### Disclosure files — the ladder

Content sits on a ladder: in-file step (primary) → in-file reference → reference
disclosed to a sibling .md behind a pointer. Two rules place material:

- Branch test: inline what every run needs; push behind a pointer what only some
  branches reach. A skill with one path discloses nothing.
- The pointer's WORDING decides when the agent opens the file — "for X, read y.md" with
  a sharp X. Must-have material behind a weak pointer is a variance bug: fix the wording
  first, inline only if that fails.

Name each file for what it holds. Never disclose a step every run executes.

## 3. Craft rules

- Leading word: one pretrained concept (tight, red, refute, skeptic) that replaces a
  restated phrase. In the body it anchors execution; in the description it anchors
  invocation — use the word you would naturally type when you want the skill. A made-up
  word recruits no priors; reach for an existing one.
- No-op test, per sentence: a line earns its place only if it changes behavior versus
  what the agent does by default. "Be thorough" is a no-op; "relentless" might not be.
  When a sentence fails, delete the whole sentence — don't trim words from it.
- Single source of truth: each meaning lives in exactly one place. Repeating it costs
  tokens and inflates the meaning's apparent importance.
- Match the register of the config it lives in. Terse imperative gets executed;
  enterprise documentation gets skimmed.
- Before writing step prose, exit criteria, or dispatch text, read
  ~/Documents/agents/docs/prompt-style.md — the Simplified Technical English
  (ASD-STE100) rules that leave a sentence one reading.

## 4. The authoring contract — every skill ships it

Every skill authored here carries an "## evals" and a "## logging" section plus their
support files. No harness = not done.

### The "## evals" section

Copy skills/ai-author/templates/eval-harness.md into `<skill>/evals/`:

- `cases.jsonl` — ≥5 cases (`{"id","input","expect","holdout","source"}`), ~20% (min 1)
  marked holdout and never shown to the mutation-proposer. Grow cases from real
  usage-log failures and judge votes, not imagination.
- `rubric.md` — the single-sourced 0-10 grading contract; the blind post-use judge and
  the eval judge read the SAME rubric. List this skill's catastrophic, un-tradeable
  failures explicitly (false pass, wrong autonomous action, hallucinated paths) — a
  catastrophic case can never be traded against a better mean.
- `run.sh` — convention: `./run.sh [candidate]` grades every non-holdout case, one JSON
  line per case to stdout, summary to stderr; `--holdout` runs the held-out slice.
- Holdout gating: a candidate replaces the incumbent only when no new catastrophic,
  higher mean, AND the win holds on the holdout slice. Ties go to the incumbent; two
  passing candidates tie → the one adding fewer conditions ships.

The authored skill's own "## evals" section is 2-4 lines: what run.sh checks, how to
invoke it.

### The "## logging" section

Every authored skill ends with a short "## logging" section (paste-ready block in the
same eval-harness template) instructing: at the end of a use, append ONE bounded JSON
line to the skill's `logs/usage.jsonl` — the relevant
transcript excerpt only (trigger, key outputs, any human correction), ~2KB cap, never
the full transcript. The timestamp is the machine's CURRENT LOCAL TIMEZONE with offset,
never UTC(Coordinated Universal Time): these logs get analyzed against the user's own
day, and UTC lands that analysis in the wrong hours.

```sh
date +%Y-%m-%dT%H:%M:%S%z   # 2026-07-31T02:45:10-0400
```

```json
{"ts":"2026-07-31T02:45:10-0400","artifact":"<name>","trigger":"<what fired it>","excerpt":"<bounded>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
## 5. Failure modes and drift signals

Diagnose a misbehaving skill against these, in order of frequency:

- Trigger too broad — fires on the topic instead of the situation, or on a neighboring
  skill's territory. Fix: sharpen "Use when", add the missing "Skip when".
- Trigger too narrow — never fires while its territory keeps coming up; qualifiers
  piled up against imagined misfires. Fix: delete every qualifier no logged false fire
  paid for; re-widen "Use when" to the situation class.
- Description no-ops — trigger words nobody ever types, identity restated from the body.
  Fix: rewrite triggers in the words actually used when the skill is wanted.
- Premature-completion drift — runs end before the criterion is met. Fix: sharpen the
  completion criterion first (cheap, local); only if it is irreducibly fuzzy AND you
  observe the rush, split the sequence so later steps sit out of view.
- Stale skill — commands or paths that no longer exist, layers nobody cleared. Fix:
  prune by relevance, line by line; shorter skills stay true longer.

Any of these observed in the wild sends the skill back through GEPA(Genetic-Pareto
prompt evolution) — the GEPA loop in skills/ai-author: reflect over logs and votes, propose
one-concern mutations, accept only on a harness win that holds on holdout. Concrete
tripwires: the same human correction in 2+ usage lines, `failure`/`partial` outcomes
clustering, a judge vote naming the same lack twice, or a skill that never fires while
its territory keeps coming up (trigger too narrow).

## Done when

- Node contract (JOB/IN/OUT) written; the body's output matches OUT.
- Description: "Use when" + leading word + "Skip when"; every sentence passes the no-op
  test.
- Every step ends on a checkable completion criterion.
- `evals/` holds cases.jsonl, rubric.md, run.sh; "## evals" and "## logging" sections
  present in the body.
- `./install.sh` run, so the skill resolves from ~/.agents/skills and both agent roots.

## logging

At the end of a use of THIS skill, append one bounded JSON line to
skills/skill-author/logs/usage.jsonl per section 4's logging spec — local timezone with
offset, ~2KB cap.
