#!/bin/zsh
set -euo pipefail
here=${0:A:h}
candidate=${1:-}
write_authority_pattern='Fetch the latest base branch from origin|Run the project.s local checks|resolve conflicts, preserving|Make the smallest safe change|Fix CI failures caused|Verify before pushing|merge the latest base|Stage only verified source fixes|Batch known fixes into one push|Integrate the latest remote state|verified fix commits|(^|[^[:alnum:]_])(git[[:space:]]+(fetch|merge|rebase|add|commit|push)|(fetch|merge|rebase|commit|push|stage|edit|fix|resolve|update)[[:space:]]+(the|this|these|a|an|changes?|result|branch|base|code|source|files?|checks?|failures?)|commit[[:space:]]+(and|or)[[:space:]]+push|run[[:space:]]+([^.;,]*[[:space:]])?(project|repository)[[:space:]]+(scripts?|tests?|checks?|commands?))([^[:alnum:]_]|$)'
write_prohibition_pattern='(^|[^[:alnum:]_])(do not|does not|don.t|never|must not|may not|cannot|can.t|without|no[[:space:]]+[^[:space:]]+[[:space:]]+may)([^[:alnum:]_]|$)'
first_write_authority() {
  cat "${1:--}" |
    sed -E $'s/[;,.]/\\\n/g; s/[[:space:]]+(but|then|instead|after|once|while|although|however)[[:space:]]+/\\\n/Ig' |
    rg -n -i "$write_authority_pattern" |
    rg -vi "$write_prohibition_pattern" |
    head -n 1
}
if [[ $candidate == --self-test ]]; then
  first_write_authority <<<'Resolve the conflict with git merge origin/main, commit the result, and run git push to update the branch.' >/dev/null || {
    print -u2 'write-authority positive sample was not rejected'
    exit 1
  }
  first_write_authority <<<'Do not push before checks pass; after they pass, update the branch and push the result.' >/dev/null || {
    print -u2 'write-authority mixed sample was not rejected'
    exit 1
  }
  first_write_authority <<<'Do not only inspect the conflict, instead edit the files and push the branch.' >/dev/null || {
    print -u2 'write-authority comma sample was not rejected'
    exit 1
  }
  first_write_authority <<<'Run project tests before review.' >/dev/null || {
    print -u2 'project-command sample was not rejected'
    exit 1
  }
  rg -qi "$write_authority_pattern" <<<'No stage may commit code or push the branch.' || {
    print -u2 'write-authority negative sample did not exercise the detector'
    exit 1
  }
  if first_write_authority <<<'No stage may commit code or push the branch.' >/dev/null; then
    print -u2 'write-authority negative sample was rejected'
    exit 1
  fi
  print 'write-authority self-test passed'
  exit 0
fi
incumbent=$here/../pr-ready.workflow.js
if [[ -z $candidate || ${candidate:t} == SKILL.md ]]; then
  definition=$incumbent
else
  definition=$candidate
fi
[[ -r $definition ]] || { print -u2 "workflow not found: $definition"; exit 1; }
node -e 'const fs=require("fs"), A=Object.getPrototypeOf(async function(){}).constructor; new A("args", "agent", "phase", "parallel", "log", fs.readFileSync(process.argv[1], "utf8").replace(/^export const meta/m, "const meta"))' "$definition" || {
  print -u2 'workflow syntax check failed'
  exit 1
}
has() {
  rg -q "$1" "$definition"
}

if match=$(first_write_authority "$definition"); then
  print -u2 "write-capable Ready instruction found: ${match#*:}"
  exit 1
fi

has 'missing input: repo_path' && has 'if \(!repo_path\) return' || { print -u2 'repo_path guard missing'; exit 1; }
has 'T6_PRIMARY' && has 'T6_FALLBACK' && has 'T5_CHAIN' || { print -u2 'tier chains missing' ; exit 1; }
has 'if \(!ready.ready\)' || { print -u2 'ready-gate missing: review/triage must not run past a blocker'; exit 1; }
if rg -A4 'triagePrompt =' "$definition" | rg -q 'transcript|conversation|caller'; then
  print -u2 'triage context leak'
  exit 1
fi
has 'usedProviders' && has 'c.provider' && has 'triageChain' && has 'effectiveTriageChain' || { print -u2 'non-same-provider rule missing'; exit 1; }
has 'deadNodes' && has 'deadReview' || { print -u2 'fan-in guard missing'; exit 1; }
has "'triage-skipped-no-review'" || { print -u2 'both-reviewers-dead case unhandled'; exit 1; }
has 'rawReviews' || { print -u2 'triage-exhausted fallback (raw reviews) missing'; exit 1; }
has "verdict: \{ type: 'string', enum: \['legit', 'not-legit'\] \}" && has "reasoning: \{ type: 'string' \}" || { print -u2 'finding schema missing verdict/reasoning'; exit 1; }
has 'Do not create a PR yourself' || { print -u2 'pre-PR mode PR-creation guard missing'; exit 1; }
has 'Never merge the PR' || { print -u2 'PR-merge guard missing'; exit 1; }
