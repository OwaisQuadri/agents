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
