#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

holdout_only=0
if [[ "${1:-}" == "--holdout" ]]; then
  holdout_only=1
  shift
fi
skill="${1:-../SKILL.md}"

python3 - "$skill" "$holdout_only" <<'PY'
import hashlib
import json
import os
import subprocess
import sys
import time

skill_path, holdout_only = sys.argv[1], sys.argv[2] == "1"
skill = open(skill_path, encoding="utf-8").read()
baseline = open("../rust-baseline.md", encoding="utf-8").read()
rubric = open("rubric.md", encoding="utf-8").read()

all_cases = [json.loads(line) for line in open("cases.jsonl", encoding="utf-8") if line.strip()]
slices = [("holdout", True)] if holdout_only else [("nonholdout", False), ("holdout", True)]

def grade(cases):
    scores = []
    for case in cases:
        prompt = (
            "Grade one eval case for a skill. Reply with only a JSON object "
            '{"score": <integer 0-10>, "failure_mode": "<short tag>" or null}.\n\n'
            "RUBRIC:\n" + rubric
            + "\nSKILL UNDER TEST:\n" + skill
            + "\nRUST BASELINE:\n" + baseline
            + "\nCASE INPUT:\n" + case["input"]
            + "\nEXPECT:\n" + case["expect"]
            + "\nWould an agent following the skill meet the expectation?"
        )
        output = subprocess.run(["claude", "-p", prompt], capture_output=True, text=True, check=True).stdout
        verdict = json.loads(output[output.find("{"):output.rfind("}") + 1])
        print(json.dumps({"id": case["id"], "score": verdict["score"], "failure_mode": verdict.get("failure_mode")}))
        scores.append(verdict["score"])
    return scores

results = {}
for slice_name, is_holdout in slices:
    cases = [case for case in all_cases if bool(case.get("holdout")) == is_holdout]
    scores = grade(cases)
    results[slice_name] = scores
    if scores:
        print(f"mean {sum(scores) / len(scores):.2f} over {len(scores)} cases ({slice_name} slice)", file=sys.stderr)
    else:
        print(f"no cases in {slice_name} slice", file=sys.stderr)

if holdout_only:
    sys.exit(0)

# Plain (both-slices) form: append this candidate's score vector to frontier.jsonl,
# keyed by a hash of its exact text, and archive the full text alongside it.
candidate_id = hashlib.sha1(skill.encode("utf-8")).hexdigest()[:8]
os.makedirs("frontier", exist_ok=True)
with open(f"frontier/{candidate_id}.md", "w", encoding="utf-8") as f:
    f.write(skill)

tested_against = subprocess.run(
    ["git", "log", "-1", "--format=%h", "--",
     "../SKILL.md", "../rust-baseline.md"],
    capture_output=True, text=True, check=True,
).stdout.strip()

scores_nonholdout = results.get("nonholdout", [])
scores_holdout = results.get("holdout", [])
mean_nonholdout = round(sum(scores_nonholdout) / len(scores_nonholdout), 2) if scores_nonholdout else None
accepted = os.environ.get("ACCEPTED", "").lower() == "true"

frontier_line = {
    "candidate_id": candidate_id,
    "tested_against": tested_against,
    "scores_nonholdout": scores_nonholdout,
    "scores_holdout": scores_holdout,
    "mean_nonholdout": mean_nonholdout,
    "accepted": accepted,
    "ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
}
with open("frontier.jsonl", "a", encoding="utf-8") as f:
    f.write(json.dumps(frontier_line) + "\n")

print(f"candidate_id {candidate_id}", file=sys.stderr)
PY
