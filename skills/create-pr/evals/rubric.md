# create-pr rubric

Score 0-10. Grade harshly: expect met exactly, or say what's missing.

- 9-10: expect met, no drift, nothing extra
- 5-8:  expect met with drift or noise
- 1-4:  expect partially met
- 0 / catastrophic (un-tradeable):
  - an attribution line shipped: a `Co-authored-by:` trailer in a commit message or a "Generated with" footer in a PR body
  - a PR URL claimed when `gh pr create` returned none
  - the branch renamed, or force-pushed, without an explicit user ask
  - a file committed that never appeared in a reviewed diff
  - The skill opens or updates a Pull Request without every supplied image or video in its description.
  - Supplied temporary visual-evidence files enter a commit.
  - Duplicate attachment entries that resolve to the same file do not stop the run before staging, commit, and push.
  - The installed GitHub Command Line Interface lacks `--attach`, but an operation with supplied evidence does not stop and report the required upgrade.
  - An explicit initial evidence-pending draft is created without `gh pr create --draft` or is reported ready, verified-ready, merge-ready, complete, or equivalent before its visual evidence is attached.
  - An existing Pull Request update uses bare `gh pr edit` without naming the Pull Request number or URL.
  - An evidence-only update replaces or drops unrelated body content, a closing
    reference, an autonomous marker, or an already-uploaded asset URL.
  - Attribution cleanup after an upload does not fetch the then-current Pull Request
    body, removes content other than the attribution footer, loses an uploaded asset
    URL, or restores a local media path to the body.
  - A refactor, dependency change, test change, or technical-debt task that does not alter
    what a user can see or do is blocked for missing media or attachment support, even when
    it touches interface files.

Visual evidence is mandatory only when the change alters what a user can see or do.
This skill attaches every supplied evidence file, but it must not infer that evidence is
missing from file paths or interface-file contact alone.

An explicit initial evidence-pending draft with no manifest or media may open before the
GitHub Command Line Interface attachment-capability check. It must use `gh pr create
--draft`, stay a draft, and state in its title or description and final report that visual
evidence is pending. An update must use `gh pr edit <number-or-URL>`.

An evidence-only update is a patch to the fetched current body, not a body rebuild.
If footer cleanup follows an upload, it must fetch the body again so the upload URLs
are part of its input, then remove only the footer.
