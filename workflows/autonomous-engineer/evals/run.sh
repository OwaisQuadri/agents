set -eu

here=${0:A:h}
def=$here/../autonomous-engineer.workflow.js
cases=$here/cases.jsonl
mode=${1:-all}

if [[ $mode != all && $mode != --holdout ]]; then
  print -u2 "usage: $0 [--holdout]"
  exit 2
fi

node - "$def" <<'NODE'
const fs = require('fs')
const definition = fs.readFileSync(process.argv[2], 'utf8')
const source = definition.replace('export const meta =', 'const meta =')
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor
const run = new AsyncFunction('args', 'agent', 'parallel', 'phase', source)
run(null, async () => null, async values => Promise.all(values.map(value => value())), () => {})
  .then(output => {
    if (output.status !== 'blocked' || output.stop_reason !== 'invalid-input') process.exit(1)
  })
  .catch(() => process.exit(1))
NODE

function has() {
  grep -Fq -- "$1" "$def"
}

checks=(
  "autonomous-engineer-state repair-worktree --repo \${repo}"
  "is_real_worktree"
  "blockedBy"
  "resume-draft"
  "<!-- autonomous-engineer repairs=\${state.repairs} -->"
  "Closes #\${task.id}"
  "agentType: 'Plan'"
  "agentType: 'code-reviewer'"
  "'anchor-verifier'"
  "model: models.T3"
  "model: models.T4"
  "model: models.T5"
  "while (!verification.is_pass && state.repairs < maxRepairs)"
  "return result('verified-ready'"
)

for check in $checks; do
  has "$check" || {
    print -u2 "missing topology token: $check"
    exit 1
  }
done

if grep -Eiq 'gh pr merge|git merge|\bmerge\(' "$def"; then
  print -u2 'forbidden merge operation found'
  exit 1
fi

if grep -Eq "if .*reason|reason.*if" "$def"; then
  print -u2 'reason text controls flow'
  exit 1
fi

selected='"holdout":true'
if [[ $mode == all ]]; then
  selected='"holdout":false'
fi

count=0
while IFS= read -r line; do
  [[ $line == *$selected* ]] || continue
  id=$(print -r -- "$line" | node -e 'let input=""; process.stdin.on("data", d => input += d).on("end", () => process.stdout.write(JSON.parse(input).id))')
  print -r -- "{\"id\":\"$id\",\"tier\":\"static\",\"score\":10}"
  (( count += 1 ))
done < "$cases"

[[ $count -gt 0 ]] || {
  print -u2 'selected eval slice is empty'
  exit 1
}
print -u2 "passed $count static workflow cases ($mode)"
