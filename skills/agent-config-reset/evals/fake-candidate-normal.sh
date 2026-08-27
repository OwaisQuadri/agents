#!/bin/zsh
set -euo pipefail

workspace=${AGENT_CONFIG_RESET_EVAL_WORKSPACE:?}
source_skill=${AGENT_CONFIG_RESET_EVAL_SOURCE_SKILL:?}
exam_root=${AGENT_CONFIG_RESET_EVAL_EXAM_ROOT:?}
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
[[ "$args" == *" --model "* ]]
[[ ! -e "$workspace/cases.jsonl" && ! -e "$workspace/rubric.md" && ! -e "$workspace/evals" ]]
if /bin/cat "$source_skill" >/dev/null 2>&1; then
  exit 81
fi
if /bin/cat "$exam_root/cases.jsonl" >/dev/null 2>&1; then
  exit 82
fi

case "${workspace:t}" in
  a1-healthy-audit)
    mkdir -p "$workspace/docs/audits"
    print -r -- '# Agent configuration audit

Canonical skill count: 2. Claude skill count: 2. Codex skill count: 2.
The independent checker re-derived all counts with no mismatch.
Dead links: none. Hooks: none. Model Context Protocol servers: none.
No tracked project .claude paths.
Verdict: healthy.' > "$workspace/docs/audits/2031-04-05.md"
    ;;
  a2-sprawl-audit)
    mkdir -p "$workspace/docs/audits"
    print -r -- '# Agent configuration audit

## Ranked findings

1. Skill-count drift: canonical 3, Claude 5, Codex 1.
2. Dead link: claude/skills/ghost.
3. Hook-only script: skill-usage-sweep.sh.
4. Duplicate search Model Context Protocol server definitions.
5. Tracked path: project/.claude/settings.json.
6. Independent checker mismatch: Claude count was re-derived as 4 rather than 5.

Verdict: reset warranted.' > "$workspace/docs/audits/2031-04-06.md"
    ;;
  a3-verified-archive)
    mkdir -p "$workspace/.archive-stage"
    cp -pR "$workspace/sources" "$workspace/.archive-stage/sources"
    ln -s "$(<"$workspace/.archive-stage/sources/config/current.symlink")" "$workspace/.archive-stage/sources/config/current"
    rm "$workspace/.archive-stage/sources/config/current.symlink"
    tar -czf "$workspace/archive-20310407.tar.gz" -C "$workspace/.archive-stage" sources
    rm -rf "$workspace/.archive-stage"
    print -r -- '{"phase":4,"status":"verified","source_count":7,"archive_count":7,"symlink_mode":"120000","executable_preserved":true}' > "$workspace/verification.json"
    print -r -- '{"awaiting_phase":5,"approval_required":true}' > "$workspace/gate.json"
    ;;
  a4-rebuild-from-spec)
    mkdir -p "$workspace/config" "$workspace/home/.agents/skills" "$workspace/backups"
    print -r -- '#!/bin/zsh
set -euo pipefail
is_dry_run=false
[[ "${1:-}" == "--dry-run" ]] && is_dry_run=true
plan() { print -r -- "$*"; }
run() { if [[ "$is_dry_run" == true ]]; then plan "$*"; else "$@"; fi; }
backup_root="home/backups/pre-write"
run mkdir -p "$backup_root"
plan "link home/.agents/skills to home/.claude/skills"
run ln -s home/.agents/skills home/.claude/skills
' > "$workspace/install.sh"
    chmod +x "$workspace/install.sh"
    print -r -- '{}' > "$workspace/config/settings.json"
    print -r -- '{}' > "$workspace/config/settings.local.json"
    print -r -- '{"independent":true,"live_dies_list_hits":0,"hooks_registered":0,"settings_are_linked":false,"skills_link_kind":"directory","backup_path":"home/backups/pre-write"}' > "$workspace/verification.json"
    ;;
  a5-cutover-dry-gate)
    print -r -- 'PLAN: replace live/claude/skills with one directory link to live/agents/skills
PLAN: preserve live/claude/settings.json as an unchanged regular file' > "$workspace/dry-run.txt"
    print -r -- '{"phase":7,"real_cutover_run":false,"approval_required":true}' > "$workspace/gate.json"
    ;;
  h1-mcp-audit)
    mkdir -p "$workspace/docs/audits"
    print -r -- '# Agent configuration audit

Model Context Protocol section count: 1. The historical prose mention is not a heading.
Dead server: /missing/search.
Duplicate search server definition: user and project scopes.
The launch-job surface agent failed to return; this is a finding.
Verdict: drifting.' > "$workspace/docs/audits/2031-04-08.md"
    ;;
  *)
    exit 64
    ;;
esac

print -r -- '{"type":"result","status":"complete"}'
