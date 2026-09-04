#!/bin/zsh
set -euo pipefail
here=${0:A:h}
candidate=${1:-$here/../SKILL.md}
skill=$here/../SKILL.md
classifier=$here/../scripts/scan.sh
heartbeat=$here/../scripts/heartbeat.sh
if [[ ${candidate:e} == md ]]; then
  skill=$candidate
else
  classifier=$candidate
fi
[[ -f $skill ]] || { print -u2 "skill not found: $skill"; exit 1; }
[[ -f $classifier ]] || { print -u2 "classifier not found: $classifier"; exit 1; }
/bin/bash -n "$classifier"
[[ ! -f $heartbeat ]] || /bin/bash -n "$heartbeat"
for needle in '^JOB:' 'cannot speak into' 'reset-spec.md' '^## evals' 'kind:"merge"'; do
  grep -qE "$needle" "$skill" || { print -u2 "SKILL.md missing required section: $needle"; exit 1; }
done

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cases=${CASES_FILE:-$here/cases.jsonl}
while IFS= read -r line; do
  [[ -n $line ]] || continue
  id=$(print -r -- "$line" | jq -r '.id')
  prev_json=$(print -r -- "$line" | jq -c '.input.prev')
  if [[ $prev_json == '"-"' ]]; then
    prev_arg=-
  else
    print -rn -- "$prev_json" > "$tmp/prev.json"
    prev_arg=$tmp/prev.json
  fi
  print -r -- "$line" | jq -c '.input.curr' > "$tmp/curr.json"
  out=$(/bin/bash "$classifier" --classify "$prev_arg" "$tmp/curr.json")
  if [[ -z $out ]]; then
    got_anomalies=
    got_routine=
  else
    got_anomalies=$(print -r -- "$out" | jq -r '[.anomalies[].kind] | sort | join(",")')
    got_routine=$(print -r -- "$out" | jq -r '[.routine[].kind] | sort | join(",")')
  fi
  case $id in
    c1) want_anomalies=; want_routine= ;;
    c2) want_anomalies=launchd_down; want_routine= ;;
    c3) want_anomalies=job_state_changed; want_routine= ;;
    c4) want_anomalies=; want_routine=workspace_updated ;;
    c5) want_anomalies=job_stuck; want_routine= ;;
    c6) want_anomalies=; want_routine=session_ended ;;
    c7) want_anomalies=; want_routine= ;;
    *) print -u2 "unknown case: $id"; exit 1 ;;
  esac
  [[ $got_anomalies == $want_anomalies && $got_routine == $want_routine ]] || {
    print -u2 "classifier mismatch for $id"
    exit 1
  }
done < "$cases"
