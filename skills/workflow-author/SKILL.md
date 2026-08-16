---
name: workflow-author
description: Use when authoring a WORKFLOW, a multi-agent graph — writing a new GRAPH SPEC, redrawing a linear chain into parallel jobs, or reviewing a spec for fake edges, self-verification, or missing guards. Output is a GRAPH SPEC block. Skip when the shape is a linear single-agent recipe (skill-author) or a standalone role (agent-author).
metadata:
  short-description: Author multi-agent workflow graphs (GRAPH SPEC)
---

# workflow-author

How to draw a multi-agent graph that runs wide without lying to you.

- skill-author: a linear recipe one agent follows.
- agent-author: a distinct role with its own context, tools, and judgment.

## nodes and edges

A graph is jobs plus waits. Two parts, whole vocabulary:

- node: one job. one agent, one task, one thing in, one thing out.
- edge: one job needs what another produced, so it waits. an edge only counts when real
  data actually passes along it.

Nodes do the thinking. Edges carry the results.

A node is wire-able only with a contract. Author every node as:

```
JOB:     research one competitor's pricing    <- one job, nothing else
IN:      { competitor: "name", url: "..." }   <- passed in, never assumed
OUT:     { price, plan, source, date }        <- fixed shape
SCHEMA:  enforced. free text is rejected and retried
WHY:     a fixed output is what lets the next node read this one
         without a human in the middle
```

A node whose output is a wall of free text is a node only a human can read. Reject it
at authoring time.

## the fake-edge test

Walk every arrow and ask one thing: does this step actually need the RESULT of the one
before it?

- yes -> real edge. keep the order.
- no -> no edge. run them side by side.

"review file A for bugs, then review file B" reads like a sequence, but B never looks
at A's result; that order is just the order it was typed. Expect two or three fake
edges in any chain you redraw. A 40-step line has 40 sequential failure points and the
summed latency of all 40; the same jobs drawn honestly usually have three to five real
dependencies and finish at the speed of the slowest layer.

The model was never the bottleneck. The line was.

## the default shape: the diamond

Fan out, reduce, verify, synthesize. Nearly the only pattern needed.

1. FAN OUT for breadth: one worker per angle/file/item, all at once.
2. REDUCE with plain code, not a model: dedupe, filter, count. Zero tokens.
3. VERIFY with FRESH-context skeptics: each verifier gets the finding only, never the
   worker's chat. Split the skeptics across different lenses — is it correct, is it
   current, is the source real. Three lenses catch what ten identical ones miss.
4. SYNTHESIZE once: one agent writes the answer from the survivors.

Iron rule: a worker and its verifier never share a context. A verifier reading the
worker's chat is nodding along in a different font — one loop grading its own homework,
later and pricier. Verify against a real signal ("does the test pass"), not "did the
agent say done". Cheap model on boring nodes, strong model where judgment lives.

## the three break modes

1. Context collapse: fan out wide, pour every raw output into one final step, blow the
   context window before synthesis starts. Fix: layered fan-in. Batch the results,
   summarize each batch, combine the summaries — never the raw pile.
2. False independence: two nodes look independent because their prompts never mention
   each other, but both write the same file or hit the same rate-limited
   API (Application Programming Interface). That is a hidden edge. Fix: isolate every
   worker (own worktree, own workspace) and audit for shared RESOURCES, not just shared
   data. Any two nodes writing the same file need an edge, not parallelism.
3. Silent node failure: one dead node among two hundred slips into a report that looks
   complete. Fix: every fan-in counts its returns against the number expected and flags
   the gap. Never synthesize on a partial set silently.

## anchors

Topology alone does not buy truth. A graph where every node reads another node's report
can be fully consistent and verify nothing; it fails like a single loop, just later,
more expensively, with more green lights on the way down.

The graph needs anchors — nodes that cannot be argued with:

