# phase 23 — close

JOB: the diff conformant to the comment whitelist and code style, fresh-reviewed, statuses closed, PR(Pull Request) open
IN:  the full branch diff, docs/comment-style.md, docs/code-style.md, tasks.json, roadmap.json, signoff.md; phase 22 committed
OUT: clean diff, statuses updated, open PR URL

## steps

1. comment audit: list every comment in `git diff <default-branch>...HEAD`; only the four whitelisted shapes survive (inexpressible concept or architecture, standard-violation exception, deliberate TODO, advanced math) — everything else is deleted and the code made to explain itself. Mechanical sweep: zero `TODO(<TICKET>` markers remain. Done when both pass.
2. style audit against docs/code-style.md (is-prefixed booleans and the rest); apply the /simplify doctrine — small refactors, renames, prunes only; behavior changes are out of scope here. Done when the diff reads like the surrounding code.
3. [FRESH] dispatch code-reviewer with repo_path + diff_range = `<default-branch>...map/<ID>`. The dispatch must NOT carry this session's transcript, a self-summary, or ANY stash or checkout instruction — repo modification by the reviewer is catastrophic in its own rubric. Criticals → fix and re-dispatch once; a second Critical round → human. Done when the review returns no Criticals.
4. close statuses: tasks → done in tasks.json; the ticket → resolved in roadmap.json (the NEXT run's phase 01 flips it to done after merge — that is the resolved/done distinction). Done when both files are updated.
5. invoke /create-pr — it commits pending work, pushes, opens or updates the PR under its own hard rules (no attribution lines, never force-push); the signoff.md bullets go into the PR body. Done when the PR URL is reported. Commit trail ends `map(<ID>): phase 23 close`.

## blame tags

`style-pass-broke-behavior` `whitelisted-comment-deleted` `task-left-not-done` `PR-diff-mismatch`
