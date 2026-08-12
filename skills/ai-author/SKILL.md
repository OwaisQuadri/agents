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
one — so name the destination: this file's `## history` under a deferred heading, or a ticket in
the relevant project's roadmap when the verdict is project work. Either one is somewhere a later
pass reads without being told to look, which is the whole property. Sweep every verdict the pass
reached, not only the most recent.

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

1. **Reflect**: read `logs/usage.jsonl` + `votes/votes.jsonl`, AND the artifact's own
   "open, measured, not yet fixed" list if it has one. Build a failure histogram — which
   criteria fail most, which complaints repeat — and record it in the usage line with the vote
   indices it came from, so the next pass can recompute it from `votes.jsonl` instead of
   trusting this one. A blind judge cannot open `votes/`, so an asserted count is unverifiable
   by the only fresh reader the pass gets. An open list nothing re-reads is a dead letter, which
   is why it is an input here.
2. **Propose**: targeted mutations aimed at the top failure modes (sharpen the trigger,
   add a skip-when, tighten a step — or widen a trigger the logs show never firing).
   Small, named, one concern each. A narrowing mutation names the logged false positive
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
5. **Record**: note the accepted mutation and its rationale in the artifact so history is
   auditable and reversible.

Fence: the mutation-proposer never writes `evals/` cases, the rubric, or `votes/`. The
exam stays out of the student's hands — and dispatching someone else to type it does not
satisfy the fence if they can see the answer. **A mutation is not accepted on cases authored
with sight of that mutation's text.** The case author gets the failure histogram and the
reproduction; never the candidate diff, never the mutated file. Otherwise every EXPECT
paraphrases the new wording, the ablation stub scores 0 by construction, and the "win" measures
nothing but the grader agreeing that the text says what it says. This is a weaker rule than
banning self-tuning, and deliberately so: self-tuning is not the failure mode, answer-key cases
are, and those are open to every author.

## history

GEPA step 5 says to note an accepted mutation and its rationale "in the artifact", and until
2026-08-10 this file had nowhere to put one — the step was unexecutable against ai-author
itself, which is the same self-violation as shipping with no `evals/`. This section is that
place.

- **2026-08-10, arrival routing** (`## what arrived?`). Driven by the vote histogram, not by a
  hunch: five of the first six blind votes are the same complaint, that an invocation which was
  not a fresh authoring job got walked into the should-it-exist tree anyway and the operator
  improvised a verdict the tree never sanctioned. Vote 1: always-on guidance falls through to
  rule 3 and would wrongly become a skill. Vote 3: rule 1 was stretched over a BUILT-IN, which
  is not an authored artifact and cannot be updated. Vote 5: "declared modes are exactly three"
  and an artifact-plus-a-live-task is none of them. Vote 2: a deferred verdict lived only in a
  log line. Vote 6: no update-an-existing-artifact branch. **UNMEASURED.** This entry first
  claimed a stub score of 5.50 against 7.80 as a measured win; the next blind judge (grade 5)
  showed the claim is circular and it is retracted. The cases were authored AFTER the mutation
  shipped, by an author who could read it, so each EXPECT paraphrases these bullets and the
  ablation stub scores 0 by construction. The mutation stands on the vote histogram and on the
  reproduction, which is the defect-fix path, and it is not entitled to a number.
- **2026-08-10, the defect-fix acceptance path** (GEPA step 4). Step 4 assumed every mutation
  is a tuning tweak the harness can see. For a defect fix whose effect no case measures, a tie
  is the expected result, so the rule made "reject" the only compliant outcome and shipping
  meant misreporting a tie as a win — which is exactly what happened, and what the grade-7 vote
  caught. The path now names itself: reproduction plus execution evidence, with a fenced case
  author dispatched in the SAME pass. **UNMEASURED**, retracted for the same reason as the
  entry above: `c2` and `c3` were written with the new step 4 in view.
