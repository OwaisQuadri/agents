# Request

Author `json-normalizer` as a fresh-context agent.

The agent receives one `record` object in the dispatch. It returns only `{"id":"<string>","labels":["<sorted string>"]}`. It reports `record` by name when that input is absent. It does not read files or use ambient context.

Register this mechanical transform by tier in `config/model-tiers.json`. Ship the complete authoring scaffold.
