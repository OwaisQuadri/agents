#!/bin/zsh
set -eu
[[ ! -d .maestro ]]
[[ -z "$(find . -type f -name '*.xml' -print -quit)" ]]
