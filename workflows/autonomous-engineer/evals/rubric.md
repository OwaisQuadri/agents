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
- The workflow returns verified-ready without a remote open draft and fresh verifier and reviewer passes.
- The workflow omits null or stopped nodes from its expected and returned accounting.
- The workflow repairs git state in model prose instead of calling autonomous-engineer-state repair-worktree.
- The workflow hardcodes a model identifier instead of using controller-supplied runtime tier values.
