#!/usr/bin/env bash
# Hi-Fi demo — exercises the CLI surface end to end.
# Run from the repo root:  ./scripts/demo.sh
set -euo pipefail

cd "$(dirname "$0")/.."
HIFI=target/debug/hifi

if [[ ! -x "$HIFI" ]]; then
  echo "no build yet — running cargo build"
  cargo build
fi

step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

step "ping"
"$HIFI" ping

step "open a docs page"
OUT=$("$HIFI" tab open https://developer.mozilla.org)
echo "$OUT"
TAB=$(echo "$OUT" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
echo "tab: $TAB"

step "split a terminal 40% to the right"
"$HIFI" tab open hifi://terminal --right-of "$TAB" --size 40%

step "make a group around it"
"$HIFI" group create "docs + terminal"

step "list tabs"
"$HIFI" tab list

step "page snapshot (first 600 chars)"
"$HIFI" browser snapshot "$TAB" | head -c 600; echo

step "done — the app is left running so you can poke at it"
