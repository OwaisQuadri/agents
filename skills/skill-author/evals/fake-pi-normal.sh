#!/bin/zsh
set -euo pipefail

workspace=${SKILL_AUTHOR_EVAL_WORKSPACE:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
for contained_path in "$HOME" "$PI_CODING_AGENT_DIR" "$PI_CONFIG_DIR" "$PI_CODING_AGENT_SESSION_DIR" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$TMPDIR"; do
  contained_path=${contained_path:A}
  [[ "$contained_path" == "${workspace:A}"/* ]]
done

args=" $* "
for fence in '--no-session' '--no-skills' '--no-extensions' '--no-prompt-templates' '--no-themes' '--no-context-files' '--no-approve'; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --session-dir $workspace/.pi/session "* ]]
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]]
[[ "$args" == *" --model fake/candidate "* ]]
[[ "$args" == *" --tools read,write,edit "* ]]
[[ ! -e "$workspace/cases.jsonl" && ! -e "$workspace/rubric.md" && ! -e "$workspace/evals" ]]

make_support() {
  local root=$1
  mkdir -p "$root/evals" "$root/logs"
  print -r -- '# Observable rubric' > "$root/evals/rubric.md"
  print -r -- '#!/bin/zsh
set -euo pipefail
print -r -- "{\"status\":\"ready\"}"' > "$root/evals/run.sh"
  chmod +x "$root/evals/run.sh"
  print -r -- '{"id":"one","input":"valid","expect":"pass","holdout":false,"source":"seed"}
{"id":"two","input":"missing input","expect":"stop","holdout":false,"source":"seed"}
{"id":"three","input":"outside trigger","expect":"skip","holdout":false,"source":"seed"}
{"id":"four","input":"valid second","expect":"pass","holdout":false,"source":"seed"}
{"id":"five","input":"held out","expect":"pass","holdout":true,"source":"seed"}' > "$root/evals/cases.jsonl"
  [[ -e "$root/logs/usage.jsonl" ]] || : > "$root/logs/usage.jsonl"
}

case "${workspace:t}" in
  s1-cache-key-auditor)
    root="$workspace/skills/cache-key-auditor"
    mkdir -p "$root"
    print -r -- '---
name: cache-key-auditor
description: Use when a maintainer compares declared cache-key inputs with a build manifest. Skip when the request is general build debugging.
---

# cache-key-auditor

JOB: compare declared cache-key inputs with one build manifest.
IN: `manifest_path` and `declared_inputs`.
OUT: `cache-key-report.json` with `missing`, `unused`, and `verdict`.

## Recipe

1. Stop if `manifest_path` or `declared_inputs` is absent.
2. Read the manifest and compare both input sets.
3. Write the fixed report after every input is accounted for.

## evals

Run `evals/run.sh` for the observable cases.

## logging

Append one bounded local-time usage record.' > "$root/SKILL.md"
    make_support "$root"
    ;;
  s2-trigger-repair)
    root="$workspace/skills/alert-triage"
    print -r -- '---
name: alert-triage
description: Use when repeated service alerts need classification. Skip when the event is a one-off local error.
---

# alert-triage

JOB: classify repeated service alerts.
IN: alert records from one service.
OUT: `triage.json` with severity and owner.

## Recipe

PRESERVE-RECIPE-MARKER

1. Read all alert records.
2. Group every repeated signature.
3. Write `triage.json` after every group has an owner.

## evals

Run `evals/run.sh` for the observable cases.

## logging

Append one bounded local-time usage record.' > "$root/SKILL.md"
    make_support "$root"
    ;;
  s3-ai-author-fence)
    print -r -- '{"verdict":"route-to-ai-author","reason":"ai-author owns the should-it-exist and artifact-type decision."}' > "$workspace/decision.json"
    ;;
  s4-conditional-reference)
    root="$workspace/skills/changelog-curator"
    mkdir -p "$root"
    print -r -- '---
name: changelog-curator
description: Use when a maintainer prepares a release changelog from merged entries. Skip when the request is release-note publication.
---

# changelog-curator

JOB: prepare one release changelog from merged entries.
IN: `entries_path` and `release_kind`.
OUT: `CHANGELOG_DRAFT.md` with Added, Changed, Fixed, and Removed sections.

## Recipe

1. Read all merged entries.
2. For a monorepo release, read `REFERENCES.md` before classification.
3. Write all four fixed sections after every entry is classified.

## evals

Run `evals/run.sh` for the observable cases.

## logging

Append one bounded local-time usage record.' > "$root/SKILL.md"
    print -r -- '# Monorepo releases

Use the package ownership matrix to assign each entry.' > "$root/REFERENCES.md"
    make_support "$root"
    ;;
  s5-hand-only)
    root="$workspace/skills/archive-retirement"
    mkdir -p "$root"
    print -r -- '---
name: archive-retirement
description: Retire an approved archive after destination verification.
disable-model-invocation: true
---

# archive-retirement

JOB: retire one approved archive.
IN: `archive_path` and `verified_destination`.
OUT: `retirement.json` with `archive_path`, `verified_destination`, and `status`.

## Recipe

1. Stop unless destination verification is explicit.
2. Verify the destination before the archive changes.
3. Write the fixed retirement record after completion.

## evals

Run `evals/run.sh` for the observable cases.

## logging

Append one bounded local-time usage record.' > "$root/SKILL.md"
    make_support "$root"
    ;;
  h1-stale-command-repair)
    root="$workspace/skills/deploy-preview"
    print -r -- '---
name: deploy-preview
description: Use when an approved manifest needs an isolated preview. Skip when the target is production.
---

# deploy-preview

JOB: create one isolated deployment preview.
IN: an approved manifest path.
OUT: `preview.json` with the preview identifier and status.

## Recipe

PRESERVE-DEPLOY-RECIPE

1. Never deploy to production.
2. Run `previewctl create --manifest <path>`.
3. Write `preview.json` after the command succeeds.

## evals

Run `evals/run.sh` for the observable cases.

## logging

Append one bounded local-time usage record.' > "$root/SKILL.md"
    make_support "$root"
    ;;
  *)
    exit 64
    ;;
esac

print -r -- '{"type":"result","status":"complete"}'