- **2026-08-10, the deferred-verdict destination.** The routing mutation above forbade the
  destination that does not count (a log line) and named none that does. Two independent signals
  said so: the fenced case author flagged it while writing `d1`, and the post-mutation run scored
  `d1` at 5 against 7, tagged `tracking-destination-unnamed`. That is "forbid the state, never
  mandate the shape" taken one step too far — here the reader needs a place to put the thing. Now
  names two.
- **2026-08-10, the harness itself.** ai-author had no `evals/` at all while telling every other
  artifact "No harness = not done". A fenced case author built 15 cases, 5 held out, and proved
  discrimination against eight surgical defect stubs. Two of those stubs (a sibling-authoring
  licence, and a sighted judge) RAISE the visible mean while a holdout case catches them, which
  is the holdout slice paying for itself twice on independent branches.

- **2026-08-10, the answer-key fence** (GEPA loop). The judge's own recommended fix, taken over
  its alternative of banning self-tuning, because the weaker rule catches more: self-tuning was
  not the failure mode, cases written with sight of the candidate were, and that is open to
  every author. Dispatching someone else to type the exam no longer satisfies the fence if they
  can see the answer.

### deferred verdicts

Verdicts this skill reached but did not execute. The routing section names this heading as one
of the two destinations that count. This heading tracks a verdict parked here. Nothing tracks a
verdict left in a log line.

- **2026-08-11, propagate the session preamble to the symlink target.** The run routed at
  branch 1, authored no artifact, and shipped `## every session is on the record` into the
  repo `CLAUDE.md`. A dispatched subagent then listed all 11 pre-change H2 headings and not
  the new one. That result proves two things. Subagents do receive `CLAUDE.md`, and a
  workspace edit alone does not reach them. The section reaches background sessions only after
  the branch merges and `~/Documents/agents/CLAUDE.md` refreshes. Owais asked for background
  sessions by name, so this half of his ask stays unverified until that run happens. Owner:
  the merge of this branch.
- **2026-08-11, branch 1 names three destinations and no rule for picking one.** The routing
  section sends always-on guidance to "CLAUDE.md, memory, or settings" and stops. The operator
  had to invent the whole comparison, then credit it to the skill. That comparison ran on three
  facts the skill never supplies. A static string needs no process. A hook earns its keep on
  dynamic content. The installer never symlinks the live `settings.json`, so a hook edit lands
  in two places and drifts. A blind judge graded the run 5 and named this the
  gap it exposes. The fix belongs in the routing section. It needs the GEPA loop plus a fenced
  case author, so this entry does not write it.
- **2026-08-11, step 4 clause 3 fights step 4's own same-pass case author.** The defect-fix
  path demands a fenced case author in the SAME pass. Clause 3 demands the win hold on the
  holdout slice. Put both in one pass and the holdout gate answers itself, because the case
  author adds a holdout case for that defect. The mouthpiece run measured it. Its clean pair
  gained 0.85 over the whole holdout slice. Now split that slice by case age. The 5 holdout
  cases that predate the mutation score 5.40 in both arms. Their per-case scores match
  exactly, at 9, 2, 8, 4, and 4. So the whole gain sits in the 2 holdout cases this same pass
  added. The non-holdout split reads 5.91 against 6.27 over 11 old cases, and 4.75 against
  6.50 over 4 new ones. A blind judge graded that run 5 and named this first. The operator
  then confirmed the split before writing this entry. Step 4 needs one more sentence. Report
  the two arms over the cases that predate the mutation, apart from the new ones. Judge
  clause 3 on the old holdout cases only. `templates/eval-harness.md` carries the same hole.
  Owner: a GEPA pass on this file with a fenced case author, because the fix is a mutation.
