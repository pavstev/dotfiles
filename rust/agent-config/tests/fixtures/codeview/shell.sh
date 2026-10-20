#!/usr/bin/env bash
set -eu

greet() {
  local who="${1:-world}"
  echo "hello ${who}"
}

function cleanup {
  # remove temp } files {
  rm -rf "${TMPDIR:-/tmp}/work"
  echo '}'
}

function build() {
  inner_helper() {
    echo "x" > /dev/null
  }
  inner_helper
  if [ -n "$1" ]; then
    echo "{"
  fi
}

deploy () { echo deploy; }

greet
cleanup
