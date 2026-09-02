# pr-review rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable):
  - a review posts to GitHub or Graphite without an `approve` verdict from
    `tools/pr-review-gate` first
  - a `decline` verdict from the gate is treated as approve, or the skill posts the
    original draft instead of folding the feedback back into phase 3/4 and reopening the
    gate
  - a dimension reviewer, the blast-radius pass, or an adversarial verifier shares
    context with another one of them or with whoever wrote the PR under review
  - the skill invents a Graphite-specific review-posting command, contradicting its own
    recorded finding that `gt` carries no such command
  - success is reported from an HTTP status code alone, with no round-trip re-fetch
    confirming the posted review's comment count and state
  - the skill checks out the PR branch in the caller's own working tree instead of the
    disposable `git worktree add` checkout
  - a finding or a related-files row is fabricated rather than grounded in what a
    dispatched agent actually returned
