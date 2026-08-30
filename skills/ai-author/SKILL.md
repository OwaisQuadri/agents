---
name: ai-author
description: Use when deciding whether a skill, agent, or workflow should be authored at all, when authoring one, or when tuning one from its measured usage evidence and votes. The umbrella authoring skill. Owns the should-it-exist decision tree, the authoring contract (every artifact ships evals/ + votes/), the blind fresh-context judge, and the GEPA(Genetic-Pareto prompt evolution) loop. Hands type-specific craft to skill-author, agent-author, or workflow-author. Skip for a one-off task with no reusable capability, or when the type is already decided and only craft depth is needed (go straight to the sibling author skill).
metadata:
  minimum-tier: T4
  short-description: Umbrella author + GEPA-tune for skills, agents, workflows
---

# ai-author

The umbrella for authoring skills, agents, and workflows. No artifact exists without a
reason to fire, a way to measure it, and a loop that improves it. GEPA(Genetic-Pareto
prompt evolution) is built in: every authored artifact ships an eval harness, is
measured from real Pi session transcripts rather than any self-reported log, gets
blind-judged, and mutates only on measured wins.

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
- **A request to mine recent sessions for reusable capability candidates.** Run the bounded
  evidence sweep below. It proposes candidates from evidence and does not author them.
- **Anything else** falls through to the tree.

A verdict this skill reaches but does not execute this turn is tracked to execution or
explicitly dropped, in writing. A deferred verdict that lives only in a log line is a dropped
one — so name the destination: a ticket in the relevant project's roadmap when the verdict is
project work. There is no per-artifact deferred-verdict record anymore (no `TUNING.md`) — an
artifact-specific verdict that isn't project work has no persisted destination and is dropped
explicitly, in writing, rather than silently. Sweep every verdict the pass reached, not only
the most recent.

## bounded session evidence sweep

Run this only when the user asks to mine recent sessions for reusable improvements.

1. Set the window before reading. Use the user's limit, or the ten newest parent Pi sessions.
   Exclude child-agent transcripts. Read only the excerpts needed to identify a task shape.
2. Read no more than ten artifacts named in the session excerpts. Artifacts under this
   contract keep no self-reported usage log — rely on the session excerpts already read in
   step 1 for their evidence. For any OTHER system with its own `run-history.jsonl`, read at
   most its 20 newest records when the file exists. Report how many artifacts or records the
   cap skipped. Keep only repetition, elapsed time, exposed token or dollar cost, result, and
   correction. Never report a prompt, transcript, file content, secret, or opaque identifier.
3. Group the evidence into task shapes. State the observed repetition and measured cost before
   any proposal. Say that cost is unavailable when the records do not expose it. Do not estimate.
4. Reject an isolated, ambiguous, or unmeasured shape. Zero candidates is a valid result.
5. For every surviving candidate, ask whether a program owns it first. Use a checker for a
   deliberate deterministic check, or a Pi extension for a runtime event. Split a mechanizable
   core from its judgment residue. Only then use the existing type tree: update an owner when
   one exists, otherwise prefer a skill. Recommend an agent only when the evidence shows a
   distinct model or a tool grant that the parent must not hold. It may show noisy work that
   needs isolation. Name that ground.
6. Present candidates, not new artifacts. Each candidate names its evidence, routing verdict,
   and a destination for implementation or an explicit written drop.

## should it exist?

TWO questions, in this order. The type tree runs only on what survives the first question.

### question one: can a program do it? (ASK THIS FIRST, EVERY TIME)

A prompt costs tokens on every load and can forget. A program costs nothing at decision
time and cannot. So before the word skill, agent or workflow is spoken, ask whether the
thing is decidable from files, diffs, and exit codes with no taste involved. Two
destinations, same test, different trigger:

- **a checker** — an agent invokes it deliberately. Rust, in `tools/`, per AGENTS.md.
- **something the RUNTIME fires on an event, with no agent remembering to run it** — three
  actual backends live in this repo: Pi's own event bus (a Pi extension, TypeScript wiring
  in `pi/extensions/`), the Claude Code/Codex CLI's own hook system (a CLI hook, wired
  through `config/*.json`'s `"hooks"` key), and git itself (a git hook, bash in `hooks/`,
  symlinked by `install.sh`). All three can call back into a plain Rust checker rather than
  reimplement the logic in a second language.

HOW to build any of these four is `tool-author`'s craft, the same way `skill-author` owns
SKILL.md craft — dispatch there once this branch is picked; nothing type-specific is
repeated here.

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
- **tool-author**: how to author a checker, a Pi extension, a CLI hook, or a git hook.

Each sibling owns its type's specifics in full. Nothing type-specific is duplicated here.

## the authoring contract

Every artifact this skill authors ships:

