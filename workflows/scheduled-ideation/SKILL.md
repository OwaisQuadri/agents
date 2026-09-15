---
name: scheduled-ideation
description: >-
  Use when the daily scheduled run must select one money-making opportunity that
  fits the owner's current work, interests, projects, and demonstrated strengths.
  It scans bounded internal context and early public market signals, verifies the
  candidates, and returns a digest for human approval. Skip an interactive
  brainstorm, which ideate owns. Skip one known external tool, which
  capability-adoption owns.
---

# scheduled-ideation

Run the workflow and return its result. Answer contract-inspection requests directly from the supplied workflow without tools, file edits, or invented evidence.

When a synthetic case supplies stage results, treat those results as given. Apply the contract without requesting live files or adding a live-run caveat.

For a digest inspection, name every required digest field. Use the exact labels `Recommended action`, `Why now`, `Why the owner can win`, and `Test`.

Also name buyer, paid pain, evidence, contrary evidence, distribution channel, first-buyer path, smallest build, success signal, stop condition, and limitations.

Use this digest-inspection order: identical result, `Recommended action`, two alternatives at most, opportunity fields, coverage, drops, failures, counts, caps, human approval.

The workflow selects one supported market test, not a long idea feed. Active commitments act as owner-fit constraints on new opportunities.

The workflow stops at the digest. It never files work, contacts a buyer, makes a purchase, publishes, or executes a proposed test.

## GRAPH SPEC

```text
workflow

GOAL: Select one supported money-making opportunity test from owner context and early market evidence.

FAN OUT: Run six fixed discovery jobs in one parallel wave.

PARALLEL JOBS:
1. `owner-profile` builds a bounded owner profile from parent sessions and personal memory.
2. `active-commitments` maps issues, pull requests, commits, and project state.
3. `hacker-news` scans Hacker News technical discussion and pain.
4. `design-signals` scans Designer News and named design-publication fallbacks.
5. `broad-news` scans broad technology and business news.
6. `market-corroboration` checks launches, adoption, work demand, spending, and regulation.

MERGE: Plain code collects facts, coverage, failures, and source references.

GENERATE: One internal agent creates at most six opportunities from the complete evidence set.

DEDUPE: Plain code removes exact identifiers before public distribution research.

DISTRIBUTION: One public researcher checks access, entry rules, costs, delay, allowed offers, and commitment signals for every candidate.

VERIFY: Two fresh skeptics review the complete enriched set in one parallel wave.

PARALLEL JOBS:
1. The market skeptic checks timing, direct commercial evidence, maturity, sources, contrary evidence, and unsupported forecasts.
2. The owner-fit skeptic checks skills, interests, commitments, buyer access, distribution evidence, and seven-day scope.

MERGE: Plain code keeps only candidates that both skeptics approve.

RANK: One fresh agent partitions approved identifiers into survivors and drops. Plain code merges duplicate evidence without rewriting the kept proposal.

RULE: A candidate needs three facts from three sources across at least two signal types.

RULE: Owner-fit evidence comes from the `owner-profile` or `active-commitments` discovery job.

RULE: One direct fact shows paid pain, budget, adoption, or buyer commitment.

RULE: A trend claim includes a dated comparison. A high current level alone is not acceleration.

RULE: Every candidate keeps contrary evidence and source limitations.

RULE: A test lasts no more than seven calendar days. Buyer access must fit inside that test window.

RULE: A test names the buyer, pain, offer, channel, first-buyer path, smallest build, success signal, and stop condition.

RULE: At least one `distribution-access` fact must support the initial buyer or channel access claim.

RULE: Views, votes, stars, funding, and model confidence do not prove payment.

RULE: The workflow rejects invented forecasts, inaccessible buyers, and mature markets without a supported wedge.

RULE: The workflow describes proposed tests without instructing the owner to act.

CAP: Six discovery agents, six raw opportunities, one distribution agent, two skeptics, one ranker, at most three survivors, and eleven maximum agents.

CAP: The host inspects at most twelve generator records when a malformed response bypasses the six-record schema cap.

ON FAIL: A missing discovery class, candidate distribution record, or required stage returns `incomplete-research` without a recommendation.

ON FAIL: Evidence identifier collisions, opportunity identifier collisions, and truncated generator output also return `incomplete-research`.

ON FAIL: Complete research with no accepted candidate returns `no-surviving-candidates` without a recommendation.

ON FAIL: An unavailable design source remains visible. Two distinct named fallbacks can complete that source class.

HUMAN GATE: A human approves every filing, contact, purchase, publication, or execution after reading the digest.

SAVE: The caller writes only the returned digest to `.context/scheduled-ideation-digest.md`.

REPORT: Return status, recommendation, at most two alternatives, coverage, drops, failures, counts, caps, and the approval boundary.
```

