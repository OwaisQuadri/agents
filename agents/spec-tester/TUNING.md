# spec-tester — tuning history

## 2026-08-25 — temp containment and fields recognised by content

Two mutations. ONE: `scratch_dir` owns the process temp environment; commands using
`mktemp -d` on Darwin get an explicit `-p <scratch_dir>`, and the resolved path must remain
under the scratch root. TWO: a required field counts as present when its content arrives
under a recognisable label variant such as `ticket summary`; `missing input:` is reserved
for content that is actually absent.

Evidence: 12 logged runs escaped scratch via Darwin `mktemp -d`, 10 of them role failures.
Nine more declined a complete brief over `ticket summary` versus `ticket_summary`, while
seven other runs accepted equally informal labels — one role, two contradictory parsers.

HARNESS RESULT: WIN on both mutations, same fixtures and blind-authored cases.

    case                    incumbent   candidate
    s8 temp containment         0           8
    s9 field by content         2           8
    non-holdout mean          5.78        7.33

Every pre-existing non-holdout case s1-s7 stayed at 8. The builder recorded substantial
run-to-run variance (s8 0,0,8 while sharpening two grader bugs), but the final grader was
proved against deliberate bad outputs and both final comparisons separate cleanly.

PATH USED: harness win. These are the only two mutations from the 2026-08-24 agent sweep
that beat their incumbents on a discriminating exam. Accepted.

Side findings, NOT mutations in this entry: s11 scored 2 `column-trusted`, and holdout s10
scored 2 `harness-invented`. Those cases target proposals not shipped in this pass.
