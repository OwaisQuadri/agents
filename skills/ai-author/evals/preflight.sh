#!/bin/zsh
set -euo pipefail
here=${0:A:h}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/artifact/votes"
print -r -- '{"ts":"x","artifact":"fake","grade":"9","vote":"SENTINEL-PRIOR-VOTE"}' > "$tmp/artifact/votes/votes.jsonl"
out=$(print -r -- 'second vote' | python3 "$here/../scripts/submit_vote.py" --artifact "$tmp/artifact" --grade 7)
[[ "$out" == 'vote recorded' ]]
[[ $(wc -l < "$tmp/artifact/votes/votes.jsonl") -eq 2 ]]
head -1 "$tmp/artifact/votes/votes.jsonl" | rg -q 'SENTINEL-PRIOR-VOTE'
python3 -c 'import json,sys; json.loads(open(sys.argv[1]).readlines()[1])' "$tmp/artifact/votes/votes.jsonl"
if print -r -- '' | python3 "$here/../scripts/submit_vote.py" --artifact "$tmp/artifact" --grade 7 >/dev/null 2>&1; then
  exit 1
fi
