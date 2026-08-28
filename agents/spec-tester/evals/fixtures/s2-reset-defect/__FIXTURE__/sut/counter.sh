#!/bin/sh
set -eu
state_file="$1"
command="$2"
case "$command" in
  incr)
    value=$(( $( [ -f "$state_file" ] && cat "$state_file" || printf '0\n' ) + 1 ))
    printf '%s\n' "$value" > "$state_file"
    printf '%s\n' "$value"
    ;;
  get)
    [ -f "$state_file" ] && cat "$state_file" || printf '0\n'
    ;;
  reset)
    printf '1\n' > "$state_file"
    printf 'reset\n'
    ;;
  *)
    printf 'unknown command\n' >&2
    exit 2
    ;;
esac
