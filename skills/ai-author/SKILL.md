---
name: ai-author
description: Use when deciding whether a skill, agent, or workflow should be authored at all, when authoring one, or when tuning one from its accumulated logs and votes. The umbrella authoring skill. Owns the should-it-exist decision tree, the authoring contract (every artifact ships evals/ + logs/ + votes/), the blind fresh-context judge, and the GEPA(Genetic-Pareto prompt evolution) loop. Hands type-specific craft to skill-author, agent-author, or workflow-author. Skip for a one-off task with no reusable capability, or when the type is already decided and only craft depth is needed (go straight to the sibling author skill).
metadata:
  minimum-tier: T4
  short-description: Umbrella author + GEPA-tune for skills, agents, workflows
---

# ai-author

The umbrella for authoring skills, agents, and workflows. No artifact exists without a
reason to fire, a way to measure it, and a loop that improves it. GEPA(Genetic-Pareto
prompt evolution) is built in: every authored artifact ships an eval harness, logs its
uses, gets blind-judged, and mutates only on measured wins.

## what arrived? (route before you decide)

The tree below answers "should this be authored". It does not answer "what am I being asked to
do", and the dominant complaint in the vote history is an invocation that was not a fresh
authoring job getting walked into the tree anyway, with the operator improvising a verdict the
tree never sanctioned. Route first, and stop at the first match:

- **A destination outside the artifact system.** The discriminator is ALWAYS-ON versus
  TRIGGERED, never the topic. Guidance that should apply without anything firing it belongs in
  CLAUDE.md, memory, or settings; anything with a trigger, however stylistic its subject, is a
  candidate for the tree. "Comments follow the whitelist" exits here. "Before every commit,
  sweep the diff's comments against the whitelist" does not — same topic, and it fires on a
  commit. That is a legitimate terminal state and it exits here. It is
  not a skill, and the tree below would wrongly make it one, because "recurring linear recipe"
  matches ambient guidance perfectly.
- **An update to an existing artifact.** Mutating something that already exists is not
  authoring, and the contract below does not govern it. Go to the GEPA loop, and read the
  acceptance rule for changes the harness cannot measure. This branch comes BEFORE the built-in
  one on purpose: "our code-reviewer misses X and the built-in covers most of it" is an update
  to code-reviewer, and the built-in branch would wrongly answer "author nothing".
- **A capability a BUILT-IN already covers, and no authored artifact does.** A built-in agent or
  tool is not an authored artifact, so rule 1's "update it" cannot apply to it. Author nothing
  unless a harness shows a custom definition beating the built-in. Absent that harness, the
  answer is no.
- **An artifact plus a live task** ("run this under protocol", "use X to do Y"). Executing an
  artifact is not authoring it and not tuning it. Do the task. Only fold it back as an eval
  case or a log line if that was asked for — burning someone's live request as harness fodder
  is the failure this branch exists to prevent.
- **Anything else** falls through to the tree.

A verdict this skill reaches but does not execute this turn is tracked to execution or
explicitly dropped, in writing. A deferred verdict that lives only in a log line is a dropped
one — so name the destination: the artifact's `TUNING.md` under its deferred heading, or a ticket
in the relevant project's roadmap when the verdict is project work. Either one is somewhere a
later pass reads without being told to look, which is the whole property. Sweep every verdict the
pass reached, not only the most recent.

## should it exist?

TWO questions, in this order. The type tree runs only on what survives the first question.

### question one: can a program do it? (ASK THIS FIRST, EVERY TIME)

A prompt costs tokens on every load and can forget. A program costs nothing at decision
time and cannot. So before the word skill, agent or workflow is spoken, ask whether the
thing is decidable from files, diffs, and exit codes with no taste involved. Two
destinations, same test, different trigger:

- **a checker** — an agent invokes it deliberately. Rust, in `tools/`, per AGENTS.md.
- **a pi extension** — the RUNTIME fires it on an event: session start, a tool call, a file
  change, a render. TypeScript, in `pi/extensions/`. Reach for this when the thing must not
  depend on an agent remembering to run it.

