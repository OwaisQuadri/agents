#!/bin/zsh
# worktree-picker — prefix+shift+o popup: one spotlight-style menu over every
# open workspace (switch) and every git worktree of every repo herdr knows
# (open; opening an already-open checkout focuses it). Rows are
# "<kind> <display>\t<action>\t<arg1>\t<arg2>".
set -euo pipefail
# popups launch with a minimal PATH; herdr and fzf live in /opt/homebrew/bin
path=(/opt/homebrew/bin /usr/local/bin $path)

snapshot="$(herdr api snapshot)" || { print "herdr unreachable" >&2; exit 1 }

rows=""
typeset -A seen_path repo_of

# open workspaces: switch by workspace_id; remember worktree-backed paths so
# their worktree rows do not duplicate them
while IFS=$'\t' read -r id label wpath; do
  [[ -n "$id" ]] || continue
  if [[ -n "$wpath" ]]; then
    seen_path[$wpath]="$id"
    rows+="open   ${label}  (${wpath/#$HOME/~})"$'\t'"focus"$'\t'"$id"$'\n'
  else
    rows+="space  ${label}"$'\t'"focus"$'\t'"$id"$'\n'
  fi
done < <(print -rn -- "$snapshot" | jq -r '.result.snapshot.workspaces[] |
  [.workspace_id, .label, (.worktree.checkout_path // "")] | @tsv')

# repos: from workspace rows plus herdr's worktree root resolved to main repos
while IFS= read -r root; do [[ -n "$root" ]] && repo_of[$root]=1; done \
  < <(print -rn -- "$snapshot" |
      jq -r '.result.snapshot.workspaces[]?.worktree?.repo_root // empty' | sort -u)
for child in ~/.herdr/worktrees/*/*(N/); do
  common="$(git -C "$child" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" || continue
  repo_of[${common:h}]=1
done

for root in ${(k)repo_of}; do
  while IFS= read -r wt; do
    [[ -d "$wt" && -z "${seen_path[$wt]:-}" ]] || continue
    branch="$(git -C "$wt" branch --show-current 2>/dev/null)"
    rows+="wt     ${root:t}/${branch:-detached}  (${wt/#$HOME/~})"$'\t'"open"$'\t'"$wt"$'\t'"$root"$'\n'
  done < <(git -C "$root" worktree list --porcelain 2>/dev/null |
    awk '/^worktree /{print substr($0,10)}')
done

# folders with no workspace yet: offer create-ws. Roots: ~/Documents, its child
# groups (GitHub, local-projects), and ~.
for dir in ~/Documents/*(N/) ~/Documents/GitHub/*(N/) ~/Documents/local-projects/*(N/); do
  [[ -z "${seen_path[$dir]:-}" ]] || continue
  rows+="new    ${dir/#$HOME/~}  [create-ws]"$'\t'"create"$'\t'"$dir"$'\n'
done

[[ -n "$rows" ]] || { print "nothing to open" >&2; exit 1 }

pick="$(print -rn -- "$rows" |
  fzf --with-nth=1 --delimiter=$'\t' --prompt='> ' --height=100% --no-sort \
      --header='enter: switch/open')" || exit 0
parts=("${(@s:	:)pick}")
case "$parts[2]" in
  focus) herdr workspace focus "$parts[3]" > /dev/null ;;
  open)  herdr worktree open --cwd "$parts[4]" --path "$parts[3]" > /dev/null ;;
  create) herdr workspace create --cwd "$parts[3]" --label "${parts[3]:t}" > /dev/null ;;
esac
