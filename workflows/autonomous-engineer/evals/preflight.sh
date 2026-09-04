#!/bin/zsh
set -euo pipefail
here=${0:A:h}
candidate=${1:-}
incumbent=$here/../autonomous-engineer.workflow.js
if [[ -z $candidate || ${candidate:t} == SKILL.md || ${candidate:e} == md ]]; then
  definition=$incumbent
else
  definition=$candidate
fi
[[ -r $definition ]] || { print -u2 "workflow not found: $definition"; exit 1; }

node - "$definition" <<'NODE'
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

has() {
  grep -Fq -- "$1" "$definition"
}

checks=(
  'autonomous-engineer-state repair-worktree --repo ${repo}'
  'is_real_worktree'
  'blockedBy'
  'manual-only'
  'closingIssuesReferences'
  'workflow('
  'stopBeforeDraft'
  "return stopBeforeDraft('failed', 'implementation-failed')"
  'verificationCheckout'
  'origin/${state.branch}^{commit}'
  'const repairedDraftSafety'
  'git worktree add --detach'
  'git worktree remove --force'
  'resume-draft'
  '<!-- autonomous-engineer repairs=${state.repairs} -->'
  'Closes #${task.id}'
  "agentType: 'Plan'"
  "agentType: 'code-reviewer'"
  "'anchor-verifier'"
  'model: models.T3'
  "isolation: 'worktree'"
  'model: models.T4'
  'models.T4ReviewAfterRepair'
  'model: models.T5'
  'while (!verification.is_pass && state.repairs < maxRepairs)'
  'agent-cap-before-repair'
  "return result('verified-ready'"
)
for check in "${checks[@]}"; do
  has "$check" || { print -u2 "missing topology token: $check"; exit 1; }
done
if grep -Eiq 'gh pr merge|git merge|\bmerge\(' "$definition"; then
  print -u2 'forbidden merge operation found'
  exit 1
fi
if grep -Eq 'if .*reason|reason.*if' "$definition"; then
  print -u2 'reason text controls flow'
  exit 1
fi
