#!/usr/bin/env bash
# run.sh — tool-author eval runner
# usage: ./run.sh [candidate-skill.md]   (non-holdout slice; defaults to ../SKILL.md)
#        ./run.sh --holdout [skill.md]   (held-out slice)
#
# Per case: ask for the PLAN the skill produces (destination picked, file layout, test
# command, wiring step) — never actually write files. Grading the plan rather than a
# live checkout keeps the harness hermetic. Every model call falls back to opus when the
# default model fails or returns nothing.
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
import os
import subprocess
import sys

skill_path, slice_name = sys.argv[1], sys.argv[2]
is_holdout_slice = slice_name == "holdout"
skill = open(skill_path, encoding="utf-8").read()
rubric = open("rubric.md", encoding="utf-8").read()

cases = [json.loads(line) for line in open("cases.jsonl", encoding="utf-8") if line.strip()]
cases = [c for c in cases if bool(c.get("holdout")) == is_holdout_slice]

os.makedirs("candidates", exist_ok=True)


def ask(prompt):
    for model_args in ([], ["--model", "opus"]):
        run = subprocess.run(["claude", "-p", *model_args, prompt], capture_output=True, text=True)
        if run.returncode == 0 and run.stdout.strip():
            return run.stdout
    sys.exit("grader failed on the default model and on opus")


scores = []
for case in cases:
    plan_prompt = (
        "Follow this skill exactly. Do NOT write any file — this is a dry plan. State: "
        "(1) which destination you picked and the one-line test from section 1 that "
        "justifies it, (2) the exact file layout you would create, (3) the test command "
        "you would run before shipping, (4) the wiring/registration step if any, (5) the "
        "one sentence you would add to the calling skill/agent naming the tool. Output "
        "nothing else.\n\nSKILL:\n" + skill +
        "\n\nSITUATION:\n" + case["input"]
    )
    plan = ask(plan_prompt).strip()
    open(os.path.join("candidates", f"{slice_name}-{case['id']}.txt"), "w", encoding="utf-8").write(plan)

    judge_prompt = (
        "Grade one eval case for a skill. Reply with ONLY a JSON object "
        '{"score": <integer 0-10>, "failure_mode": "<short tag>" or null}.\n\n'
        "RUBRIC:\n" + rubric +
        "\n\nSITUATION:\n" + case["input"] +
        "\n\nEXPECT:\n" + case["expect"] +
        "\n\nCANDIDATE PLAN:\n" + plan +
        "\n\nGrade the candidate plan against EXPECT per the rubric. Any catastrophic item "
        "in the rubric scores 0 regardless of the rest."
    )
    out = ask(judge_prompt)
    verdict = json.loads(out[out.find("{"):out.rfind("}") + 1])

    print(json.dumps({"id": case["id"], "score": verdict["score"],
                      "failure_mode": verdict.get("failure_mode")}), flush=True)
    scores.append(verdict["score"])

if scores:
    print(f"mean {sum(scores) / len(scores):.2f} over {len(scores)} cases ({slice_name} slice)",
          file=sys.stderr)
else:
    print(f"no cases in {slice_name} slice", file=sys.stderr)
PY
