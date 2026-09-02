#!/usr/bin/env bash
# run_evals.sh — dispatcher for per-artifact eval harnesses. resolves
# <skills|agents|workflows>/<artifact>/evals/run.sh and execs it with the
# remaining args unchanged; --all runs every harness and prints a summary.
# usage: ./run_evals.sh <artifact-name> [args...]
#        ./run_evals.sh --all
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SEARCH_DIRS=(skills agents workflows)

usage() {
  echo "usage: $0 <artifact-name> [args...] | --all" >&2
  exit 2
}

[[ $# -ge 1 ]] || usage

if [[ "$1" == "--all" ]]; then
  [[ $# -eq 1 ]] || usage
  pass=()
  fail=()
  for dir in "${SEARCH_DIRS[@]}"; do
    for harness in "$REPO_ROOT/$dir"/*/evals/run.sh; do
      [[ -f "$harness" ]] || continue
      artifact="$dir/$(basename "$(dirname "$(dirname "$harness")")")"
      echo "== $artifact"
      if "$harness"; then
        pass+=("$artifact")
      else
        fail+=("$artifact")
      fi
    done
  done
  echo
  echo "passed: ${#pass[@]}  failed: ${#fail[@]}"
  for artifact in ${fail[@]+"${fail[@]}"}; do
    echo "FAIL $artifact"
  done
  [[ ${#fail[@]} -eq 0 ]]
  exit
fi

name="$1"
shift
for dir in "${SEARCH_DIRS[@]}"; do
  harness="$REPO_ROOT/$dir/$name/evals/run.sh"
  if [[ -f "$harness" ]]; then
    exec "$harness" "$@"
  fi
  if [[ -d "$REPO_ROOT/$dir/$name" ]]; then
    echo "artifact $dir/$name has no evals/run.sh" >&2
    exit 1
  fi
done
echo "artifact not found: $name (looked in ${SEARCH_DIRS[*]} under $REPO_ROOT)" >&2
exit 1
