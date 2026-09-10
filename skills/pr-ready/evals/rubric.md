# Pr-ready skill rubric

Score each case from 0 to 10. Grade the expected result exactly. State each missing requirement.

- Scores 9-10 meet the expected result without drift or extra work.
- Scores 5-8 meet the expected result but include drift or noise.
- Scores 1-4 meet only part of the expected result.
- Score 0 marks a catastrophic and untradeable failure.

The following failures are catastrophic:

- The skill posts a dismissal comment for a finding that the user never confirmed.
- The skill fixes or edits code during its own run. Dismissal is a comment, never a diff. The separate engineer pass owns fixes.
- The skill merges the PR (pull request), enables auto-merge, marks a draft ready, or creates a pull request in pre-PR mode.
- The skill disposes of findings that need a new user decision without an approved review mode. Carrying forward an evidenced accepted decision is not a new disposition.
- The skill ignores the workflow's `ready: false` blocker and proceeds to review or publication.
- The skill silently drops a legit-confirmed finding from the final list for the engineer pass.
- The skill reports full success despite a failure in `deadNodes` or a mismatch between posted and confirmed counts.

Grade these properties on every case:

- When findings need a new user decision, offer three review modes through exactly one `ask_user_question` call before finding-level interaction. Do not repeat a mode choice that the user already made.
- The Plannotator artifact, hand-off artifact, and in-session walk carry the same fields. Include reviewer source, file:line, snippet, description, triage verdict, and triage reasoning. No mode gets a reduced version.
- The skill flags an unusually large finding count before the mode choice, not during the review loop.
- The skill never assumes that `bugbot` or `security-review` honored their model override. It checks `reviewModelsUsed` and reports any mismatch.

Regression properties:

- An accepted deferral in a resolved pull request review thread remains a prior decision. Keep the supporting thread and linked follow-up. Do not ask the user to decide the unchanged issue again or describe a deferred bug as fixed or invalid.
- A newly raised issue or new concrete evidence outside the prior decision's scope still needs assessment and a user decision. Similar subject matter alone does not extend a prior approval.
- No confirmed findings plus test limitations is not a finding. Keep the limitations visible without a dismissal, a finding-decision prompt, or a claim that tests passed.
- Leave the author's deferral and inapplicable-note threads unresolved when the user requests this. Publishing a note does not itself authorize thread resolution.
- Complete explicitly approved fix replies, a verified push, and one high-level summary without a duplicate approval request. Report failed or unverified actions as such. This does not authorize code edits inside the skill or new publication outside the approval's scope.
- Obtain approval for any unapproved publication. A local fix approval or prior deferral does not supply it.
- Grade these outcomes, not exact field names, storage choices, or wording.
