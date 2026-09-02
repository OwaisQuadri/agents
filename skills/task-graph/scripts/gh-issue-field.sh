#!/bin/sh
set -eu
usage() { echo "usage: gh-issue-field.sh <issue-number> <Status|Priority> <value>" >&2; exit 2; }

[ $# -eq 3 ] || usage
issue=$1
field=$2
value=$3

# Option display names on the project ("In progress", "Urgent") and the enum values
# scripts and skills pass ("in-progress", "urgent") are the same value in two
# spellings; comparing both through this normalization keeps either spelling valid.
norm() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | tr ' ' '-'; }

repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
owner=${repo%%/*}
name=${repo#*/}

projects=$(gh api graphql \
  -f query='query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { projectsV2(first: 2) { nodes { id number } } } }' \
  -f owner="$owner" -f name="$name" --jq '.data.repository.projectsV2.nodes')
project_count=$(printf '%s' "$projects" | jq 'length')
if [ "$project_count" -ne 1 ]; then
  echo "gh-issue-field: expected exactly one project linked to $repo, found $project_count" >&2
  exit 1
fi
project_id=$(printf '%s' "$projects" | jq -r '.[0].id')
project_number=$(printf '%s' "$projects" | jq -r '.[0].number')

fields=$(gh project field-list "$project_number" --owner "$owner" --format json | jq '.fields')
field_json=$(printf '%s' "$fields" | jq -c --arg f "$(norm "$field")" \
  '[.[] | select((.name | ascii_downcase | gsub(" "; "-")) == $f)] | first')
if [ "$field_json" = "null" ]; then
  echo "gh-issue-field: no field named '$field' on project $project_number (fields: $(printf '%s' "$fields" | jq -r '[.[].name] | join(", ")'))" >&2
  exit 1
fi
field_id=$(printf '%s' "$field_json" | jq -r '.id')
option_id=$(printf '%s' "$field_json" | jq -r --arg v "$(norm "$value")" \
  '[.options[] | select((.name | ascii_downcase | gsub(" "; "-")) == $v)] | first | .id')
if [ "$option_id" = "null" ]; then
  echo "gh-issue-field: no option '$value' on field '$field' (options: $(printf '%s' "$field_json" | jq -r '[.options[].name] | join(", ")'))" >&2
  exit 1
fi

find_item() {
  gh project item-list "$project_number" --owner "$owner" --format json --limit 500 \
    | jq -r --arg repo "$repo" --argjson n "$issue" \
      '[.items[] | select(.content.type == "Issue" and ((.content.repository // "") | endswith($repo)) and .content.number == $n)] | first | .id'
}

item_id=$(find_item)
if [ "$item_id" = "null" ]; then
  url=$(gh issue view "$issue" --json url --jq .url)
  item_id=$(gh project item-add "$project_number" --owner "$owner" --url "$url" --format json | jq -r '.id')
fi

gh project item-edit --id "$item_id" --project-id "$project_id" \
  --field-id "$field_id" --single-select-option-id "$option_id" >/dev/null

# AGNT-INV-003 round-trip: confirm the value actually landed as sent, not just that
# gh returned 0.
got=$(gh project item-list "$project_number" --owner "$owner" --format json --limit 500 \
  | jq -r --arg id "$item_id" --arg f "$(norm "$field")" '[.items[] | select(.id == $id)] | first | .[$f] // ""')
if [ "$(norm "$got")" != "$(norm "$value")" ]; then
  echo "gh-issue-field: reported success but #$issue's $field reads '$got', not '$value'" >&2
  exit 1
fi

echo "ok: #$issue $field = $value" >&2
