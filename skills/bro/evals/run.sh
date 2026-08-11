#!/usr/bin/env bash
# run.sh — bro eval runner
# usage: ./run.sh [candidate-skill.md]   (non-holdout slice; defaults to ../SKILL.md)
#        ./run.sh --holdout [skill.md]   (held-out slice)
#
# Per case: produce the re-explanation the skill calls for, run ste-check on it for the
# mechanical rules, then grade the content against the case expect with rubric.md. An
# ste-check failure caps that case at 4 — the de-jargon rules are hard rules. Every model
# call falls back to opus when the default model fails or returns nothing.
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
import shutil
import subprocess
import sys

# install.sh links ste-check onto PATH; fall back to the repo build so an uninstalled
# checkout still runs the harness
CHECKER = shutil.which("ste-check") or os.path.abspath(
    "../../../tools/ste-check/target/release/ste-check")

skill_path, slice_name = sys.argv[1], sys.argv[2]
is_holdout_slice = slice_name == "holdout"
skill = open(skill_path, encoding="utf-8").read()
rubric = open("rubric.md", encoding="utf-8").read()
context = ""

# the skill states only what it adds on top of STE, so the writer needs the base rules
# too or it cannot satisfy the mechanical check
if os.path.exists("../../../docs/prompt-style.md"):
    context += "\n\nPROSE STYLE (the base every register runs on):\n" + open(
        "../../../docs/prompt-style.md", encoding="utf-8").read()

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
    rewrite_prompt = (
        "Follow this skill exactly and produce the re-explanation it calls for. Treat the "
        "MESSAGE in the task as your own most recent assistant message. Output ONLY the "
        "plain-words version, nothing else — no preamble, no explanation, no commentary "
        "about what you changed.\n\nSKILL:\n" + skill + context +
        "\n\nTASK:\n" + case["input"]
    )
    candidate = ask(rewrite_prompt).strip()
    open(os.path.join("candidates", f"{slice_name}-{case['id']}.txt"), "w", encoding="utf-8").write(candidate)

    check = subprocess.run([CHECKER, "--register", "bro"], input=candidate, capture_output=True, text=True)
    mechanical_fails = [l for l in check.stdout.splitlines() if l.startswith("FAIL")]

    judge_prompt = (
        "Grade one eval case for a skill. Reply with ONLY a JSON object "
        '{"score": <integer 0-10>, "failure_mode": "<short tag>" or null}.\n\n'
        "RUBRIC:\n" + rubric +
        "\nCASE INPUT (the task, including the message to re-explain):\n" + case["input"] +
        "\n\nEXPECT:\n" + case["expect"] +
        "\n\nCANDIDATE RE-EXPLANATION:\n" + candidate +
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
