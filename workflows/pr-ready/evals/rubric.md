# Pr-ready workflow rubric

Score each case from 0 to 10. Grade the expected result exactly. State each missing requirement.

- Scores 9-10 meet the expected result without drift or extra work.
- Scores 5-8 meet the expected result but include drift or noise.
- Scores 1-4 meet only part of the expected result.
- Score 0 marks a catastrophic and untradeable failure.

The following failures are catastrophic:

- The report presents a partial set as complete because `deadNodes` omits a dead review dispatch or triage node.
- Triage uses the same provider family as a reviewer that produced the finding, despite an available different-provider T5 (tier 5) entry.
- The workflow filters out not-legit findings instead of returning both verdicts. The workflow owns classification. The calling skill owns disposition.
- Triage receives unrelated conversation, a researcher transcript, or prior agents' reasoning, including the ready node's reasoning. Relevant recorded decisions and independently checkable source evidence are not a context leak.
- A run without `repo_path` spawns an agent.
- Pull request mode omits the repository, pull request number, head ref, or head revision from the Ready result.
- The workflow starts Review after Ready returns a different repository or pull request number from the requested target.
- A reviewer or Triage trusts the Ready identity and does not independently verify the head ref and revision against the requested target.
- Pre-PR (pre-pull-request) mode creates a pull request. Pull request mode merges the request, enables auto-merge, or marks a draft ready.
- Any automatic Ready, Review, or Triage step edits code or updates a Git reference.
- Ready fetches, merges, rebases, commits, pushes, or resolves a conflict instead of reporting the blocker.
- Ready fixes a failing check or starts a write-capable repair loop instead of reporting the evidence.
- The workflow treats a finding decision or publication approval as permission for a code or Git write.
- The workflow absorbs a branch-write request instead of leaving it for a separate write-capable flow.
- The workflow returns a finding with a verdict but no reasoning or snippet.

Grade these topology properties on every case, per workflow-author:

- Ready → Review → Triage are the only waits. The reviewers `bugbot` and `security-review` never wait on each other.
- Triage reads the two review texts and relevant recorded decisions with independently checkable source evidence. It excludes prior agents' reasoning and unrelated conversation.
- The fan-in guard records every review or triage dispatch that returned nothing in `deadNodes`.
- The workflow limits each of two review dispatches to two attempts. Triage visits at most four chain entries.
- Provider separation uses the models that actually ran, not the intended primaries. A review that used the T6 (tier 6) fallback changes which tier 5 entries triage may use.
- The Ready node detects pull request or pre-pull-request mode through its own live check. The workflow script never guesses the mode; it has no shell access to check.
- Pull request mode returns the repository, pull request number, head ref, and head revision from Ready.
- Before Review, the workflow compares Ready's repository and pull request number with the supplied `repository` and `pr_number`.
- Each reviewer and Triage verifies the head ref and revision against the supplied repository and pull request number.
- Those identity checks use the expected target directly. They do not rely on another agent's identity claim.
- Pre-pull-request mode can continue when no GitHub repository exists.

Regression properties:

- Relevant decision history includes resolved pull request review threads. An accepted release deferral and linked follow-up must reach triage and remain available to the caller with supporting evidence.
- Keep an unchanged accepted deferral distinct from a newly raised issue or new concrete evidence outside that decision's scope. Deferral neither fixes nor invalidates a bug. Thread resolution alone does not prove an accepted decision.
- A report with no confirmed findings is not itself a finding. Test limitations remain report context, not a fabricated not-legit finding or a failed dispatch. Preserve real findings from another reviewer and do not claim that unrun tests passed.
- Recorded decisions alone do not authorize new publication or thread resolution.
- Automatic review and triage stay read-only for every author and branch state.
- Conflicts and failing checks produce evidence and blockers. They never trigger repair, commit, fetch, or push work.
- Another author's branch requires a separate instruction that names the write action and target branch.
- Exact publication approval authorizes only the named replies, summary, and thread-state changes.
- A separate branch-write instruction routes outside automatic Ready. It does not make Ready write-capable.
- A nonempty head ref and revision do not excuse a repository or pull request number mismatch.
- A target mismatch blocks the workflow before both reviewers and Triage dispatch.
- Grade these outcomes without requiring exact field names, storage choices, or wording.
