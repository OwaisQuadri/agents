#!/bin/zsh
# TODO(AGNT-0032.T34): make the session-stats exam execute real artifact behavior
# ./run.sh — grade the non-holdout functional cases against the fixture store.
set -euo pipefail
cd "$(dirname "$0")"
REPO_ROOT="$(git rev-parse --show-toplevel)"
CRATE="$REPO_ROOT/tools/session-stats"

cargo build --release --quiet --manifest-path "$CRATE/Cargo.toml"
JSON="$("$CRATE/target/release/session-stats" \
  --claude-dir fixtures/claude \
  --pi-dir /nonexistent --codex-dir /nonexistent --cursor-db /nonexistent \
  --json -)"

python3 - "$JSON" <<'EOF'
import json, sys
rows = json.loads(sys.argv[1])
def grade(case_id, ok, detail):
    print(json.dumps({"id": case_id, "score": 10 if ok else 0, "detail": detail}))
    if not ok:
        sys.exit(1)

grade("fixture-shape",
      len(rows) == 1 and rows[0]["src"] == "claude" and rows[0]["model"] == "claude-test-1"
      and (rows[0]["input"], rows[0]["output"], rows[0]["cacheRead"], rows[0]["cacheCreate"]) == (110, 90, 6000, 500)
      and (rows[0]["firstCtx"], rows[0]["lastCtx"]) == (1300, 5310),
      json.dumps(rows)[:300])
grade("dedup", rows[0]["messages"] == 2, f"messages={rows[0]['messages']}")
grade("synthetic-model", all(not r["model"].startswith("<") for r in rows), "no synthetic rows")
grade("analysis-not-transcripts", True, "process case; graded by the blind judge from the transcript")
EOF
echo "all functional cases pass" >&2
