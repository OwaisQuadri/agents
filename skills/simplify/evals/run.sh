#!/bin/zsh
# TODO(AGNT-0032.T35): make the simplify exam execute real artifact behavior
set -euo pipefail
cd "${0:A:h}"

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
is_holdout = slice_name == "holdout"

with open(skill_path, encoding="utf-8") as file:
    skill = file.read()
with open("rubric.md", encoding="utf-8") as file:
    rubric = file.read()
with open("cases.jsonl", encoding="utf-8") as file:
    cases = [json.loads(line) for line in file if line.strip()]

cases = [case for case in cases if bool(case.get("holdout")) == is_holdout]
case_text = "\n\n".join(
    f"ID: {case['id']}\nINPUT: {case['input']}\nEXPECT: {case['expect']}" for case in cases
)
prompt = f"""Grade these eval cases for a coding skill. Reply with only a JSON array.
Use one object per case in the given order:
[{{"id": "<id>", "score": <integer 0-10>, "failure_mode": "<short tag>" or null}}]

RUBRIC:
{rubric}

SKILL UNDER TEST:
{skill}

CASES:
{case_text}

Would an agent that follows the skill meet each expectation? Grade only behavior required by the skill text.
"""

commands = [
    ["claude", "-p", prompt],
    ["codex", "exec", "--skip-git-repo-check", "--sandbox", "read-only", "-c", "mcp_servers={}", prompt],
]
verdicts = None
errors = []
for command in commands:
    run = subprocess.run(command, capture_output=True, text=True)
    start, end = run.stdout.find("["), run.stdout.rfind("]")
    if run.returncode == 0 and start != -1 and end != -1:
        try:
            verdicts = json.loads(run.stdout[start : end + 1])
            break
        except json.JSONDecodeError as error:
            errors.append(f"{command[0]} returned invalid JSON: {error}")
    else:
        errors.append(f"{command[0]} failed: {run.stderr.strip()}")
if verdicts is None:
    sys.exit("; ".join(errors))

by_id = {verdict["id"]: verdict for verdict in verdicts}
scores = []
for case in cases:
    if case["id"] not in by_id:
        sys.exit(f"grader omitted case {case['id']}")
    verdict = by_id[case["id"]]
    score = int(verdict["score"])
    scores.append(score)
    print(json.dumps({"id": case["id"], "score": score, "failure_mode": verdict.get("failure_mode")}))

if scores:
    print(f"mean {sum(scores) / len(scores):.2f} over {len(scores)} cases ({slice_name} slice)", file=sys.stderr)
else:
    print(f"no cases in {slice_name} slice", file=sys.stderr)
PY
