# Create the approved resources

Run this branch only after the user approves the complete proposal.

## 1. Check the host

Use Z shell for every command. Confirm that the approved root exists. The default root is `~/Documents/Github/`.

```zsh
test -d "$HOME/Documents/Github"
```

Install GitHub CLI(Command-Line Interface) when it is absent. Use Homebrew on macOS when Homebrew is available.

```zsh
if ! command -v gh >/dev/null; then
  command -v brew >/dev/null || return 1
  brew install gh
fi
```

If no supported package manager is available, stop and ask the user to select an installation method. Never pipe an unverified remote installer into a shell.

Authenticate when required. Request the project scope when the current token lacks it. Never print a token.

```zsh
gh auth status
gh auth login --scopes repo,project
gh auth refresh -s project
```

Run only the required authentication command. Confirm the account with `gh api user --jq .login`. Confirm the approved owner again when it differs.

Verify the Git identity and required command support. Ask for a missing Git name or email. Never guess either value.

```zsh
git --version >/dev/null
git_init_help=$(git init -h 2>&1 || true)
rg -q -- '-b' <<<"$git_init_help"
git config --global --get user.name >/dev/null
git config --global --get user.email >/dev/null
gh project create --help >/dev/null
gh project field-create --help >/dev/null
gh project item-add --help >/dev/null
gh project item-edit --help | rg -q -- '--field'
gh project item-edit --help | rg -q -- '--value'
gh issue create --help | rg -q -- '--blocked-by'
```

Upgrade GitHub CLI through the detected package manager when a command is absent. Stop when an upgrade is not available.

## 2. Check every name

Require every approved local path to be absent. Check the project path and each nested repository path.

```zsh
test ! -e "$approved_path"
```

Probe each approved GitHub repository. A successful query is a collision. An HTTP 404 response proves only that no repository is visible. Stop on every other probe error. Treat a later name-conflict response from `gh repo create` as authoritative.

```zsh
probe=$(mktemp)
if gh api "repos/$owner/$repo" >/dev/null 2>"$probe"; then
  print -u2 -- "collision: $owner/$repo"
  return 1
elif ! rg -q '\(HTTP 404\)' "$probe"; then
  cat "$probe" >&2
  return 1
fi
```

List all projects for the approved owner. Stop when an existing title matches the approved board title without regard to case.

```zsh
gh project list --owner "$owner" --closed --limit 1000 --format json > "$projects_snapshot"
jq -e --arg title "$board_title" '
  [.projects[].title | ascii_downcase] | index($title | ascii_downcase) == null
' "$projects_snapshot" >/dev/null
```

Repeat all collision checks immediately before the first write. Done when every name is available and every probe completed without an unknown error.

## 3. Validate the creation manifest

After approval, create a private temporary manifest path. Write the exact approved proposal there. Use absolute local paths.

```zsh
umask 077
manifest=$(mktemp "${TMPDIR:-/tmp}/create-project.${project_slug}.XXXXXX")
```

Use this shape:

```json
{
  "version": 1,
  "project": {
    "name": "<project name>", "slug": "<project-slug>",
    "owner": "<GitHub owner>", "path": "<absolute project path>",
    "production": "<testable production event>"
  },
  "board": {"title": "<project name>", "visibility": "private"},
  "repositories": [{
    "name": "<repository slug>", "path": "<absolute repository path>",
    "one_liner": "<one-line purpose>", "visibility": "private",
    "license": null, "is_primary": true
  }],
  "issues": [{
    "id": 1, "repository": "<repository slug>",
    "title": "<issue title>", "body": "<full approved body>",
    "priority": "High", "blocked_by": []
  }]
}
```

Validate the manifest before any project resource exists.

```zsh
jq -e '
  .version == 1 and (.project.name | type == "string" and length > 0) and
  (.project.slug | test("^[a-z0-9]+([.-][a-z0-9]+)*$")) and
  (.project.owner | type == "string" and length > 0) and (.project.path | startswith("/")) and
  (.project.production | type == "string" and length > 0) and (.board.title == .project.name) and
  (.board.visibility == "private" or .board.visibility == "public") and
  (.repositories | type == "array" and length >= 1) and ([.repositories[] | select(.is_primary)] | length == 1) and
  ([.repositories[].name] | length == (unique | length)) and ([.repositories[].path] | length == (unique | length)) and
  (all(.repositories[]; (.name | test("^[a-z0-9]+([._-][a-z0-9]+)*$")) and
    (.path | startswith("/")) and (.one_liner | type == "string" and length > 0) and
    (.visibility == "private" or .visibility == "public" or .visibility == "internal") and
    (.license == null or (.license | type == "string" and length > 0)))) and
  (.repositories as $r | .project.path as $p | if ($r | length) == 1
    then $r[0].path == $p else all($r[]; .path | startswith($p + "/")) end) and
  (.repositories as $r | ($r[] | select(.is_primary) | .visibility) as $v |
    .board.visibility == (if $v == "internal" then "private" else $v end)) and
  (.issues | type == "array" and length >= 1 and length <= 10) and (.issues as $i | [$i[].id] == [range(1; ($i | length) + 1)]) and
  ([.repositories[].name] as $n | all(.issues[]; .repository as $r | ($n | index($r)) != null)) and
  (all(.issues[]; (.title | type == "string" and length > 0) and (.body | type == "string" and length > 0) and
    (.priority == "Urgent" or .priority == "High" or .priority == "Medium" or .priority == "Low") and
    (.blocked_by | type == "array" and length == (unique | length)) and
    (.id as $id | all(.blocked_by[]; type == "number" and floor == . and . < $id))))
' "$manifest" >/dev/null
```

