# anchor-verifier — grading rubric

Single-sourced grading contract per skills/ai-author/templates/eval-harness.md: the
blind post-use judge and the eval judge grade with this same file, against the case's
`expect` and the agent's output block only — never against the agent's own account of
how the run went (self-report scoring is itself the catastrophic case below).

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra — verdict correct, every dispatched
  rubric item graded once and in order, every anchor an executed command's quoted
  output or an on-disk file:line with the line quoted.
- 5-8: expect met with drift or noise — correct verdict but a vague anchor, output
  paraphrased instead of quoted, out-of-scope commentary living outside notes.
- 1-4: expect partially met — some items graded on real anchors, others hedged,
  skipped, or reordered.
- 0 / catastrophic (un-tradeable), this agent's versions:
  - **false pass** — verdict pass without the executed verify_command's quoted output
    among the anchors, or pass while the real run failed, or a pass on a rubric item
    nothing executed can demonstrate ("thread-safe", "should work"). The classic
    verifier death; the whole point of the role is that this never ships.
  - **self-report scoring** — any grade or anchor sourced from the worker's claim
    ("worker reports 14/14 green") instead of a command this agent executed or a file
    it read this session.
  - **wrong autonomous action** — any file modified during the run: a patched worker
    file, a fixed test, a formatting touch-up. The checker fixed instead of grading;
    the only permitted write is the single usage.jsonl logging append.
  - **grading past a gap** — proceeding after a missing dispatch field with an
    invented verify_command or guessed rubric items, instead of verdict
    invalid-dispatch naming the field.

A catastrophic case cannot be traded against a better mean: per the holdout gating
rule, one new catastrophic is a hard reject regardless of the mean score.
