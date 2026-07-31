#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

slice=nonholdout
if [[ "${1:-}" == "--holdout" ]]; then
  slice=holdout
  shift
fi
skill="${1:-../SKILL.md}"

python3 - "$skill" "$slice" <<'PY'
import json
import subprocess
import sys

skill_path, slice_name = sys.argv[1], sys.argv[2]
is_holdout_slice = slice_name == "holdout"
skill = open(skill_path, encoding="utf-8").read()
rubric = open("rubric.md", encoding="utf-8").read()

cases = [json.loads(line) for line in open("cases.jsonl", encoding="utf-8") if line.strip()]
cases = [c for c in cases if bool(c.get("holdout")) == is_holdout_slice]

scores = []
for case in cases:
    prompt = (
        "Grade one eval case for a skill. Reply with ONLY a JSON object "
        '{"score": <integer 0-10>, "failure_mode": "<short tag>" or null}.\n\n'
        "RUBRIC:\n" + rubric +
        "\nSKILL UNDER TEST:\n" + skill +
        "\nCASE INPUT:\n" + case["input"] +
        "\n\nEXPECT:\n" + case["expect"] +
        "\n\nWould an agent following the skill on this input meet EXPECT? Grade per the rubric."
    )
    out = subprocess.run(["claude", "-p", prompt], capture_output=True, text=True, check=True).stdout
    verdict = json.loads(out[out.find("{"):out.rfind("}") + 1])
    print(json.dumps({"id": case["id"], "score": verdict["score"], "failure_mode": verdict.get("failure_mode")}))
    scores.append(verdict["score"])

if scores:
    print(f"mean {sum(scores) / len(scores):.2f} over {len(scores)} cases ({slice_name} slice)", file=sys.stderr)
else:
    print(f"no cases in {slice_name} slice", file=sys.stderr)
PY
