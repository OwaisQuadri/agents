# Installed artifact inventory

`skills/task-graph/SKILL.md` frontmatter:

> Use when turning work items with dependencies into a validated graph — the task graph for one ticket's implementation plan, or `ABCD-NNNN` tickets filed into a project's roadmap.

Its node contract also names `roadmap.json` as an input and output shape. Its steps assign ticket identifiers, validate dependencies and cycles, update the roadmap file, and report the resulting waves.

No separate roadmap-ticket artifact is present in the installed inventory.
