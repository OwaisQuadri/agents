---
name: scheduled-ideation
description: Use when a daily-scheduled kickoff prompt (from the scheduled-ideation launchd job) needs to fan out candidate generation across skills, agents, workflows, checkers/linters, and pi extensions worth authoring plus existing public tools worth adopting — filter the results with a fresh critic, and write one markdown digest. Skip for an on-demand, single-question brainstorm with the user present (that's ideate), and skip for evaluating one already-identified external tool in depth (that's capability-adoption).
---

# scheduled-ideation

The fan-out-then-filter topology over ai-author's own bounded session evidence sweep
(reused, not duplicated), the default session model, and web-research-summarizer.
Runs unattended, so it never asks the user anything — a fresh critic stage stands in
for the human-in-the-loop check that `ideate` gets for free by being interactive.

## GRAPH SPEC

```
workflow

GOAL:     propose a reviewable batch of candidates across six categories — skills,
          agents, workflows, checkers/linters, and pi extensions worth authoring,
          plus existing public tools worth adopting — writing one markdown digest
          for a human to review later, never auto-filing anything. A mining
          dispatch's own candidate reports whichever of the five build-it
          categories ai-author's "should it exist?" type tree actually concludes,
          not a fixed skill-or-agent bucket.
FAN OUT:  a plan node designs exactly 2 mining dispatches (skill-evidence-sweep,
          agent-candidate-scan — both reusing skills/ai-author/SKILL.md's bounded
          session evidence sweep procedure by reference, run on the default session
          model — the founding version ran these on the lighter-weight Explore
          agent, which two real 2026-08-28 live runs both aborted mid-task against
          Pi's session-transcript directory, unrelated to content size) plus 1-3
          tool-radar dispatches (run on web-research-summarizer, angle/source
          rotated daily so consecutive runs don't repeat the same search) — one
          combined parallel wave
MERGE:    plain code collects every dispatch's raw candidate array — no model, zero
          tokens
VERIFY:   a fresh-context filter agent, never having seen the generating dispatches'
          own reasoning, scores every raw candidate on evidence strength, relevance,
          and actionability, and drops anything unmeasured or vague — the direct
          answer to 2026 reporting on AI-generated noise overwhelming reviewers
RULE:     every candidate's evidence is a measured fact (a real repetition count, a
          real cost, a real URL fetched that run) — never an estimate; ai-author's own
          "do not estimate" rule, inherited by reference
CAP:      2 mining + 3 tool-radar dispatches; digest capped at 10 survivors
ON FAIL:  any dispatch that returns nothing is named in the report by label, never
          dropped silently; zero raw candidates is a valid, honestly-reported result
SAVE:     returns the digest text; the caller (the seeded kickoff prompt's Pi session)
          writes it to .context/scheduled-ideation-digest.md — this workflow has no
          filesystem access of its own
REPORT:   digest markdown + candidates array + expected vs returned counts + missing
          dispatch labels + raw-vs-survivor counts
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
survivorCount }` — `digest` is the final markdown, grouped under "## Skills worth
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

## logging

At the end of a use, append ONE JSON(JavaScript Object Notation) line to this
artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-08-28T15:00:09-0400>","artifact":"scheduled-ideation","trigger":"<what fired it, e.g. daily launchd 3pm run>","excerpt":"<candidate counts, dispatch labels, filter verdict>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md' ':(exclude)**/logs/**' ':(exclude)**/votes/**'`.
- `ts` is the machine's current local timezone with offset
  (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user
  analyzes these against their own day.
- The excerpt is the relevant parts only — dispatch labels, counts, the filter
  verdict, any human correction. Never the full digest; cap ~2KB per line.