DECIDABLE IS NOT THE SAME AS RIGHT. Before building a checker, say which quantity it
measures and which quantity you actually care about, and check they are the same one. A
checker that is deterministic about the wrong quantity is worse than no checker, because it
carries authority and it fires on every run. Cyclomatic complexity is the worked example: a
program computes it exactly, and it still does not measure whether the code is clear — a
flat match over twelve variants is the clearest shape available and scores badly, and the
repair the number rewards is extracting a helper, which this repo's own rule against
abstraction forbids. Clippy says the same about its own lint, which is allow-by-default and
carries a note recommending `excessive_nesting` and `too_many_lines` instead. Where the
quantity does not match, the thing is a REVIEW SIGNAL, never a gate.

Three rules once a program owns it:

- Most rules SPLIT rather than fall one way. Take the mechanizable core into the tool and
  leave the prose the judgment residue only. "Never invent a specific" splits into a checker
  listing every number, hash, and path in the output that is absent from the input, and a
  prose rule about which of those the writer may keep.
- Where a tool owns a rule, the prose NAMES the tool and never restates its constant. A
  restated constant drifts: `skills/mouthpiece/SKILL.md` capped lists at 3 while
  `tools/ste-check` enforced 5, and 14 logged lines record agents caught between them.
- Prefer the shape that makes the failure IMPOSSIBLE over the one that makes it visible. A
  tool that writes the log line beats a rule asking for care with shell quoting, which the
  fleet lost 19 unreadable lines to.

### repair costs a turn, not a token

A checker that rejects work sends it back to be redone, and re-running the producing agent
re-reads its whole context to change three characters. Never repair that way. Three tiers,
in order:

1. IMPOSSIBLE (0 tokens) — the tool constructs the artifact, so the bad shape has no way to
   exist. Every bookkeeping rule belongs here.
2. DETERMINISTIC REPAIR (0 model tokens) — where a failure has one correct repair, the
   checker APPLIES it rather than reporting it. An auto-fix never touches a number, a path,
   a quoted span, or anything in backticks: those are facts, and fabricating them is the
   worst failure in the log.
3. SPAN-SCOPED REPAIR (~300 tokens) — what is left goes to a cheap tier with the failing
   span, the failure line, and the facts to preserve. It returns the replacement span. The
   producer never re-runs. Cap at 2 attempts, then ship with the failure named.

A checker reports only its failures, never its passes. A 20-line report to say three things
is three things and seventeen wasted.

### question two: what type? (only for what a program cannot do)

Decide in order, stop at the first match:
1. An existing artifact already owns the capability → update it. Never author a sibling.
2. A true one-off, pure Q&A(question and answer), or unrepeatable → author nothing.
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
  TUNING.md          # the loop's record: mutations, deferred verdicts, the open list
  evals/             # the GEPA harness — copy templates/eval-harness.md
    cases.jsonl
    rubric.md
    run.sh
  logs/usage.jsonl   # appended per the artifact's "## logging" section
  votes/votes.jsonl  # blind judge votes; written ONLY by scripts/submit_vote.py
```

Frontmatter must parse under strict YAML, not just the lenient parser one client happens
to use. A plain scalar that contains `: ` (colon plus space) is illegal YAML and breaks
pi's loader even though Claude Code accepts it. Any frontmatter value containing `: ` is
written as a `>-` block scalar. Check before shipping:

```sh
node -e "const y=require('yaml'),fs=require('fs');y.parse(fs.readFileSync(process.argv[1],'utf8').split('---')[1])" <artifact>.md
```

No harness = not done. No `## logging` section = not done. `templates/eval-harness.md`
carries both the harness files and the paste-ready logging section. A draft goes live
only when every non-holdout case passes and the holdout slice holds (rule in the template).

## judge protocol (blind by construction)

After logging, dispatch a fresh-context judge:

- Fresh context: the judge receives the artifact's source and the just-logged usage line
  ONLY. It must NOT read `votes/`, prior `logs/` history, or any other vote.
- It grades harshly, strictly, critically, constructively: a grade (letter or 0-10) plus
  an open-ended vote on where to adjust the artifact and what it is lacking.
- It submits ONLY via the script — never by editing files. The first line of the vote is
  the exact `prompt_version` from the usage line it judged, so a later Reflect pass can
  retire the vote with that prompt:

```sh
printf 'prompt_version: %s\n%s\n' '<short sha from usage line>' '<open-ended vote text>' | \
  python3 skills/ai-author/scripts/submit_vote.py --artifact <name> --grade <grade>
```

The script is append-only and never returns existing votes, so blindness holds by
construction. Aggregation across votes is a separate later pass; no judge ever sees it.

## GEPA loop

Run per artifact, on demand or once logs/votes accumulate:

1. **Reflect**: compute the artifact's current `prompt_version` with the command in its
   logging section. Read only lines in `logs/usage.jsonl` whose `prompt_version` equals that
   value; a missing or different value is stale evidence, counted and dropped. Apply the
   same filter to `votes/votes.jsonl`: keep only votes whose `vote` text starts with
   `prompt_version: <current value>`; a missing or different first line is stale, counted
   and dropped. Then read the surviving usage lines + surviving votes, AND the artifact's
   own `TUNING.md` if it has one, whose open list is the standing input. Build a failure histogram — which criteria fail
   most, which complaints repeat — and record it in the usage line with the vote
   indices it came from, so the next pass can recompute it from `votes.jsonl` instead of
   trusting this one. A blind judge cannot open `votes/`, so an asserted count is unverifiable
   by the only fresh reader the pass gets. An open list nothing re-reads is a dead letter, which
   is why it is an input here.
2. **Propose**: targeted mutations aimed at the top failure modes (sharpen the trigger,
   add a skip-when, tighten a step — or widen a trigger the logs show never firing).
   Small, named, one concern each. Every mutation states whether it is PROSE, a CHECKER, or
   a PI EXTENSION, against question one above. A mutation a checker could enforce ships as the checker,
   because the failure it answers already survived the prose telling an agent not to do it. A narrowing mutation names the logged false positive
   it answers; none logged → don't narrow.
3. **Test**: run `evals/run.sh` — incumbent vs candidate on the same cases.
4. **Decide**: accept ONLY on a harness win — no new catastrophic failure, higher mean
   score, and the win holds on the holdout slice. Ties go to the incumbent; two
   candidates tying each other → the one adding fewer conditions ships (weakest wins).
   No churn on noise. **That rule assumes the harness can see the change, and it silently
   assumes the mutation is a tuning tweak.** When the mutation is a DEFECT FIX whose effect no
   existing case measures, a tie is the expected result and rejecting on it would mean a proven
   bug can never be fixed. So: a defect fix ships on a reproduction plus execution evidence,
   NOT on a mean, and the pass is only complete when a fenced case author has been dispatched
   in the SAME pass to add the missing case. The fence bars the mutation-proposer from writing
   the exam; it has never barred dispatching someone who can, and recording the coverage debt
   in a history line instead is a dead letter. Say plainly which of the two paths was used.
   Reporting a tie as a harness win is the failure this clause exists to stop.
5. **Record**: note the accepted mutation and its rationale in the artifact's `TUNING.md` so
   history is auditable and reversible. It never goes in the body, which every run loads.

Fence: the mutation-proposer never writes `evals/` cases, the rubric, or `votes/`. The
exam stays out of the student's hands — and dispatching someone else to type it does not
satisfy the fence if they can see the answer. **A mutation is not accepted on cases authored
with sight of that mutation's text.** The case author gets the failure histogram and the
reproduction; never the candidate diff, never the mutated file. Otherwise every EXPECT
paraphrases the new wording, the ablation stub scores 0 by construction, and the "win" measures
nothing but the grader agreeing that the text says what it says. This is a weaker rule than
banning self-tuning, and deliberately so: self-tuning is not the failure mode, answer-key cases
are, and those are open to every author.

## logging

Every authored skill and agent carries a short `## logging` section (paste-ready text in
`templates/eval-harness.md`). This is ai-author's own, and the model for all of them:

At the end of a use, append ONE JSON(JavaScript Object Notation) line to
`skills/ai-author/logs/usage.jsonl`:

```json
{"ts":"2026-07-31T14:05:09-0400","artifact":"ai-author","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
- `ts` is the machine's current local timezone with offset
  (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user
  analyzes these against their own day.
- Bounded: the excerpt is the relevant parts only — the trigger, the key outputs, any
  human correction. Never the full transcript; cap ~2KB per line.

These lines are the GEPA reflective dataset, not a dump nobody reads.