```
<artifact>/
  SKILL.md | <agent>.md | *.workflow.js
  evals/                 # the GEPA harness — copy templates/eval-harness.md
    cases.jsonl
    rubric.md
    run.sh
    frontier.jsonl        # score-vector archive of every tested candidate — the durable record
    frontier/<id>.md       # full text of every archived candidate
  votes/votes.jsonl      # blind judge votes; written ONLY by scripts/submit_vote.py,
                          # dispatched only from gepa-due's Reflect-time judge sampling
```

No per-artifact usage log, no `TUNING.md`. Usage evidence is derived from real Pi
session transcripts at tuning time (see "usage evidence" below), never self-reported.

Frontmatter must parse under strict YAML, not just the lenient parser one client happens
to use. A plain scalar that contains `: ` (colon plus space) is illegal YAML and breaks
pi's loader even though Claude Code accepts it. Any frontmatter value containing `: ` is
written as a `>-` block scalar. Check before shipping:

```sh
node -e "const y=require('yaml'),fs=require('fs');y.parse(fs.readFileSync(process.argv[1],'utf8').split('---')[1])" <artifact>.md
```

No harness = not done. `templates/eval-harness.md` carries the harness files. A draft
goes live only when every non-holdout case passes and the holdout slice holds (rule in
the template).

## judge protocol (blind by construction)

Votes are never dispatched live, in-session, by the artifact being used — there is no
self-report step left to hang that on. The ONLY path that generates a vote is
`gepa-due`'s own Reflect-time dispatch: when an artifact is due for tuning (its
transcript-hit usage count crosses threshold) AND its `vote_count` is still low, the
dispatched session samples a handful of the most recent real transcript hits found by
the same scan that counted the evidence, and dispatches one fresh-context judge per
sampled hit before Reflecting.

- Fresh context: the judge receives the artifact's source and the real transcript
  excerpt around the sampled use ONLY. It must NOT read `votes/`, any transcript beyond
  that excerpt, or any other vote.
- It grades harshly, strictly, critically, constructively: a grade (letter or 0-10) plus
  an open-ended vote on where to adjust the artifact and what it is lacking.
- It submits ONLY via the script — never by editing files. The first line of the vote is
  the exact `prompt_version` (the artifact's current definition commit, computed as in
  "usage evidence" below) at judging time, so a later Reflect pass can retire the vote
  once the artifact's definition has since changed:

```sh
printf 'prompt_version: %s\n%s\n' '<current short sha>' '<open-ended vote text>' | \
  python3 skills/ai-author/scripts/submit_vote.py --artifact <name> --grade <grade>
```

The script is append-only and never returns existing votes, so blindness holds by
construction. Aggregation across votes is a separate later pass; no judge ever sees it.

## GEPA loop

Run per artifact, on demand or once logs/votes accumulate:

1. **Reflect**: compute the artifact's current `prompt_version` with the command in
   "usage evidence" below — still needed to identify frontier candidates and to filter
   votes, though it no longer filters usage evidence directly. Scan real Pi session
   transcripts (bounded, parent sessions only) for `read` tool_call hits on this
   artifact's own definition path, keeping only hits after the time cutoff defined in
   "usage evidence" below; for each surviving hit, read the actual transcript excerpt
   around it — not a self-reported summary, since none exists — to see what happened.
   Apply the vote filter to `votes/votes.jsonl`: keep only votes whose `vote` text starts
   with `prompt_version: <current value>`; a missing or different first line is stale,
   counted and dropped. Then read the surviving transcript excerpts + surviving votes,
   AND this artifact's own `evals/frontier.jsonl` if present — the archive of every
   candidate ever tested for it, score vector attached, that step 2 below reads to pick a
   mutation parent. Build a failure histogram — which criteria fail most, which
   complaints repeat — from this pass's own reading; there is no persisted place to
   record it for a later pass to trust instead (no `TUNING.md`), so each Reflect pass
   rebuilds it fresh from the real evidence rather than trusting a prior pass's claim.
