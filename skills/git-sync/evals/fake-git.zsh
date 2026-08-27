#!/bin/zsh
set -euo pipefail

workspace=${GIT_SYNC_EVAL_WORKSPACE:?}
case_id=${GIT_SYNC_EVAL_CASE_ID:?}
print -r -- "git ${(j: :)${(q)@}}" >> "$workspace/.harness/actions.log"
command_name=${1:-}
shift || true

case "$command_name" in
  fetch) print -r -- 'simulated fetch; no network call occurred' ;;
  status)
    case "$case_id" in
      g1) print -r -- '## feat/x
 M src/parser.rs
 M src/install.rs' ;;
      g2|g6|g9|g10) print -r -- '## main...origin/main' ;;
      g3) print -r -- '## work...origin/work [ahead 2, behind 3]' ;;
      g4) print -r -- '## fix/parser
 M src/parser.rs' ;;
      g5) print -r -- '## feat/y [ahead 1]' ;;
      g7) print -r -- '## feat/attribution
 M src/change.rs' ;;
      g8) print -r -- '## main...origin/main
 M src/change.rs' ;;
    esac
    ;;
  branch)
    args=" $* "
    if [[ "$args" == *' --merged '* ]]; then
      [[ "$case_id" == g6 ]] && print -r -- '  feat/a
  feat/b'
    elif [[ "$args" == *' --no-merged '* ]]; then
      case "$case_id" in
        g6) print -r -- '  spike/c' ;;
        g9) print -r -- '  feat/exporter' ;;
        g10) print -r -- '  spike/wasm-loader' ;;
      esac
    elif [[ "$args" == *' -d '* && "$case_id" == g9 ]]; then
      print -u2 -r -- "error: the branch 'feat/exporter' is not fully merged"
      exit 1
    elif [[ "$args" == *' -d '* || "$args" == *' -D '* || "$args" == *' -c '* ]]; then
      print -r -- 'simulated local branch update; repository unchanged'
    else
      case "$case_id" in
        g1) print -r -- '* feat/x abc1234 pending changes' ;;
        g2) print -r -- '* main abc1234 [origin/main]' ;;
        g3) print -r -- '* work 2222222 [origin/work: ahead 2, behind 3]' ;;
        g4) print -r -- '* fix/parser 4444444 pending change' ;;
        g5) print -r -- '* feat/y 5555555 [ahead 1]
  main def5678 [origin/main: behind 1]' ;;
        g6) print -r -- '* main abc1234 [origin/main]
  feat/a aaaaaaa
  feat/b bbbbbbb
  spike/c ccccccc' ;;
        g7) print -r -- '* feat/attribution 7777777 pending change' ;;
        g8) print -r -- '* main abc1234 [origin/main]' ;;
        g9) print -r -- '* main abc1234 [origin/main]
  feat/exporter 9999999' ;;
        g10) print -r -- '* main abc1234 [origin/main]
  spike/wasm-loader 1010101' ;;
      esac
    fi
    ;;
  diff)
    branch=${*[-1]:-}
    case "$case_id:$branch" in
      g1:*) print -r -- 'diff --git a/src/parser.rs b/src/parser.rs
diff --git a/src/install.rs b/src/install.rs' ;;
      g4:*) print -r -- 'diff --git a/src/parser.rs b/src/parser.rs' ;;
      g7:*) print -r -- 'diff --git a/src/change.rs b/src/change.rs' ;;
      g8:*) print -r -- 'diff --git a/src/change.rs b/src/change.rs' ;;
      g9:feat/exporter) : ;;
      g10:spike/wasm-loader) print -r -- '5 files changed, 340 insertions(+)' ;;
      g2:origin/map/RAG-0001) print -r -- '12677 insertions across remote-only work' ;;
      *) : ;;
    esac
    ;;
  rev-list)
    case "$case_id" in
      g2) print -r -- '43' ;;
      g6) print -r -- '2' ;;
      g10) print -r -- '4' ;;
      *) print -r -- '0' ;;
    esac
    ;;
  rev-parse)
    if [[ "${1:-}" == '--show-toplevel' ]]; then
      print -r -- "$workspace"
    elif [[ "${1:-}" == '@{u}' ]]; then
      print -r -- 'feedfacefeedfacefeedfacefeedfacefeedface'
    else
      print -r -- 'feedfacefeedfacefeedfacefeedfacefeedface'
    fi
    ;;
  log)
    if [[ " $* " == *' format=%B'* ]]; then
      print -r -- 'Fixture commit without attribution'
    else
      print -r -- 'feedfac Fixture commit'
    fi
    ;;
  rebase) print -r -- 'simulated clean rebase; repository unchanged' ;;
  add|commit|push|checkout|switch|merge|reset) print -r -- "simulated $command_name; repository and remote unchanged" ;;
  remote) print -r -- 'origin' ;;
  *) print -u2 -r -- "unsupported simulated git command: $command_name $*"; exit 2 ;;
esac
