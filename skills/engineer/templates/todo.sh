#!/bin/sh
set -eu
step1() {
  :
}
list() {
  printf '%s\n' \
    'step1 <task-id>'
}
"$@"
