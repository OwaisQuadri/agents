#!/bin/zsh
set -eu
[[ -z "$(find . -type f -name '*.xml' -print -quit)" ]]
