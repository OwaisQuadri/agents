#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

# Default (no --holdout): grades BOTH slices in one pass and writes one merged
# evals/frontier.jsonl record at the end (candidate_id, both slices' scores, ts) --
# per templates/eval-harness.md's "frontier.jsonl" section. --holdout keeps a fast,
# frontier-write-free single-slice recheck for a quick look at the holdout slice alone.
holdout_only=0
if [[ "${1:-}" == "--holdout" ]]; then
  holdout_only=1
  shift
fi
skill="${1:-../SKILL.md}"
# ACCEPTED=true|false, set by the caller once GEPA loop step 4 (Decide) has judged this
# run against the incumbent -- run.sh cannot know acceptance at grading time, so it
# defaults to false (nothing ships without an explicit Decide anyway).
accepted="${ACCEPTED:-false}"

T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/fake-artifact/votes"
printf '%s\n' '{"ts":"x","artifact":"fake","grade":"9","vote":"SENTINEL-PRIOR-VOTE"}' \
  > "$T/fake-artifact/votes/votes.jsonl"
out=$(echo "second vote" | python3 ../scripts/submit_vote.py --artifact "$T/fake-artifact" --grade 7)
[[ "$out" == "vote recorded" ]] || { echo "smoke: submit_vote did not confirm" >&2; exit 1; }
if grep -q "SENTINEL-PRIOR-VOTE" <<<"$out"; then
  echo "smoke: submit_vote leaked a prior vote to its caller" >&2; exit 1
fi
[[ "$(wc -l < "$T/fake-artifact/votes/votes.jsonl")" -eq 2 ]] || { echo "smoke: votes not append-only" >&2; exit 1; }
head -1 "$T/fake-artifact/votes/votes.jsonl" | grep -q "SENTINEL-PRIOR-VOTE" || { echo "smoke: prior vote rewritten" >&2; exit 1; }
python3 -c 'import json,sys; json.loads(open(sys.argv[1]).readlines()[1])' "$T/fake-artifact/votes/votes.jsonl" \
  || { echo "smoke: appended vote is not valid JSON" >&2; exit 1; }
if echo "" | python3 ../scripts/submit_vote.py --artifact "$T/fake-artifact" --grade 7 >/dev/null 2>&1; then
  echo "smoke: empty vote not rejected" >&2; exit 1
fi
echo "smoke: scripts pass" >&2

python3 - "$skill" "$holdout_only" "$accepted" <<'PY'
import hashlib
import json
import os
import subprocess
import sys
import time

skill_path, holdout_only, accepted = sys.argv[1], sys.argv[2] == "1", sys.argv[3] == "true"
skill_dir = os.path.abspath("..")
skill = open(skill_path, encoding="utf-8").read()
rubric = open("rubric.md", encoding="utf-8").read()
all_cases = [json.loads(line) for line in open("cases.jsonl", encoding="utf-8") if line.strip()]

def grade_slice(is_holdout):
    cases = [c for c in all_cases if bool(c.get("holdout")) == is_holdout]
    scores = []
    for case in cases:
        text = skill
        for extra in case.get("files", []):
            path = os.path.join(skill_dir, extra)
            text += "\n\n--- " + extra + " ---\n" + open(path, encoding="utf-8").read()
        prompt = (
            "Grade one eval case for a skill. Reply with ONLY a JSON object "
            '{"score": <integer 0-10>, "failure_mode": "<short tag>" or null}.\n\n'
            "RUBRIC:\n" + rubric +
            "\nSKILL UNDER TEST (the skill plus the support files this case exercises):\n" + text +
            "\nCASE INPUT:\n" + case["input"] +
            "\n\nEXPECT:\n" + case["expect"] +
            "\n\nWould an agent following the skill on this input meet EXPECT? Grade per the rubric. "
            "Grade only what the skill text makes an agent do: if the skill is silent on what EXPECT "
            "requires, the agent would have to improvise it, and that is a miss."
        )
        verdict = None
        for model_args in ([], ["--model", "opus"]):
            run = subprocess.run(["claude", "-p", *model_args, prompt], capture_output=True, text=True)
            start, end = run.stdout.find("{"), run.stdout.rfind("}")
            if run.returncode == 0 and start != -1:
                verdict = json.loads(run.stdout[start:end + 1])
                break
        if verdict is None:
            sys.exit(f"grader failed on the default model and on opus (case {case['id']})")
        print(json.dumps({"id": case["id"], "score": verdict["score"], "failure_mode": verdict.get("failure_mode")}))
        scores.append(verdict["score"])
    slice_name = "holdout" if is_holdout else "nonholdout"
    if scores:
        print(f"mean {sum(scores) / len(scores):.2f} over {len(scores)} cases ({slice_name} slice)", file=sys.stderr)
    else:
        print(f"no cases in {slice_name} slice", file=sys.stderr)
    return scores

if holdout_only:
    grade_slice(True)
    sys.exit(0)

scores_nonholdout = grade_slice(False)
scores_holdout = grade_slice(True)

# Frontier record -- appended every full run, accepted or not, per
# templates/eval-harness.md's "frontier.jsonl" section: run.sh already computes these
# per-case scores, this just stops discarding them the moment the process exits.
candidate_id = hashlib.sha1(skill.encode("utf-8")).hexdigest()[:8]
tested_against = subprocess.run(
    ["git", "log", "-1", "--format=%h", "--", "..",
     ":(exclude)**/evals/**", ":(exclude)**/TUNING.md", ":(exclude)**/logs/**", ":(exclude)**/votes/**"],
    capture_output=True, text=True, cwd=skill_dir,
).stdout.strip()
ts = subprocess.run(["date", "+%Y-%m-%dT%H:%M:%S%z"], capture_output=True, text=True).stdout.strip()
mean_nonholdout = round(sum(scores_nonholdout) / len(scores_nonholdout), 2) if scores_nonholdout else None

os.makedirs("frontier", exist_ok=True)
with open(f"frontier/{candidate_id}.md", "w", encoding="utf-8") as f:
    f.write(skill)
with open("frontier.jsonl", "a", encoding="utf-8") as f:
    f.write(json.dumps({
        "candidate_id": candidate_id,
        "tested_against": tested_against,
        "scores_nonholdout": scores_nonholdout,
        "scores_holdout": scores_holdout,
        "mean_nonholdout": mean_nonholdout,
        "accepted": accepted,
        "ts": ts,
    }) + "\n")
print(f"frontier: recorded candidate {candidate_id} (accepted={accepted})", file=sys.stderr)
PY
