#!/usr/bin/env bash
# Hi-Fi demo — exercises the CLI surface end to end.
# Run from the repo root:  ./scripts/demo.sh
set -euo pipefail

cd "$(dirname "$0")/.."
HIFI=build/Hi-Fi.app/Contents/MacOS/hifi

if [[ ! -x "$HIFI" ]]; then
  echo "no build yet — running scripts/bundle.sh"
  ./scripts/bundle.sh
fi

step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

step "ping (launches the app if needed)"
"$HIFI" ping

step "open a docs page"
"$HIFI" tab open https://developer.mozilla.org --json

step "grab its id"
TAB=$("$HIFI" tab list --window --json | python3 -c '
import json,sys
r=json.load(sys.stdin)["result"]["rows"]
print([t for t in r if "mozilla" in (t.get("url") or "")][-1]["id"])')
echo "tab: $TAB"

step "split a terminal 40% to the right"
"$HIFI" tab open hifi://terminal --right-of "$TAB" --size 40% --json

step "make a group around it"
"$HIFI" group create "docs + terminal" --tab "$TAB" --json

step "list tabs"
"$HIFI" tab list --window

step "page snapshot (first 600 chars)"
"$HIFI" page snapshot --tab "$TAB" | head -c 600; echo

step "done — the app is left running so you can poke at it"