- **2026-08-11, step 3 never freezes the instrument.** It says "incumbent vs candidate on the
  same cases" and it names nothing else. The mouthpiece run graded its two arms with two
  different checker builds, and that pair read backwards. Reading the messages found the
  cause, which was 8 false positives, and the operator then re-ran both arms under one build.
  The skill asked for none of that recovery. `evals/run.sh` also falls back to a second model
  and records nothing about which model graded which case. Step 3 should name one checker
  build, one rubric revision, and one grader model. It should also drop every number that
  predates a change to any of the three.
- **2026-08-11, the narrowing licence says "logged" and it should say "observed".** Step 2
  reads "A narrowing mutation names the logged false positive it answers". The mouthpiece run
  narrowed three times on false positives seen in harness output, not in a log line. The
  strict reading licenses none of the three, and the strict reading is the wrong one, because
  harness output beats a log line as evidence. The weakest fix is one word.
- **2026-08-11, an accepted mutation's evidence lives where git cannot keep it.** The
  mouthpiece run wrote its per-case output, its 88 graded messages, and its reproduction text
  under `.context/`. The repository excludes that directory. It excludes `logs/` and `votes/`
  as well. So step 1 loses its stated property, that a later pass recomputes the histogram
  instead of trusting it. A blind judge could check every number in that run, and only from
  files no clone holds. This is `c1` with a sharper edge, and it outranks the gate field.
- **2026-08-11, the live clone carries a deferred verdict this file does not.** Commit
  `3c26536` sits unpushed in `~/Documents/agents`, and it touches 11 files including this
  one. Its copy records that `find_words` misses a quoted term, so a writer can evade every
  word ban. The mouthpiece run borrowed a rule that runs on `find_words`, and it never saw
  that entry. The judge proved the hole is live. The line `The word "idempotent" is banned in
  this register.` passes both borrowed rules. Step 1 should read the artifact's deferred list
  from the merged copy before it proposes anything. Owner: the merge of `3c26536`.
- **2026-08-11, the `gate` field has no documentation.** The `## open, measured` list calls a
  gate field in the logging format the highest-value next mutation. The 2026-08-11 run emitted
  one ad hoc rather than adding it to the format. That leaves c1 open while it looks closed. A
  format change is a mutation, so it routes through the GEPA loop, and this entry defers it.

### open, measured, not yet fixed

The new harness scores the incumbent at 7.80 on both slices, and it does not sweep its own
exam on purpose. The live gaps it measures:

- `c1` scores **4**: the contract states the go-live condition and never requires the evidence
  to be reported, so "the evals were authored" reads as "the evals passed". Vote 4 asked for a
  gate field in the logging format in July. Highest-value next mutation, and the pass that named
  it then committed it — reporting bare means with no pass count and no holdout line, which the
  new rubric grades catastrophic. Fix belongs in the logging format as a required field.
- The holdout gate was never shown met for either 2026-08-10 mutation. Step 4 requires the win
  to hold on the holdout slice; both entries reported non-holdout figures only, and one of them
  quoted `a4`, a holdout case, inside a non-holdout claim. Under step 4 as written neither
  mutation was acceptable on numbers, which is the second reason both are now marked unmeasured.
- Tree rule 1 ends at "update it" and never routes to the GEPA loop, though the arrival branch
  routes that same state there. `b4` sits at 7 for it. One line.
- `h1` at 6 and `b4` at 7: branches this file states but does not make checkable.
- The one-sentence test, vote aggregation, the loop's trigger threshold, and "no churn on
  noise" are all unobservable as written — no run can be said to have honoured or skipped them.
- The deferred-verdict rule names the destination that does NOT count and never names one that
  does.
- ~~This file's own `## logging` is not its last section.~~ FIXED 2026-08-10, after the judge
  called out authoring the rubric that condemns it while not fixing it. The move initially
  corrupted the file — the naive search for `## logging` matched the contract's own code-block
  comment first and spliced the judge protocol into it — which is worth remembering as the
  cheapest possible edit still needing a diff read afterwards.

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
