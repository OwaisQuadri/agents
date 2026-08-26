# Tuning

## 2026-08-26 domain-escape defect

- Reproduction: the user rejected quota and status ideas for a footer slot, but the skill returned quota, status, and delivery ideas.
- Accepted repair: name the active domain, treat the slot as a container rather than a domain, and require three external domains when the user asks for unrelated ideas.
- Evidence: `status-footer-domain-escape` was authored before this repair and checks the missing boundary.

## Open list

- Get a fresh-context blind judge vote after the repair.
