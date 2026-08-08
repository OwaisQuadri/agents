# phase 18 — final-implement

JOB: the shelved implementation restored onto the invariant-hardened plan, every remaining TODO implemented, zero markers left
IN:  state.json.stash, tasks.json (now including invariant tasks); phase 17 at fixpoint
OUT: full implementation on the branch; stash consumed; tasks resolved

## the three checks — before anything is consumed

1. message: `git stash list --format='%gd %s'` contains the recorded label.
2. SHA: that entry's `git rev-parse` equals `state.json.stash.sha`. Labels can collide across attempts; the SHA cannot.
3. content: `git stash show --include-untracked --stat <ref>` touches only files inside the union of tasks' `files` arrays, and no `.map/` path. Use the untracked-aware form, because a plain `--stat` is blind to exactly the untracked files phase 16 shelved.

On any mismatch or ambiguity, such as two candidates or a mismatched parent, STOP. Recover from the recorded `state.json.stash.backup_branch`, where `git diff $P12 <backup_branch>` is the loss-proof reference. Then ask the human. A blind pop is this phase's catastrophic. These checks run in THIS session. Never delegate them to code-reviewer or anchor-verifier with stash instructions, because a stash or a checkout by the reviewer is catastrophic in its own rubric.

## steps

1. run the three checks, then `git stash apply <ref>`. Apply, and never pop: the entry survives until the merge is proven. Resolve conflicts per task, using tasks.json and deviations.jsonl as the guide. Done when `git status` is conflict-free and the build passes.
2. `git stash drop <ref>` only now. Delete the recorded backup branch only at this phase's DONE. Done when the drop is logged in state.json.
3. implement every remaining TODO, which is the phase-17 delta, through the phase-13 graph on the residual DAG(directed acyclic graph). Done when the residual tasks are resolved.
4. verify three gates. `grep -rn "TODO(<TICKET>" --exclude-dir=.map` returns zero hits. Every `./todo.sh list` step has been executed, and an idempotent re-run is legal. The phase-14 suite is green through FRESH spec-tester dispatches, because this session resolving conflicts and then grading its own resolutions is self-verification. Commit `map(<ID>): phase 18 final-implement`. Done when all three gates pass.

## blame tags

`todo-left-undone` `wrong-stash-popped` `conflict-mis-resolved`
