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
