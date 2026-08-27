# project inspiration

- "what would iron man do": one candidate per cycle is the ambitious version nobody asked for. It must make the current approach look like a prototype.
- Prefer the candidate that deletes a class of future work over the candidate that adds a feature.
- Refactoring follows make it work, make it right, then make it fast. Never file a make-it-fast ticket while a make-it-right debt sits unfiled.
- Steal from the bleeding edge only what at least one shipped product already proves. Cite it in the candidate.
- A candidate that needs a new dependency states the cost of owning it, not only the win.
- Boring beats clever everywhere except the one iron-man slot.

## adopted references

- Honeycomb wide events: keep rich, high-cardinality context together, then reveal and filter it on demand instead of fixing the questions in advance. Adopted for local agent telemetry at Gate UX on 2026-08-17. https://www.honeycomb.io/blog/evaluating-observability-tools-for-the-ai-era

- magit (Emacs) status buffer: one keyboard-driven buffer where files are collapsible sections — TAB unfolds a file into its hunks in place, so the list survives while the patch is read; no view switch. Adopted for the Pi live-diff overlay at Gate UX on 2026-08-19 (AGNT-0015). https://magit.vc/manual/magit/Sections.html

- Braintrust experiment comparison: align identical cases across model experiments, lead with improvement or regression, and keep score, cost, latency, errors, and per-case evidence available beneath the comparison. Adopted for the model qualification staircase at Gate UX on 2026-08-22 (AGNT-0032). https://www.braintrust.dev/docs/evaluate/compare-experiments

- MLflow model evaluation and champion-challenger management: preserve comparable model runs, admit challengers against saved incumbents, and keep specialist capability evidence separate from the general rank. Adopted for the bottom-up model challenger contract at Gate UX on 2026-08-27 (AGNT-0032). https://mlflow.org/docs/latest/ml/evaluation/