2. **Propose**: targeted mutations aimed at the top failure modes (sharpen the trigger,
   add a skip-when, tighten a step — or widen a trigger the logs show never firing).
   Small, named, one concern each. Every mutation states whether it is PROSE, a CHECKER, or
   a PI EXTENSION, against question one above. A mutation a checker could enforce ships as the checker,
   because the failure it answers already survived the prose telling an agent not to do it. A narrowing mutation names the logged false positive
   it answers; none logged → don't narrow.

   **Pick the mutation parent from the Pareto frontier, not always the incumbent.** Compute
   the frontier from `evals/frontier.jsonl`: a candidate (the incumbent always counts as one)
   is ON the frontier if no other archived candidate's score vector beats-or-ties it on every
   case with at least one strict win — i.e. it's the best-known scorer on at least one case,
   even with a lower mean overall. When the frontier has **≥2 non-incumbent members** (the
   incumbent plus 2 or more others — below that, behave exactly as before and mutate from the
   incumbent), weight parent selection toward whichever frontier member owns the most
   uniquely-best cases, load its text from `evals/frontier/<candidate_id>.md`, and mutate
   from there instead of the incumbent. This changes only which text step 2 starts from —
   it never changes what step 4 ships; the incumbent is still the only thing that can become
   the live artifact, gated by the unchanged rule below.
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

   **If the candidate was tested via `evals/run.sh` and accepted**, mark it as this
   loop's final action — flip that run's `candidate_id` (printed by `run.sh` to stderr)
   to `accepted:true` in place in `evals/frontier.jsonl`, per the `jq` one-liner in
   `templates/eval-harness.md`'s "frontier.jsonl" section. Never re-run `run.sh` just to
   set this: that re-grades every case through the judge for zero new information.

There is no Record step. `evals/frontier.jsonl`'s score-vector archive is the only
durable record a Decide leaves behind — what was tested and whether it won, not the
narrative reasoning; there is no `TUNING.md` to write that to. When `gepa-due`
dispatches this loop, the dispatched session continues straight from Decide to pushing
its branch and opening a PR without merging: the human sign-off gate is reviewing and
merging that PR, not any pause earlier in the loop.

## applying frontier data (once an artifact has it)

What to actually do, in order, the next time you tune an artifact that has
`evals/frontier.jsonl` on file — nothing here is a new mechanism, it's the same four
steps above with frontier data folded in at the points it actually matters:

1. Reflect (step 1) already reads `evals/frontier.jsonl` if present — nothing to do here
   beyond the loop's existing Reflect.
2. Propose (step 2) already samples its mutation parent from the Pareto frontier once the
   artifact has ≥ 2 non-incumbent frontier members; below that, it mutates from the
   incumbent exactly as before. Nothing to choose here either — it's automatic once
   enough real candidates exist.
3. Test (step 3): run `evals/run.sh <candidate>` with NO `--holdout` flag. This grades
   both slices and appends a frontier line + candidate text automatically, whether the
   candidate wins or loses — `--holdout` alone skips this and should only be used for a
   quick recheck, never for a run feeding a real Decide.
4. Decide (step 4): apply the unchanged holdout-gating rule. If it accepts, run the
   mark-accepted `jq` one-liner (above) against that run's `candidate_id` — the only new
   action frontier data adds to Decide. This is the loop's last step (no Record).

Pruning `evals/frontier.jsonl` past 20 entries per artifact (drop-oldest-dominated, per
the template) is rare enough at current volume to stay a manual "do it next time you're
in there" instruction — not yet worth its own script.

Fence: the mutation-proposer never writes `evals/` cases, the rubric, or `votes/`. The
exam stays out of the student's hands — and dispatching someone else to type it does not
satisfy the fence if they can see the answer. **A mutation is not accepted on cases authored
with sight of that mutation's text.** The case author gets the failure histogram and the
reproduction; never the candidate diff, never the mutated file. Otherwise every EXPECT
paraphrases the new wording, the ablation stub scores 0 by construction, and the "win" measures
nothing but the grader agreeing that the text says what it says. This is a weaker rule than
banning self-tuning, and deliberately so: self-tuning is not the failure mode, answer-key cases
are, and those are open to every author.

## usage evidence (no self-reported logging)

No artifact writes a usage log. `tools/gepa-due` and any dispatched Reflect pass derive
usage evidence directly from real Pi session transcripts under `~/.pi/agent/sessions/`
(parent sessions only, bounded scan): a `read` tool_call whose `arguments.path` matches
this artifact's own definition file counts as one use.

`prompt_version` is still computed the same way — the short commit of the last change to
the files this artifact loads, excluding its own harness and vote history:

```sh
git -C ~/Documents/agents log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/votes/**'
```

It identifies which definition a frontier candidate or a vote was tested/cast against —
it no longer filters usage evidence, since a transcript hit carries no `prompt_version`
field. Usage evidence is filtered by time instead: only hits after `max(this commit's
timestamp, the reviewed_through timestamp gepa-due last recorded for this artifact)`
count as current. See `workflows/gepa-due/README.md` for the full cutoff mechanics and
`tools/gepa-due`'s own implementation.

**Known limitation, stated rather than solved**: a `read` tool_call on this artifact's
definition path can't distinguish "this was the active skill guiding the turn's work"
from an incidental read (a sibling artifact reading it while authoring, a human asking
what it says without using it). This is accepted, not disambiguated — false positives
inflate the usage count somewhat rather than being filtered out.
