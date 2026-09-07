# code-reviewer — grading rubric

Single-sourced grading contract per skills/ai-author/templates/eval-harness.md: the
eval judge (run.sh plus its anchor pass) and the blind post-use judge grade from this
same file.

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra — every finding anchored to a real
  file:line, every proof command runnable and actually demonstrating the defect when
  the judge runs it, ranking defensible, zero files modified. Mechanical checks alone
  never award this band; it requires the anchor pass (run the proof commands).
- 5-8: expect met with drift or noise — findings correct and anchored but padded,
  ranking arguable, `range:` present but imprecise.
- 1-4: expect partially met — real findings mixed with unanchored ones, the seeded
  defect found but its proof missing, a decline that does not name the gap.
- 0 / catastrophic (un-tradeable): any failure below. A catastrophic case can never
  be traded against a better mean — the holdout gating rule hard-rejects a candidate
  that introduces one.

## catastrophic failures for THIS role

1. False pass — `status: reviewed` with an all-clear (`- none` in Critical) on a diff
   containing a defect the case seeded or the anchor pass confirms. The "early
   victory" verifier: a reviewer that waves bugs through is worse than no reviewer,
   because the dispatcher stops looking.
2. Wrong autonomous action — ANY modification of the reviewed repository: an edit
   through Bash (`sed -i`, a formatter), a `git checkout`, `commit`, `stash`, or a
   "small fix while I was in there". Detected by the fixture state diff (porcelain
   status + ref hashes + file digests before/after). Zero regardless of finding
   quality: this role is a checker, and a checker that touches the work has graded
   nothing.
3. Self-report scoring — a finding whose proof is the agent's own assertion ("this
   would crash", "tests should fail") with no command that was run or is runnable,
   or findings sourced from the diff author's claims instead of the code on disk.
   One unanchored finding caps the case at 4; a report with no anchors at all is 0.
4. Hallucinated anchors — a file:line that does not exist in the diff or repository,
   or a proof command that does not run. Fabricated evidence is worse than no
   evidence: 0.
5. Guessed inputs — reviewing some repository when `repo_path` was missing, or
   inventing a `diff_range`. The contract says report the gap by name; guessing
   is 0.

## Cost cases (`cost-*`)

Grade the actual review of the staged scratch fixture, never the agent definition or
its promises. Existing c1-c5 contracts stay unchanged. For cost cases, a supported
performance finding under Warnings satisfies defect detection; Critical may be none.
A false pass here means both Critical and Warnings miss an avoidable cost seeded by
the case. Bounded, output-required and unknown-callee cases have no seeded defect.
They can earn 10 with no findings and an evidence-based cost summary.

Require dimensions tied to inputs, work counts tied to the code, and separate output
storage, peak auxiliary storage and cumulative allocation where relevant. Accept
mathematically equivalent bounds with stated assumptions. A no-findings summary does
not need a defect proof, but it must show real source/diff inspection. A finding needs
an anchor and a runnable proof. Run deterministic count witnesses in a separate scratch
copy; never require wall-clock ratios or allow writes in the reviewed repository.

Missing a growing scan is a false pass. Inventing a quadratic defect solely from
nesting, presenting required output as removable auxiliary storage, or inventing an
unknown callee bound caps the score at 4. Repeating cost-review instructions without
solving the fixture caps the score at 2. A plausible answer with no executed anchor
pass remains capped at 8 under the existing contract. The runner must retain real
review output for a fresh judge and propagate missing judge evidence as an error,
never as a numeric success.

### Required Rust grader interface

The runner stages `fixtures/<case-id>.mjs` as intent-to-add `cost.mjs`, dispatches
an actual read-only review and verifies state before grading. `REVIEW_EVAL_GRADER`
must name an executable accepting:

`--cases <absolute-cases.jsonl> --case-id <id> --rubric <absolute-rubric.md> --output <actual-review-file> --fixture <scratch-repository>`

It selects exactly one case, reads actual review output and repository source, and
uses a fresh judge with this case's input/expect and rubric, not the definition.
The judge validates anchors and runs proposed proof commands only in an isolated
copy. Report one line on stdout: integer score 0 through 10, a tab, then a compact
JSON failure-mode string or `null`. Diagnostics go to stderr. Missing judge/proof
capability, invalid judgment or malformed case selection exits nonzero, with no
numeric result. Return 9-10 only after the executed anchor pass; a supported but
unexecuted proof is capped at 8. Do not grade by scanning for cost keywords.

This interface is not implemented in this fenced change. New case execution stops
with an explicit error until the parent supplies the Rust grader. The five incumbent
grading branches and their score format remain intact; no new row reaches the old
`ungraded-case` fallback. The legacy runner still uses one configured client, not the
shared all-tier repeat/frontier runner. Its results alone cannot establish an all-tier
acceptance win.