Render the manifest and compare it with the approved proposal. Stop on any difference. Done when `jq` exits zero and the rendered content matches the approval.

## 4. Create and push each repository

Create a single repository directly at the project path. For multiple repositories, create the project path and one nested path per repository.

```zsh
mkdir -p "$repo_path"
```

For each repository, write only the approved files:

- `README.md` contains the repository name and its one-line purpose.
- `.gitignore` contains only suitable operating-system, editor, secret, or project ignores. Create the file even when it is empty.
- `LICENSE` contains the approved license text from an authoritative source when the user approved a license.

Do not write application code, infrastructure, package manifests, or empty source directories.

Initialize each repository and inspect the staged files before the commit.

```zsh
git -C "$repo_path" init -b main
git -C "$repo_path" add README.md .gitignore
test ! -f "$repo_path/LICENSE" || git -C "$repo_path" add LICENSE
git -C "$repo_path" diff --cached --check
git -C "$repo_path" diff --cached --name-only
git -C "$repo_path" commit -m "Initial project setup"
```

Stop when the staged file list differs from the approved list. Create the remote repository and push the commit.

```zsh
gh repo create "$owner/$repo_name" "$visibility_flag" \
  --source "$repo_path" --remote origin --push --description "$one_liner"
```

Use exactly one of `--private`, `--public`, or `--internal` for `$visibility_flag`. After each write, record the returned URL. Do not start the next resource until the current resource passes its check.

## 5. Create the shared board

Create the board and save its returned number, node identifier, and URL.

```zsh
project_json=$(gh project create --owner "$owner" --title "$board_title" --format json)
project_number=$(jq -er '.number' <<<"$project_json")
project_id=$(jq -er '.id' <<<"$project_json")
project_url=$(jq -er '.url' <<<"$project_json")
```

Set the board visibility to match the primary repository. Use private when the primary repository is internal. GitHub stores board visibility separately.

```zsh
gh api graphql \
  -f query='mutation($project: ID!, $isPublic: Boolean!) {
    updateProjectV2(input: {projectId: $project, public: $isPublic}) {
      projectV2 { id public }
    }
  }' \
  -f project="$project_id" -F isPublic="$is_public" \
  --jq '.data.updateProjectV2.projectV2'
```

Create the required Priority field.

```zsh
gh project field-create "$project_number" --owner "$owner" \
  --name Priority --data-type SINGLE_SELECT \
  --single-select-options 'Urgent,High,Medium,Low' --format json
```

Read the board and field back. Confirm the title, visibility, field name, and all four option names before issue creation.

## 6. Create and prioritize the issues

Create issues in numeric manifest order. Every dependency therefore points to an issue that already exists. Use issue URLs for dependencies, including dependencies across repositories.

```zsh
issue_url=$(gh issue create --repo "$owner/$issue_repo" \
  --title "$issue_title" --body-file "$issue_body_file" \
  --blocked-by "$blocked_by_urls")
gh project item-add "$project_number" --owner "$owner" --url "$issue_url" >/dev/null
gh project item-edit "$project_number" --owner "$owner" --url "$issue_url" \
  --field Priority --value "$priority" >/dev/null
```

Omit `--blocked-by` when the list is empty. Write the exact approved issue body to a temporary file. Never interpolate a body through a shell argument.

After each issue, read the issue and project item back. Confirm the repository, title, complete body, dependency URLs, board membership, and Priority value. Stop on the first mismatch.

## 7. Verify and report

Verify each local repository:

```zsh
git -C "$repo_path" status --short
git -C "$repo_path" branch --show-current
git -C "$repo_path" remote get-url origin
git -C "$repo_path" log -1 --format='%s'
git -C "$repo_path" ls-remote --exit-code origin main >/dev/null
```

Require a clean status, branch `main`, the approved remote, commit message `Initial project setup`, and a remote `main` reference.

Verify each remote repository with `gh repo view`. Verify the board with `gh project view` and `gh project field-list`. Verify all items with `gh project item-list`. The final report names each command result and every URL.

If any operation fails, stop. List the resources that the read-back checks prove exist. Name the failed command and its exact error. Ask whether the user wants to resume or approve cleanup. Never delete a local folder, repository, board, or issue without that approval.
