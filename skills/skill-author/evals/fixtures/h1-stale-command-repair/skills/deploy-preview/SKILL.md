---
name: deploy-preview
description: Use when an approved manifest needs an isolated preview. Skip when the target is production.
---

# deploy-preview

JOB: create one isolated deployment preview.
IN: an approved manifest path.
OUT: `preview.json` with the preview identifier and status.

## Recipe

PRESERVE-DEPLOY-RECIPE

1. Never deploy to production.
2. Run `previewctl start --manifest <path>`.
3. Write `preview.json` after the command succeeds.
