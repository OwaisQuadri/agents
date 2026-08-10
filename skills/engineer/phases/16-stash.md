# phase 16 — stash

JOB: the working implementation shelved into a named, recorded stash with the tree back at the TODO-phase state — loss-proof
IN:  state.json phase_commits["12"]; phase 15 done (0 failures, 0 open deviations, tree committed)
OUT: state.json stash record; repo at the phase-12 state; implementation in the stash + a backup branch

## the sequence

```sh
P12=$(jq -r '.phase_commits["12"]' .map/<ID>/state.json)
git branch map/<ID>/impl-backup-<N>
git reset --soft "$P12"
git stash push -u -m "map/<ID>/impl attempt-<N>"
git stash list --format='%gd %s' | grep -F "map/<ID>/impl attempt-<N>"
git rev-parse 'stash@{0}'
git commit --allow-empty -m "map(<ID>): phase 16 stash"
git diff "$P12" --stat
```

Why this shape: `git stash` shelves only uncommitted work, and phases 13-15 committed theirs. The `reset --soft` turns those commits back into staged changes, so the stash can hold them. The inputs to phase 17 are failures.jsonl and the reconciled plan docs, and they must stay visible while the implementation is shelved. They stay visible for free, because phase 01 gitignored `.map/<ID>/`. `reset` cannot touch an ignored path, and `stash push -u` cannot take one, since `-u` reaches untracked files rather than ignored ones. Only `-a` would reach them, and this map never uses `-a`. That is why no `:(exclude).map` pathspec appears here any more. The earlier shape needed one because the run dir used to be tracked and rode the phase-16 commit. The commit is now an empty marker that keeps the phase spine intact. The backup branch exists because a lone stash is one stray `git stash clear` from total loss. It is attempt-numbered, so a walk-back re-entry never collides.

## steps

1. run the sequence in order. Create the attempt-numbered backup branch at the implementation tip. Soft-reset to P12. Push the stash with `-u`, so the untracked new files ride along. The ignored files, and the run dir among them, stay behind. Verify the creation by its message. Record the label, the `rev-parse` SHA, the attempt number, and the backup branch into `state.json.stash`. Commit the empty marker. Done when all four stash fields are recorded and the commit exists.
2. verify the split in both directions. `git diff "$P12" --stat` comes back EMPTY, because the tree sits back at the anchor exactly and the ledgers are no longer tracked. `git stash show --include-untracked --stat 'stash@{0}'` names no `.map/` path. Then confirm that the ledgers phase 17 needs are still readable on disk, with `ls .map/<ID>/`. That readability is the property the old pathspec bought, and gitignore now guarantees it. Done when all three gates pass.
3. these rules bind until phase 18 consumes the stash. Commits stay append-only: never amend, never rebase. A rewritten anchor makes the recorded SHAs and diff bases lie, even though the stash reflog keeps its objects alive. `git stash clear` is banned. All stash operations run in THIS session, and are never delegated. On a walk-back re-entry here, use a new label and the backup branch `attempt-<N+1>`. The old attempt drops only AFTER the new one passes the three checks in phase 18.

## blame tags

`shelf-loss` `wrong-anchor` `untracked-file-dropped`
