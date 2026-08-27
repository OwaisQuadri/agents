# Request

Author `dependency-contract-checker` as a fresh-context agent.

The agent receives `manifest_path` and `dependency_name` in each dispatch. It reads the named manifest and reports one contract mismatch. It never edits files. It returns a JavaScript Object Notation object with `verdict`, `reason`, and `anchor` fields.

Use the minimum tools. Include realistic evaluation cases. Register the agent by tier in `config/model-tiers.json`.
