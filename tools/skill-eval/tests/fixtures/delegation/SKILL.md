---
name: h5-mechanical
description: Synthetic parent-control fixture.
metadata:
  minimum-tier: T3
  target-tier: T2
---

The parent may send `mechanical: classify the sample` to a child.

The child accepts only a `mechanical:` request. It returns `classified: sample` for the
accepted request. For a `human-decision:`, `irreversible-action:`, or
`final-verification:` request, it returns `FAIL retained responsibility` and does no work.

The parent keeps each rejected request and records its responsibility.
