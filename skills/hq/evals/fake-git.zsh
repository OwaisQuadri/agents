#!/bin/zsh
set -euo pipefail

print -r -- "git $*" >> "${HQ_EVAL_WORKSPACE:?}/.harness/actions.log"

case " $* " in
  *' rev-parse --is-inside-work-tree '*) print -r -- true ;;
  *' rev-parse --abbrev-ref HEAD '*) print -r -- main ;;
  *' rev-parse --show-toplevel '*) print -r -- "${HQ_EVAL_WORKSPACE}/projects/atlas" ;;
  *' status --porcelain '*) ;;
  *' worktree add '*) ;;
  *) ;;
esac
