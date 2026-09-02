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

# One targeted query for everything read-only: linked projects with their fields,
# plus this issue's node id and project items. A full item-list board scan costs
# hundreds of GraphQL points per call; this stays flat at any board size.
snapshot=$(gh api graphql \
  -f query='query($owner: String!, $name: String!, $n: Int!) {
    repository(owner: $owner, name: $name) {
      projectsV2(first: 2) { nodes { id number fields(first: 30) { nodes {
        ... on ProjectV2SingleSelectField { id name options { id name } } } } } }
      issue(number: $n) { id projectItems(first: 10) { nodes { id project { id } } } }
    } }' \
  -f owner="$owner" -f name="$name" -F n="$issue" --jq '.data.repository')

project_count=$(printf '%s' "$snapshot" | jq '.projectsV2.nodes | length')
if [ "$project_count" -ne 1 ]; then
  echo "gh-issue-field: expected exactly one project linked to $repo, found $project_count" >&2
  exit 1
fi
project_id=$(printf '%s' "$snapshot" | jq -r '.projectsV2.nodes[0].id')
project_number=$(printf '%s' "$snapshot" | jq -r '.projectsV2.nodes[0].number')

fields=$(printf '%s' "$snapshot" | jq '[.projectsV2.nodes[0].fields.nodes[] | select(.name != null)]')
field_json=$(printf '%s' "$fields" | jq -c --arg f "$(norm "$field")" \
  '[.[] | select((.name | ascii_downcase | gsub(" "; "-")) == $f)] | first')
if [ "$field_json" = "null" ]; then
  echo "gh-issue-field: no field named '$field' on project $project_number (fields: $(printf '%s' "$fields" | jq -r '[.[].name] | join(", ")'))" >&2
  exit 1
fi
field_id=$(printf '%s' "$field_json" | jq -r '.id')
field_name=$(printf '%s' "$field_json" | jq -r '.name')
option_id=$(printf '%s' "$field_json" | jq -r --arg v "$(norm "$value")" \
  '[.options[] | select((.name | ascii_downcase | gsub(" "; "-")) == $v)] | first | .id')
if [ "$option_id" = "null" ]; then
  echo "gh-issue-field: no option '$value' on field '$field' (options: $(printf '%s' "$field_json" | jq -r '[.options[].name] | join(", ")'))" >&2
  exit 1
fi

item_id=$(printf '%s' "$snapshot" | jq -r --arg p "$project_id" \
  '[.issue.projectItems.nodes[] | select(.project.id == $p)] | first | .id')
if [ "$item_id" = "null" ]; then
  content_id=$(printf '%s' "$snapshot" | jq -r '.issue.id')
  item_id=$(gh api graphql \
    -f query='mutation($project: ID!, $content: ID!) {
      addProjectV2ItemById(input: {projectId: $project, contentId: $content}) { item { id } } }' \
    -f project="$project_id" -f content="$content_id" \
    --jq '.data.addProjectV2ItemById.item.id')
fi

gh api graphql \
  -f query='mutation($project: ID!, $item: ID!, $field: ID!, $option: String!) {
    updateProjectV2ItemFieldValue(input: {projectId: $project, itemId: $item,
      fieldId: $field, value: {singleSelectOptionId: $option}}) { projectV2Item { id } } }' \
  -f project="$project_id" -f item="$item_id" -f field="$field_id" -f option="$option_id" \
  >/dev/null

# AGNT-INV-003 round-trip: confirm the value actually landed as sent, not just that
# gh returned 0.
got=$(gh api graphql \
  -f query='query($item: ID!, $fieldName: String!) { node(id: $item) {
    ... on ProjectV2Item { fieldValueByName(name: $fieldName) {
      ... on ProjectV2ItemFieldSingleSelectValue { name } } } } }' \
  -f item="$item_id" -f fieldName="$field_name" \
  --jq '.data.node.fieldValueByName.name // ""')
if [ "$(norm "$got")" != "$(norm "$value")" ]; then
  echo "gh-issue-field: reported success but #$issue's $field reads '$got', not '$value'" >&2
  exit 1
fi

echo "ok: #$issue $field = $value" >&2
