# phase 23 — close

JOB: the diff conformant to the comment whitelist and code style, fresh-reviewed, statuses closed, PR(Pull Request) open
IN:  state.json.gates.D recorded (phase 22's gate cleared), the full branch diff, docs/comment-style.md, docs/code-style.md, tasks.json, roadmap.json, signoff.md; phase 22 committed
OUT: clean diff, statuses updated, open PR URL

## steps

1. audit the comments. List every comment in `git diff <default-branch>...HEAD`. Only the four whitelisted shapes survive: inexpressible concept or architecture, standard-violation exception, deliberate TODO, and advanced math. Delete everything else, and make the code explain itself. Then sweep mechanically: zero `TODO(<TICKET>` markers remain. Done when both pass.
2. audit the style against docs/code-style.md, covering is-prefixed booleans and the rest. Apply the /simplify doctrine: small refactors, renames, and prunes only. Behavior changes are out of scope here. Done when the diff reads like the surrounding code.
3. audit the prose. Invoke /byline on the commit trail and on any README, changelog, or doc the branch touched. The PR body goes through it in step 7 before it is submitted. Facts stay verbatim through the pass. Done when `skills/byline/evals/check.py` returns zero FAIL lines on each edited piece.
4. [FRESH] dispatch code-reviewer with repo_path + diff_range = `<default-branch>...map/<ID>`. The dispatch must NOT carry this session's transcript, a self-summary, or ANY stash or checkout instruction, because repo modification by the reviewer is catastrophic in its own rubric. Criticals → fix and re-dispatch once. A second Critical round goes to the human. Done when the review returns no Criticals.
5. close the statuses. The tasks go to done in tasks.json. The ticket goes to resolved in roadmap.json, and the NEXT run's phase 01 flips it to done after the merge — that is the resolved/done distinction. Done when both files are updated.
6. HUMAN GATE E. The branch stays local until this gate clears, and nothing is pushed before it. Present the diff stat, the commit count, the target branch, the review verdict, and any finding accepted rather than fixed, with its reasoning. Then STOP. Rationale: the push is the run's first outward-facing act. It publishes the work, it triggers CI(continuous integration), and it notifies collaborators. /create-pr does not gate on its own: it stops only on an empty diff or on a rejected push. Everything before this phase is local and rewindable. This is not. Done when the human says push and `state.json.gates.E` records it. A push before this gate is the map's one unrecoverable act. Force-push is banned skill-wide, so a premature or wrong push cannot be retracted, only added to. If it happens, STOP, tell the user what reached the remote, and let them choose the remedy. Never quietly push and report the PR as though the gate had cleared.
7. invoke /create-pr. It commits the pending work, pushes, and opens or updates the PR under its own hard rules: no attribution lines, and never force-push. The signoff.md bullets go into the PR body. Done when the PR URL is reported. The commit trail ends `map(<ID>): phase 23 close`.

## blame tags

`style-pass-broke-behavior` `whitelisted-comment-deleted` `task-left-not-done` `PR-diff-mismatch` `pushed-unreviewed`
