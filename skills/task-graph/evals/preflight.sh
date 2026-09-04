#!/bin/zsh
set -euo pipefail
here=${0:A:h}
candidate=${1:-$here/../SKILL.md}
[[ -r $candidate ]] || { print -u2 "candidate not found: $candidate"; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

print -r -- '{"ticket":"SMK-0001","tasks":[{"id":"SMK-0001.T01","short":"a","long":"x","deps":[],"status":"todo","files":["a"],"created":"x","kind":"code"},{"id":"SMK-0001.T02","short":"b","long":"x","deps":["SMK-0001.T01"],"status":"todo","files":["b"],"created":"x","kind":"code"}]}' > "$tmp/tasks.json"
"$here/../scripts/dag-mermaid.sh" "$tmp/tasks.json" | grep -q 'flowchart TD' || { print -u2 'mermaid render failed'; exit 1; }

print -r -- '{"ticket":"SMK-0001","tasks":[{"id":"A","short":"a","long":"x","deps":["B"],"status":"todo","files":[],"created":"x","kind":"code"},{"id":"B","short":"b","long":"x","deps":["A"],"status":"todo","files":[],"created":"x","kind":"code"}]}' > "$tmp/cycle.json"
if "$here/../scripts/dag-mermaid.sh" "$tmp/cycle.json" >/dev/null 2>&1; then
  print -u2 'cycle not rejected'
  exit 1
fi

print -r -- '{"prefix":"SMK","next_nnnn":5,"tickets":[{"id":"SMK-0001","short":"base","long":"x","deps":[],"status":"done","files":[],"created":"x","kind":"ticket"},{"id":"SMK-0002","short":"mid","long":"x","deps":["SMK-0001"],"status":"todo","files":[],"created":"x","kind":"ticket"},{"id":"SMK-0003","short":"leaf","long":"x","deps":["SMK-0002"],"status":"todo","files":[],"created":"x","kind":"ticket"},{"id":"SMK-0004","short":"side","long":"x","deps":["SMK-0001"],"status":"todo","files":[],"created":"x","kind":"ticket"}]}' > "$tmp/roadmap.json"
[[ $("$here/../scripts/next-ticket.sh" "$tmp/roadmap.json" 2>/dev/null) == SMK-0002 ]] || { print -u2 'next-ticket chose the wrong ticket'; exit 1; }

print -r -- '{"ticket":"SMK-0001","tasks":[{"id":"A","short":"a","long":"x","deps":[],"status":"blocked","files":[],"created":"x","kind":"code"}]}' > "$tmp/badstatus.json"
if "$here/../scripts/dag-mermaid.sh" "$tmp/badstatus.json" >/dev/null 2>&1; then
  print -u2 'out-of-enum status not rejected'
  exit 1
fi

print -r -- '{"ticket":"SMK-0001","tasks":[{"id":"A","short":"a","long":"x","deps":[],"status":"todo","files":["s.ts"],"created":"x","kind":"code"},{"id":"B","short":"b","long":"x","deps":[],"status":"todo","files":["s.ts"],"created":"x","kind":"code"}]}' > "$tmp/overlap.json"
if "$here/../scripts/dag-mermaid.sh" "$tmp/overlap.json" >/dev/null 2>&1; then
  print -u2 'same-wave shared file not rejected'
  exit 1
fi

print -r -- '{"prefix":"SMK","next_nnnn":2,"tickets":[{"id":"SMK-0001","short":"a","long":"x","deps":["SMK-9999"],"status":"todo","files":[],"created":"x","kind":"ticket"}]}' > "$tmp/unknown-dep.json"
if "$here/../scripts/next-ticket.sh" "$tmp/unknown-dep.json" >/dev/null 2>&1; then
  print -u2 'unknown dependency not rejected'
  exit 1
fi

print -r -- '{"prefix":"SMK","next_nnnn":3,"tickets":[{"id":"SMK-0002","short":"b","long":"x","deps":[],"status":"todo","files":[],"created":"x","kind":"ticket"},{"id":"SMK-0001","short":"a","long":"x","deps":[],"status":"todo","files":[],"created":"x","kind":"ticket"}]}' > "$tmp/tie.json"
[[ $("$here/../scripts/next-ticket.sh" "$tmp/tie.json" 2>/dev/null) == SMK-0001 ]] || { print -u2 'tie-break chose the wrong ticket'; exit 1; }

print -r -- '{"prefix":"SMK","next_nnnn":4,"tickets":[{"id":"SMK-0001","short":"a","long":"x","deps":[],"status":"cancelled","files":[],"created":"x","kind":"ticket"},{"id":"SMK-0002","short":"b","long":"x","deps":["SMK-0001"],"status":"todo","files":[],"created":"x","kind":"ticket"},{"id":"SMK-0003","short":"c","long":"x","deps":[],"status":"todo","files":[],"created":"x","kind":"ticket"}]}' > "$tmp/replan.json"
pick=$("$here/../scripts/next-ticket.sh" "$tmp/replan.json" 2>"$tmp/replan-error")
[[ $pick == SMK-0003 ]] || { print -u2 'replan ticket was auto-selected'; exit 1; }
grep -q 'needs-replan: SMK-0002' "$tmp/replan-error" || { print -u2 'replan warning missing'; exit 1; }
