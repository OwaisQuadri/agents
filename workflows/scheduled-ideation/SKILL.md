---
name: scheduled-ideation
description: Use when a daily-scheduled kickoff prompt (from the scheduled-ideation launchd job) needs to find the highest-impact levers to pull on this workspace and AI setup — fan out candidate generation across skills, agents, checkers/linters, and existing public tools, check every finding against what the repo already has, rank survivors by leverage, and write one markdown digest. Skip for an on-demand, single-question brainstorm with the user present (that's ideate), and skip for evaluating one already-identified external tool in depth (that's capability-adoption).
---

# scheduled-ideation

The fan-out-then-filter topology over ai-author's own bounded session evidence sweep
(reused, not duplicated), the default session model, and web-research-summarizer.
Runs unattended, so it never asks the user anything — a fresh critic stage stands in
for the human-in-the-loop check that `ideate` gets for free by being interactive. The
fixed daily mission is not an exhaustive catalog: it's finding the highest-impact
levers to pull on this workspace and AI setup, so the filter stage owns comparing
every finding against the repo's actual current state and ranking what survives by
leverage, not evidence quality alone.

## GRAPH SPEC

```
workflow

GOAL:     find the highest-impact levers to pull on this workspace and AI setup right
          now, across five categories — skills, agents, workflows, checkers/linters,
          and pi extensions worth authoring, plus existing public tools worth
          adopting — writing one impact-ranked markdown digest for a human to review
          later, never auto-filing anything. A mining dispatch's own candidate
          reports whichever of the five build-it categories ai-author's "should it
          exist?" type tree actually concludes, not a fixed skill-or-agent bucket.
FAN OUT:  a plan node designs exactly 3 mining dispatches — skill-evidence-sweep,
          agent-candidate-scan, and correction-mining — for parent sessions active in
          the last 24 hours. The first two reuse `skills/ai-author/SKILL.md`'s bounded
          session evidence sweep by reference. One fixed prompt-snippet-audit runs
          beside all three in the same parallel wave. The audit
          inspects current prompt snippets, at most 500 direct-use records from the
          latest 30 days, and grep-first bounded windows across at most 200 parent
          sessions active in that period for equivalent manual requests. Direct and
          manual evidence stay distinct; absence is unknown;
          adds require two parent sessions; merge/removal requires measured co-use,
          conflict, or existing-skill overlap; weak evidence returns zero candidates.
          The shared wave THEN feeds 1-3 tool-radar dispatches in a second wave — a
          genuine barrier because tool-radar receives the complete mining evidence as
          grounding text.
MERGE:    plain code collects every dispatch's raw candidate array — no model, zero
          tokens
VERIFY:   a fresh-context filter agent, never having seen the generating dispatches'
          own reasoning, does two jobs over the whole raw set at once (a genuine
          barrier: ranking needs every candidate together, not one at a time) — (1)
          checks each candidate with real read/grep/bash access against this repo's
          actual current implementation and open GitHub issues, dropping anything
          already built or already tracked; (2) scores what survives on evidence
          strength, relevance, and actionability, drops any tool candidate whose
          rationale ignores the grounding block or reads as generic "seems useful"
          noise, THEN ranks every remaining survivor by actual leverage (recurring
          cost removed × how often the friction recurs), highest-impact first — the
          direct answer to 2026 reporting on AI-generated noise overwhelming
          reviewers, and to a candidate list with no sense of which item matters most
RULE:     every candidate's evidence is a measured fact (a real repetition count, a
          real cost, a real URL fetched that run) — never an estimate; ai-author's own
          "do not estimate" rule, inherited by reference. Mining evidence is not
          limited to artifact usage logs: a marker that greps 2+ times across the
          session window (a recurring warning, a repeated manual workaround, a
          repeated human correction) is measured evidence too, since it's a counted
          occurrence, not a guess. A candidate already built or already an open issue
          never survives Filter, regardless of how well-evidenced its rationale is.
CAP:      3 planned mining + 1 fixed prompt-snippet audit + 3 tool-radar
          dispatches; audit reads at most 500 latest-30-day direct-use records and
          bounded windows from at most 200 latest-30-day parent sessions; digest
          capped at 10 survivors
ON FAIL:  any dispatch, including prompt-snippet-audit, that returns nothing is named
          in expected, returned, and missingLabels accounting; zero raw candidates is
          a valid, honestly-reported result
SAVE:     returns the digest text; the caller (the seeded kickoff prompt's Pi session)
          writes it to .context/scheduled-ideation-digest.md — this workflow has no
          filesystem access of its own
REPORT:   digest markdown (leading with a "Top lever today" section) + candidates
          array in rank order + expected vs returned counts + missing dispatch labels
          + raw-vs-survivor counts. Audit output contains conclusions and aggregate
          evidence only: never prompt or response text, raw records, transcript
          excerpts, session identifiers, or session paths. Audit candidates can reach
          the private Filter and digest, but never the web-research dispatches.
```

