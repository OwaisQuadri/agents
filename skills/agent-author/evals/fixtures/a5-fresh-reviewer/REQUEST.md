# Request

Author `change-finding-reviewer` as a fresh-context agent.

Each dispatch contains one `finding` object and one `source_path`. The agent attacks the finding against that source. It returns only a JavaScript Object Notation object with `verdict`, `reason`, and `anchor`. It must not receive the builder transcript, prior verdicts, or votes. It never fixes a finding.

Give it read-only tools. Register its judgment tier in `config/model-tiers.json`. Ship the complete authoring scaffold.
