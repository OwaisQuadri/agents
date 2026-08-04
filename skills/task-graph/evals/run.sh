#!/usr/bin/env bash
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
cat > "$T/tasks.json" <<'EOF'
{"ticket":"SMK-0001","tasks":[
{"id":"SMK-0001.T01","short":"a","long":"x","deps":[],"status":"todo","files":["a"],"created":"x","kind":"code"},
{"id":"SMK-0001.T02","short":"b","long":"x","deps":["SMK-0001.T01"],"status":"todo","files":["b"],"created":"x","kind":"code"}
]}
EOF
../scripts/dag-mermaid.sh "$T/tasks.json" | grep -q "flowchart TD" || { echo "smoke: mermaid render failed" >&2; exit 1; }
cat > "$T/cycle.json" <<'EOF'
{"ticket":"SMK-0001","tasks":[
{"id":"A","short":"a","long":"x","deps":["B"],"status":"todo","files":[],"created":"x","kind":"code"},
{"id":"B","short":"b","long":"x","deps":["A"],"status":"todo","files":[],"created":"x","kind":"code"}
]}
EOF
if ../scripts/dag-mermaid.sh "$T/cycle.json" >/dev/null 2>&1; then
  echo "smoke: cycle not rejected" >&2; exit 1
fi
cat > "$T/roadmap.json" <<'EOF'
{"prefix":"SMK","next_nnnn":5,"tickets":[
{"id":"SMK-0001","short":"base","long":"x","deps":[],"status":"done","files":[],"created":"x","kind":"ticket"},
{"id":"SMK-0002","short":"mid","long":"x","deps":["SMK-0001"],"status":"todo","files":[],"created":"x","kind":"ticket"},
{"id":"SMK-0003","short":"leaf","long":"x","deps":["SMK-0002"],"status":"todo","files":[],"created":"x","kind":"ticket"},
{"id":"SMK-0004","short":"side","long":"x","deps":["SMK-0001"],"status":"todo","files":[],"created":"x","kind":"ticket"}
]}
EOF
[[ "$(../scripts/next-ticket.sh "$T/roadmap.json" 2>/dev/null)" == "SMK-0002" ]] || { echo "smoke: next-ticket wrong pick" >&2; exit 1; }
cat > "$T/badstatus.json" <<'EOF'
{"ticket":"SMK-0001","tasks":[{"id":"A","short":"a","long":"x","deps":[],"status":"blocked","files":[],"created":"x","kind":"code"}]}
EOF
if ../scripts/dag-mermaid.sh "$T/badstatus.json" >/dev/null 2>&1; then
  echo "smoke: out-of-enum status not rejected" >&2; exit 1
fi
cat > "$T/overlap.json" <<'EOF'
{"ticket":"SMK-0001","tasks":[
{"id":"A","short":"a","long":"x","deps":[],"status":"todo","files":["s.ts"],"created":"x","kind":"code"},
{"id":"B","short":"b","long":"x","deps":[],"status":"todo","files":["s.ts"],"created":"x","kind":"code"}
]}
EOF
if ../scripts/dag-mermaid.sh "$T/overlap.json" >/dev/null 2>&1; then
  echo "smoke: same-wave shared file not rejected" >&2; exit 1
fi
cat > "$T/unknowndep.json" <<'EOF'
{"prefix":"SMK","next_nnnn":2,"tickets":[{"id":"SMK-0001","short":"a","long":"x","deps":["SMK-9999"],"status":"todo","files":[],"created":"x","kind":"ticket"}]}
EOF
if ../scripts/next-ticket.sh "$T/unknowndep.json" >/dev/null 2>&1; then
  echo "smoke: unknown dep not rejected" >&2; exit 1
fi
cat > "$T/tie.json" <<'EOF'
{"prefix":"SMK","next_nnnn":3,"tickets":[
{"id":"SMK-0002","short":"b","long":"x","deps":[],"status":"todo","files":[],"created":"x","kind":"ticket"},
{"id":"SMK-0001","short":"a","long":"x","deps":[],"status":"todo","files":[],"created":"x","kind":"ticket"}
]}
EOF
[[ "$(../scripts/next-ticket.sh "$T/tie.json" 2>/dev/null)" == "SMK-0001" ]] || { echo "smoke: tie-break wrong" >&2; exit 1; }
cat > "$T/replan.json" <<'EOF'
{"prefix":"SMK","next_nnnn":4,"tickets":[
{"id":"SMK-0001","short":"a","long":"x","deps":[],"status":"cancelled","files":[],"created":"x","kind":"ticket"},
{"id":"SMK-0002","short":"b","long":"x","deps":["SMK-0001"],"status":"todo","files":[],"created":"x","kind":"ticket"},
{"id":"SMK-0003","short":"c","long":"x","deps":[],"status":"todo","files":[],"created":"x","kind":"ticket"}
]}
EOF
pick=$(../scripts/next-ticket.sh "$T/replan.json" 2>"$T/replan-err")
[[ "$pick" == "SMK-0003" ]] || { echo "smoke: replan ticket auto-selected" >&2; exit 1; }
grep -q "needs-replan: SMK-0002" "$T/replan-err" || { echo "smoke: replan warning missing" >&2; exit 1; }
echo "smoke: scripts pass" >&2

python3 - "$skill" "$slice" <<'PY'
import json
import subprocess
import sys

skill_path, slice_name = sys.argv[1], sys.argv[2]
is_holdout_slice = slice_name == "holdout"
skill = open(skill_path, encoding="utf-8").read()
for script in ("../scripts/dag-mermaid.sh", "../scripts/next-ticket.sh"):
    skill += "\n\n--- " + script + " ---\n" + open(script, encoding="utf-8").read()
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
