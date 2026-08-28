# tool-author rubric

Score 0-10. Grade harshly. Grade the plan the candidate produces against EXPECT.

- 9-10: correct destination, correct one-line justification, file layout matches the
  named section's convention exactly (including tests and any wiring step), the
  registration/wiring step is named when one exists.
- 5-8: correct destination, but drift in the layout, the test command, or the wiring
  step (e.g. named the file but skipped `config/tools.toml` / `install.sh` / the
  `"hooks"` key entry).
- 1-4: wrong destination but the plan is still code-shaped (e.g. checker instead of
  extension, or vice versa), or the destination is right but the plan invents a
  structure this skill never describes.
- 0 / catastrophic (untradeable):
  - The plan authors a sibling that duplicates an existing checker/extension/hook
    instead of extending it, when the situation names or clearly implies one already
    exists.
  - The plan proposes a GEPA(Genetic-Pareto prompt evolution) eval harness
    (`cases.jsonl`/`rubric.md`/blind judge) for the checker/extension/hook itself,
    rather than a test suite.
  - The plan writes new tooling logic in Python or shell where the logic is real
    computation (parsing, scanning) rather than orchestrating other processes.
  - The plan proposes a checker/hook as a hard gate for something the situation
    describes as requiring taste or judgment, with no "review signal, not a gate"
    framing.
  - The plan skips the destination-picking test entirely and jumps straight to a file
    layout.
  - The plan for a Pi extension puts real decision logic directly in the wiring file
    with no separate testable unit (policy.ts, or the Rust binary), making it
    untestable without the full ExtensionAPI.
