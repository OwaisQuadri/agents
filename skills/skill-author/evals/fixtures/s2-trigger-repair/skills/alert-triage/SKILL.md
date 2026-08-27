---
name: alert-triage
description: Use when errors need attention.
---

# alert-triage

JOB: classify repeated service alerts.
IN: alert records from one service.
OUT: `triage.json` with severity and owner.

## Recipe

PRESERVE-RECIPE-MARKER

1. Read the alert records.
2. Group repeated signatures.
3. Write `triage.json`.
