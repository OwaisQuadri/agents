---
name: create-project
description: >-
  Use when starting a new project from a goal and the user wants discovery,
  current research, repository structure, a GitHub Projects board, and a
  production roadmap created after approval. Supports software and non-software
  projects. Skip when a project repository already exists, when the user wants
  ideas only, or when the user only wants to publish an existing folder.
metadata:
  minimum-tier: T3
  short-description: Research, plan, and create a new project
---

# create-project

JOB: Turn one new-project goal into an approved, researched project that is ready for roadmap execution.
IN: A goal for a new project with no existing local project folder or remote repository.
OUT: Create the approved repositories, one shared GitHub Projects board, and one prioritized roadmap with at most ten GitHub Issues.

## hard limits

- Support every project type. Never assume that the project needs software.
- Ask one focused question at a time. Never bundle unrelated decisions into one question.
- Research every project before proposing its structure or roadmap.
- Create no project resource before the final approval.
- Write no application code or starter implementation.
- Stop on a name collision. Never select another name without approval.
- Never delete a partial resource without separate approval.

## steps

1. **State the goal.** Ask for the intended outcome when the user did not give one. Restate the goal in one sentence. Ask the user to correct it. Done when the user confirms the sentence.

2. **Define production.** Identify the target user and the first real use by that user. Treat production as that specific event. Do not substitute deployment, publication, or repository creation. Done when one testable sentence defines production.

3. **Interview adaptively.** Ask only questions that can change the project boundary, research, repository structure, or first ten roadmap steps. Cover target users, success, monetization, countries, legal duties, privacy, hardware, platforms, integrations, open-source intent, budget, operating cost, skills, timeline, security, and maintenance when relevant. Ask follow-up questions until each relevant answer is specific. Record unresolved points instead of guessing. Done when no unresolved answer can change the proposal.

4. **Research current facts.** Dispatch `web-research-summarizer` with only its four input fields. Give `objective` one self-contained research question that covers every decision-changing factual gap. Put exclusions in `boundaries`. Put preferred authorities in `source_guidance`. Put the freshness requirement in `recency`. Never send the conversation, a plan, or a draft. Use an authoritative primary source for each critical claim. Use a second independent source when the primary source does not settle the claim. Record each source URL and access date. Ask new questions when a finding changes a prior decision. Done when every decision-changing factual claim has support or appears as a named gap.

5. **Choose the project shape.** Propose a project name and repository slugs. Use one repository unless separate ownership, visibility, release, or operating boundaries justify more. For multiple repositories, select one primary repository and one common owner. Use one shared board. Match its visibility to the primary repository. Use a private board when the primary repository is internal. Ask a clarification question only when a shape decision remains unresolved. Step 7 owns the approval. Done when the complete proposal can include the shape without a guess.

6. **Draft the roadmap.** Create one to ten steps that reach the confirmed production event. Add limited post-launch work only when the project needs it within this roadmap. File component work in its repository. File cross-project work in the primary repository. Add a dependency only when one step needs another step's result. Give every issue one priority: Urgent, High, Medium, or Low. Put the necessary project context and cited research in each relevant issue. Adapt the detail, but make each issue executable without another discovery pass.

Use this issue body shape when each section adds useful information:

```markdown
## Outcome
<one observable result>

## Scope
<work included and excluded>

## Acceptance checks
- <checkable result>

## Dependencies
<earlier roadmap steps, or None>

## Context
<decisions that this issue needs>

## Sources
- <source title, URL, and access date>
```

Done when the roadmap has at most ten issues, every issue has a repository and priority, and every dependency points backward.

7. **Show the complete proposal.** Read `byline` before writing text that will enter the repositories or GitHub. Show the production sentence, research findings, gaps, local layout, repositories, primary repository, owner, visibility, licenses, and board. Show the full issue text and each planned side effect. Ask one final approval question. Create nothing when the user declines or changes the plan. Done when the user approves the exact proposal.

8. **Create the approved project.** After approval, read `create.md` and follow it exactly. Stop when the approved plan and the validated creation manifest differ. Done when every required round-trip check passes or one named failure stops the run.

9. **Report the result.** Return the local paths, repository details, board details, issue URLs, priorities, and every check result. Include each URL and visibility. On partial failure, list each resource that exists and the failed operation. Ask whether to resume or clean up. Done when the report distinguishes verified resources from missing resources.

## evals

`evals/run.sh` grades the discovery, research, approval, scope, and failure rules against `evals/cases.jsonl`.
Run `evals/run.sh` without a flag for both the normal and holdout slices.
