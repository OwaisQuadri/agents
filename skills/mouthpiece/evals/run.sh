#!/usr/bin/env bash
# run.sh — mouthpiece eval runner
# usage: ./run.sh [candidate-skill.md]   (non-holdout slice; defaults to ../SKILL.md)
#        ./run.sh --holdout [skill.md]   (held-out slice)
#
# Per case: write a candidate message under the skill, run check.py on it for the
# mechanical rules, then grade the content against the case expect with rubric.md.
# A check.py failure caps that case at 4 — the voice rules are hard rules.
# Every candidate lands in candidates/<slice>-<id>.txt so a low score can be read
# back rather than guessed at.
# ../examples.md is included when present because the skill's rhythm section requires
# it; without it the candidate cannot match the user's rhythm and scores unfairly low.
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
examples = ""
if os.path.exists("../examples.md"):
    examples = "\n\nRHYTHM EXAMPLES (the user's own messages):\n" + open("../examples.md", encoding="utf-8").read()

cases = [json.loads(line) for line in open("cases.jsonl", encoding="utf-8") if line.strip()]
cases = [c for c in cases if bool(c.get("holdout")) == is_holdout_slice]


def ask(prompt):
    for model_args in ([], ["--model", "opus"]):
        run = subprocess.run(["claude", "-p", *model_args, prompt], capture_output=True, text=True)
        if run.returncode == 0 and run.stdout.strip():
            return run.stdout
    sys.exit("grader failed on the default model and on opus")


scores = []
for case in cases:
    write_prompt = (
        "Follow this skill exactly and write the message it calls for. Output ONLY the "
        "message body, nothing else — no preamble, no explanation.\n\nSKILL:\n" + skill +
        examples + "\n\nSITUATION:\n" + case["input"]
    )
    candidate = ask(write_prompt).strip()
    os.makedirs("candidates", exist_ok=True)
    open(f"candidates/{slice_name}-{case['id']}.txt", "w", encoding="utf-8").write(candidate)

    check = subprocess.run(["python3", "check.py"], input=candidate, capture_output=True, text=True)
    mechanical_fails = [l for l in check.stdout.splitlines() if l.startswith("FAIL")]

    judge_prompt = (
        "Grade one eval case for a skill. Reply with ONLY a JSON object "
        '{"score": <integer 0-10>, "failure_mode": "<short tag>" or null}.\n\n'
        "RUBRIC:\n" + rubric +
        "\nCASE INPUT (the situation the message reports on):\n" + case["input"] +
        "\n\nEXPECT:\n" + case["expect"] +
        "\n\nCANDIDATE MESSAGE:\n" + candidate +
        "\n\nGrade the candidate against EXPECT per the rubric."
    )
    out = ask(judge_prompt)
    verdict = json.loads(out[out.find("{"):out.rfind("}") + 1])

    score = verdict["score"]
    failure_mode = verdict.get("failure_mode")
    if mechanical_fails:
        score = min(score, 4)
        failure_mode = "mechanical: " + "; ".join(f.split(":")[0].replace("FAIL  ", "") for f in mechanical_fails)

    print(json.dumps({"id": case["id"], "score": score, "failure_mode": failure_mode}), flush=True)
    scores.append(score)

if scores:
    print(f"mean {sum(scores) / len(scores):.2f} over {len(scores)} cases ({slice_name} slice)", file=sys.stderr)
else:
    print(f"no cases in {slice_name} slice", file=sys.stderr)
PY
