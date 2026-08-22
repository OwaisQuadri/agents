#!/usr/bin/env bash
# TODO(AGNT-0032.T22): make the ai-author exam execute real artifact behavior
# TODO(AGNT-0032.T17): route predictive graders through the configured judge
set -euo pipefail
cd "$(dirname "$0")"

slice=nonholdout
if [[ "${1:-}" == "--holdout" ]]; then
  slice=holdout
  shift
fi
skill="${1:-../SKILL.md}"

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

python3 - "$skill" "$slice" <<'PY'
import json
import os
import subprocess
import sys

skill_path, slice_name = sys.argv[1], sys.argv[2]
is_holdout_slice = slice_name == "holdout"
skill_dir = os.path.abspath("..")
skill = open(skill_path, encoding="utf-8").read()
rubric = open("rubric.md", encoding="utf-8").read()

cases = [json.loads(line) for line in open("cases.jsonl", encoding="utf-8") if line.strip()]
cases = [c for c in cases if bool(c.get("holdout")) == is_holdout_slice]

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

if scores:
    print(f"mean {sum(scores) / len(scores):.2f} over {len(scores)} cases ({slice_name} slice)", file=sys.stderr)
else:
    print(f"no cases in {slice_name} slice", file=sys.stderr)
PY