Anchors: every mining candidate's evidence traces to a real logged repetition/cost
(never estimated, per ai-author's own rule); every tool-radar candidate's source is a
URL fetched that run; the filter stage's dropped-count is reported, never silently
absorbed.

## input contract

Run via the Workflow tool, normally with no args (the daily 3pm trigger calls it bare):

```
Workflow({ scriptPath: "<repo>/workflows/scheduled-ideation/scheduled-ideation.workflow.js",
           args: { focus: "<optional steering note for the tool-radar angle>",
                    max_tool_radar: 3 } })
```

- `focus` — optional. A one-off manual run can steer which external-tool angle the
  plan node emphasizes this run (e.g. "focus on Rust tooling"). The scheduled daily
  run passes no args at all.
- `max_tool_radar` — optional cap on planned tool-radar dispatches, clamped to 3.

## output contract

`{ candidates, digest, expected, returned, missingLabels, rawCandidateCount,
survivorCount }` — `expected`, `returned`, and `missingLabels` include the fixed
`prompt-snippet-audit`; its candidate output excludes prompt and response text, raw
records, transcript excerpts, session identifiers, and session paths. `digest` is the
final markdown, grouped under "## Skills worth
authoring" / "## Agents worth authoring" / "## Workflows worth authoring" /
"## Checkers/linters worth building" / "## Pi extensions worth building" / "## Tools
worth trying" headings (a heading is omitted entirely when it has zero survivors).
`candidates` is the same survivor set as structured data, for a caller that wants to
act on it programmatically instead of reading prose.

## why a workflow, not a skill

Per `~/.agents/skills/ai-author/SKILL.md`'s type-decision rule 5 ("fans out over ≥2
agents, loops over items, or has a generate→judge shape → workflow"): 2024-2026 prior
art for this exact shape (agents-radar's 10-parallel-source daily digest,
ArXiv-Research-Monitor-Agent's per-item relevance scoring, the tech-radar
nomination→triage→decision-trail rubric) all converge on fan-out-then-filter, not a
single linear recipe one agent could follow start to finish. No existing artifact
owned the combination either: ai-author's own sweep is skill-only and evidence-only;
`ideate` needs a live user to grill; `capability-adoption` writes up one
already-identified candidate rather than discovering new ones.

## install (macOS)

The trigger mechanics live outside this workflow entirely — a mechanizable shell script
(`scripts/trigger.sh`) plus a launchd plist, per ai-author's "can a program do it?"
rule. Mirrors `skills/hq/launchd/com.owaisquadri.hq.plist`'s exact install pattern:

```sh
cp /Users/owaisquadri/Documents/agents/workflows/scheduled-ideation/launchd/com.owaisquadri.scheduled-ideation.plist ~/Library/LaunchAgents/
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.owaisquadri.scheduled-ideation.plist
launchctl kickstart gui/$(id -u)/com.owaisquadri.scheduled-ideation
```

`launchctl kickstart` fires one immediate run (covers "run once now"); the plist's own
`StartCalendarInterval` (Hour=15, Minute=0) then fires it daily at 3pm without a repeat
`kickstart`. Uninstall is `launchctl bootout gui/$(id -u)/com.owaisquadri.scheduled-ideation`.
Linux (Arch PC) cron wiring is a deferred fast-follow — see `TUNING.md`; `scripts/trigger.sh`
itself is already OS-agnostic, only the scheduler wiring differs.

## history

- 2026-08-28 founding run, built for issue #124 (superseding #67, which was narrower —
  session-evidence-only, skills-only, no scheduling). Full design research at
  `.context/scheduled-ideation/research.md` in the pi-from-iphone worktree that
  authored this: the 2026 TCC(Transparency, Consent, and Control) unattended-Automation
  risk that looked like the biggest blocker turned out moot, since the launchd trigger
  never drives a GUI Terminal — it calls the already-running herdr daemon's socket API
  directly.
