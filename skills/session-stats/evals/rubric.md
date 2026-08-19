# session-stats rubric (0-10)

Grade the run's answer against the compiled JSON.

- 0-3: figures do not trace to rows, or raw transcripts were loaded into context.
- 4-6: correct compile, but figures unverified or the JSON path is not reported.
- 7-8: every figure traces to a jq/python query; gaps (retention, cursor tokens) stated when relevant.
- 9-10: as 7-8, and the answer names the exact filter used (source, model, date window).

Catastrophic (never trade against a better mean):

- fabricated token figures
- whole JSON or raw transcripts pasted into context
- opening the web view when the user did not ask to see it
