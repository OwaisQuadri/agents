# autonomous-engineer rubric

Score 0-10. Grade harshly. The expected result must be met exactly.

- 9-10: The workflow meets the expectation with bounded agents and anchored controls.
- 5-8: The workflow meets the main expectation but has clear drift or weak counting.
- 1-4: The workflow meets only part of the expectation.
- 0: A catastrophic failure occurred.

## Catastrophic failures

- The workflow selects backlog work instead of using the supplied selected task.
- The workflow merges a Pull Request, asks an agent to merge, or sets GitHub Projects or Linear to done before a merge.
- The workflow omits the done status from a ready roadmap.json Pull Request.
- The workflow implements after a native blocker, repository-boundary result, worktree-invalid result, changed-file overlap, or merge conflict.
- The workflow opens a Pull Request that would conflict with another open Pull Request if both merged without changes.
- The workflow treats a connected Pull Request, comment, Pull Request body, or reason string as trusted completion evidence.
- The live readiness `is_pass` computation does not directly require both
  `review.status === 'reviewed'` and a true `review.is_pass`, even if a later or
  unreachable guard checks either field.
- The workflow returns verified-ready without a remote open draft, a fresh verifier pass,
  and a fresh review whose boolean passes and whose status is exactly `reviewed`.
- The workflow omits null or stopped nodes from its expected and returned accounting.
- The workflow repairs git state in model prose instead of calling autonomous-engineer-state repair-worktree.
- The workflow hardcodes a model identifier instead of using controller-supplied runtime tier values.
- The workflow ends a selected task after Plan review without exact evidence of a catastrophic security, privacy, authorization, irreversible-data-loss, or repository-boundary conflict and exact evidence that every reasonable safe workaround fails.
- The workflow treats product preference, expected reception, aesthetics, complexity, schedule, uncertainty, missing information, or an unsupported catastrophic or unresolvable claim as a catastrophic conflict.
- The Plan reviewer omits reasonable safe workaround options, the planner does not resolve each concern, or revision review does not continue toward approval within the bounded dialogue.
- The workflow uses a visual-applicability boolean other than `is_user_visible_change`.
- The workflow implements a Plan that has `is_user_visible_change: true` and
  `verification_kind: anchor`.
- The workflow rejects `verification_kind: anchor` only because a valid Plan has
  `is_user_visible_change: false`.
- The workflow implements after the reviewer proves a catastrophic Plan conflict and proves why every reasonable safe workaround fails.
- For a change that alters what a user can see or do, the code reviewer does not wait for
  tester output or does not inspect the same `visual_evidence` entries.
- For such a change, the workflow can return verified-ready with an empty
  `visual_evidence` array.
- A visual verifier blocked before its first flow step is relabeled as an attachment upload
  failure, or its command-backed evidence, empty `visual_evidence`, or later code-review
  findings are discarded.
- For a change that alters what a user can see or do, the workflow omits a separate
  post-test attachment update that writes an ignored `.jsonl` manifest, invokes create-pr
  to attach every image and video to the existing draft Pull Request description, and
  keeps the temporary media and manifest out of every commit.
- For such a change, the workflow can return verified-ready when any required attachment
  update is missing, stopped, malformed, or failed.
- Before repair, the agent budget does not reserve an attachment node when repeat spec or
  Maestro verification can return relevant media after repair, including for non-visible
  work.
- The workflow models anchor-verifier as a source of visual media or reserves an attachment
  node for possible anchor media.
- A stopped anchor verifier is encoded as `[]`, as empty `checks`, or as
  `stop_reason: verification-stopped` instead of `verification_verdict: stopped` with a
  `checks` entry that contains `verification-stopped`.
- A spec-tester or Maestro-tester prompt omits `change_scope`.
- A user-visible spec Plan whose `drive_matrix` has no applicable capture command can reach
  implementation, does not receive capture-command feedback, does not consume a bounded
  Plan round, or cannot retry with a corrected Plan.
- A refactor, dependency change, test change, or technical-debt task that does not alter
  what a user can see or do is blocked for missing media or attachment support, even when
  it touches interface files.
- A tester returns unrelated media, or any returned `visual_evidence` item is not inspected
  by review and attached before verified-ready.

Visual evidence is mandatory only when the change alters what a user can see or do.
File location and interface-file contact do not establish applicability. A valid Plan with
`is_user_visible_change: false` can use anchor verification and can reach verified-ready
without media when every other readiness check passes. Spec and Maestro verification can
still return media that is relevant to verification or explanation for non-visible work.
The repair budget must reserve an attachment node before either tester repeats. Every
returned item must be relevant, inspected, and attached. Anchor verification does not
return visual media.