## Source bounds

The owner-profile job reads ten parent sessions and eight personal-memory results at most. It returns twenty aggregate facts at most.

The active-commitments job reads thirty closed issues, thirty merged pull requests, fifty commits, and two hundred project events at most.

The Hacker News job reviews forty items, twenty comments, and eight linked pages at most. It covers top, new, show, ask, and jobs.

The design job reviews twenty items across six sources at most. It tries Designer News before Sidebar, Smashing Magazine, UX Collective, and design-system news.

The broad-news job reviews twenty items across eight sources at most. It uses at least three independent publishers and applicable first-party sources.

The market job reviews thirty items across twelve sources at most. It covers four source families, including commercial proof and adoption or work demand.

Each discovery job keeps at most thirteen coverage rows. The digest prefixes each row with its producing job.

Private jobs return aggregate facts and references. They never send raw records, transcripts, paths, customer details, or private text to a web agent.

## Distribution rules

The distribution researcher receives opaque candidate keys and public evidence only. It uses twelve public sources at most and returns four fetched public URLs per candidate at most.

It compares direct owner access, paid work marketplaces, developer marketplaces, usable-product launches, partners, buyer-specific outreach, and search or publication.

It records buyer presence, owner access, entry requirements, cost or limits, access days, allowed offer, first-buyer path, commitment signal, and limitations.

Show Hacker News requires a usable project. GitHub Marketplace is a later channel because paid listings require verification and an installation base.

Attention tests channel reach. A payment, deposit, paid pilot, or signed commitment tests demand.

## Result states

`recommendation-ready` means all required stages completed and at least one candidate passed both skeptics and ranking.

`no-surviving-candidates` means complete research returned no candidate that passed every gate.

`incomplete-research` means a required source, distribution record, stage, or validation safety check failed. This state contains no recommendation.

The workflow builds deterministic digest text in plain code. Identical validated inputs produce identical digest text.

## Input contract

The daily run uses exactly one call with no arguments:

```text
SubagentWorkflow({
  scriptPath: "workflows/scheduled-ideation/scheduled-ideation.workflow.js"
})
```

The call uses no arguments. The trigger does not try workflow-name lookup or retry the workflow call.

## Output contract

The output fields include `status`, `recommendation`, and `coverage`.

```text
{
  status,
  recommendation,
  candidates,
  digest,
  coverage,
  expected,
  returned,
  missingLabels,
  rawCandidateCount,
  survivorCount
}
```

The digest opens with one recommended action when the status is `recommendation-ready`. It includes at most two alternatives.

The digest shows buyer, paid pain, offer, timing, owner fit, evidence, contrary evidence, limitations, distribution, the test, coverage, drops, failures, counts, and caps.

Evidence identifiers use at most eight entries per candidate. Other candidate lists use at most four entries, and each text field uses at most one thousand characters.

`missingLabels` can name source or stage labels. It can also name `evidence-id-collision`, `opportunity-id-collision`, or `generator-output-truncated`.

## Install on macOS

Copy the launch agent from `<repo>/workflows/scheduled-ideation/launchd/` to `$HOME/Library/LaunchAgents/`.

Run `launchctl bootstrap gui/$(id -u) $HOME/Library/LaunchAgents/com.owaisquadri.scheduled-ideation.plist` to install the daily job.

Run `launchctl kickstart gui/$(id -u)/com.owaisquadri.scheduled-ideation` for one immediate test run.

Run `launchctl bootout gui/$(id -u)/com.owaisquadri.scheduled-ideation` to uninstall the job.

## Evaluation

`evals/preflight.sh` runs deterministic synthetic cases. `evals/run.sh` grades the behavior cases in `evals/cases.jsonl` against `evals/rubric.md`.
