#!/bin/zsh
set -eu

here=${0:A:h}
def=$here/../autonomous-engineer.workflow.js
cases=$here/cases.jsonl
mode=all

for argument in "$@"; do
  if [[ $argument == --holdout ]]; then
    mode=--holdout
  elif [[ $def == $here/../autonomous-engineer.workflow.js ]]; then
    def=$argument
  else
    print -u2 "usage: $0 [--holdout] [candidate-workflow.js]"
    exit 2
  fi
done

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
  "manual-only"
  "closingIssuesReferences"
  "workflow("
  "stopBeforeDraft"
  "verificationCheckout"
  "git worktree add --detach"
  "resume-draft"
  "<!-- autonomous-engineer repairs=\${state.repairs} -->"
  "Closes #\${task.id}"
  "agentType: 'Plan'"
  "agentType: 'code-reviewer'"
  "'anchor-verifier'"
  "model: models.T3"
  "isolation: 'worktree'"
  "model: models.T4"
  "models.T4ReviewAfterRepair"
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
  print -r -- "{\"id\":\"$id\",\"tier\":\"static\",\"score\":5}"
  (( count += 1 ))
done < "$cases"

[[ $count -gt 0 ]] || {
  print -u2 'selected eval slice is empty'
  exit 1
}
print -u2 "passed $count static workflow cases at the 5/10 mechanical ceiling ($mode)"
