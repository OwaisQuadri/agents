# phase 01 — setup

JOB: leave exactly one ticket in progress with its run dir, branch, and unlock reasoning on the record
IN:  `.map/roadmap.json` (bootstrapped here on first run); no other ticket `in progress` unless this is a resume or take-over
OUT: roadmap.json status flips; `.map/<ID>/state.json` created; branch `map/<ID>` checked out

## steps

1. bootstrap when roadmap.json is missing: look for a connected task system — Linear MCP(Model Context Protocol) tools, `gh issue list`, TODO or BACKLOG files — and offer to import selected items as tickets (ask the 2-4 letter prefix once, uppercase); nothing found → ask the user to create the roadmap. HUMAN GATE either way. Done when roadmap.json parses with prefix + next_nnnn + tickets.
2. close out the past: every `resolved` ticket whose `map/<ID>` branch is merged into the default branch (`git branch --merged`, `gh pr list --state merged`) flips to `done`. Done when no merged ticket is still resolved.
3. pick: run `skills/task-graph/scripts/next-ticket.sh .map/roadmap.json`. Accept the pick, or override with a one-line reason stored in `state.json.next_override`. Needs-replan tickets (cancelled dep) go to the user, never auto-selected. Done when one ticket is chosen with its unlock count known.
4. open the run: set the ticket `in progress`; create `.map/<ID>/` and state.json (`phase: 1`, empty ledgers); create or check out branch `map/<ID>`. Take-over (branch exists, map never ran): create the run dir, then follow the take-over entry point instead of re-initializing. Done when dir + branch + pointer exist.
5. HUMAN GATE A: present the chosen ticket, its unlock reasoning, and any override. Commit `map(<ID>): phase 01 setup`. Done when the user has seen the pick and the commit exists.

## blame tags

`wrong-task-selected` `stale-roadmap-state` `duplicate-run-dir` `resume-at-wrong-phase`