- tests that actually RAN and passed, not "should pass"
- numbers that cannot argue back: revenue landed, users retained, a link that resolves
- frozen rules: the ones an optimizer would bend to win stay off-limits

Every authored graph names at least one anchor. Judge the run on it.

## when NOT to build a graph

A graph buys breadth, not judgment. Skip it when:

- the task is small or isolated: one function, one bug. coordination is pure overhead.
- every step wants approval: the graph's point is running wide without you.
- exploratory work: steer one agent, don't lock a fleet into a plan.
- steps depend on each other: the graph adds cost for zero speedup.

The tell is the fake-edge test: zero cuts means there is no graph to build. It is a
loop, and a loop is fine.

## output format: the GRAPH SPEC block

Author the workflow as a GRAPH SPEC. The executing prompt starts with the word
"workflow" — that word is what makes Claude build the coordinated fleet instead of a
line of steps. Full vocabulary; use only the lines a run needs:

```
GOAL:          the one outcome, stated so the report can be judged against it
FAN OUT:       what splits and how wide (one agent per file / per angle, in parallel)
PARALLEL JOBS (<batch name>, run at once): numbered jobs; multiple named batches allowed
MERGE:         how results combine (into an outline, into one ranked report)
DEDUPE:        each new find checked against everything already seen
LOOP:          keep going until two rounds in a row find nothing new, then stop
RULE:          a per-finding constraint (every claim needs a source link + date)
VERIFY:        independent checker per finding, fresh context
HUMAN GATE:    where the run stops and waits for a yes
CAP:           hard limit (files this run, total agents) so it can't run away
ON FAIL:       flag any node that doesn't return, never skip it silently
SAVE:          where output lands (drafts/, research-report.md, a named workflow)
REPORT:        what comes back, including how many nodes returned
```

Worked example:

```
workflow

GOAL: audit every route file under src/routes/ for missing auth (authentication) checks

FAN OUT:    one agent per file, all in parallel
VERIFY:     an independent checker on each finding, fresh context
CAP:        20 files on this first run
ON FAIL:    flag any file that doesn't return, never skip it silently
REPORT:     one merged list of routes missing auth, plus how many files came back
```

Non-negotiable lines in every authored spec: VERIFY (fresh context), CAP on the first
run, ON FAIL. A spec fanning out wider than ~40 results also states its MERGE batching
(the layered fan-in from break mode 1).

Write every line of the spec under ~/Documents/agents/docs/prompt-style.md — the
Simplified Technical English (ASD-STE100) rules that leave a sentence one reading. A
GOAL or RULE line with two readings fans out into N agents holding N interpretations.

## cost discipline

A graph costs more than a chat; the coordination gets cheaper, the work does not.

- CAP the first run. run one scoped, watch what it costs, then widen.
- when a run comes out good, SAVE it as a named workflow: one command, re-run by name.

## evals

Copy skills/ai-author/templates/eval-harness.md into
evals/. Cases are graph specs: an input situation plus the spec this skill should
produce, graded on topology properties:

- no fake edges: every ordered pair in the spec actually passes data
- verifier context-isolation: no verifier shares context with the worker it judges
- fan-in guard present: returns counted against expected, gaps flagged
- CAP present on a first run; at least one anchor named

Catastrophic (score 0, un-tradeable, listed in rubric.md):

- a spec that lets a worker verify its own output
- a spec that synthesizes on a silently partial set

## logging

At the end of a use, append ONE bounded JSON (JavaScript Object Notation) line to
skills/workflow-author/logs/usage.jsonl — the relevant transcript excerpt only, ~2KB
cap. Timestamp in the machine's CURRENT LOCAL TIMEZONE with offset
(`date +%Y-%m-%dT%H:%M:%S%z`), never UTC (Coordinated Universal Time): these lines are
analyzed against the user's own day, so UTC timestamps are useless.

```json
{"ts":"2026-07-31T02:45:09-0500","artifact":"workflow-author","trigger":"<what fired it>","excerpt":"<relevant parts only>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```
