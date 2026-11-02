#!/usr/bin/env bash
# #1085 on a machine: with the person's Browser open first, a Mind's web tools are refused and the
# person's tabs do not change; on a cold start they work, in Mind View's own browser.
#
#   sudo -v && bash tests/browser-core/live-proof-1085.sh      # as the person, in their session
#
# Needs: the desktop running, no Chromium open, python3, curl, ss, and passwordless sudo to the
# mind account (`sudo -n -u yantrik-mind`), which is how a Mind reaches the desktop (the mind
# door, YANTRIK_MIND_RUN). A Mind's web_go / web_tabs / web_read are these acts on the browser:
# web_go = go, web_tabs = tabs, web_read = read. Prints PASS or FAIL per check and exits non-zero
# on any FAIL. Leaves no browser open.
set -u

MIND_USER=${MIND_USER:-yantrik-mind}
MIND_RUN=${YANTRIK_MIND_RUN:-/run/yantrik-minds}
PORT=9222
FAILS=0

pass() { echo "PASS  $*"; }
fail() { echo "FAIL  $*"; FAILS=$((FAILS + 1)); }
check() { local what=$1; shift; if "$@"; then pass "$what"; else fail "$what"; fi; }

person() { yos "$@" 2>&1; }
mind() { sudo -n -u "$MIND_USER" env YANTRIK_MIND_RUN="$MIND_RUN" yos "$@" 2>&1; }
tabs_json() { curl -s --max-time 3 "http://127.0.0.1:$PORT/json/list" | python3 -c '
import json, sys
print(json.dumps(sorted((t["id"], t["url"], t["title"]) for t in json.load(sys.stdin) if t["type"] == "page")))'; }
listener_pid() { ss -ltnpH "sport = :$PORT" 2>/dev/null | sed -n 's/.*pid=\([0-9]*\).*/\1/p' | head -1; }
close_browser() { local p; p=$(listener_pid); [ -n "$p" ] && kill "$p" && for _ in $(seq 50); do [ -z "$(listener_pid)" ] && return; sleep 0.2; done; }
wait_port() { for _ in $(seq 100); do [ -n "$(listener_pid)" ] && return 0; sleep 0.2; done; return 1; }

command -v yos >/dev/null || { echo "FAIL  yos is not on PATH"; exit 1; }
sudo -n -u "$MIND_USER" true 2>/dev/null || { echo "FAIL  cannot run as $MIND_USER with sudo -n; run this where that works"; exit 1; }
[ -z "$(listener_pid)" ] || { echo "FAIL  something already listens on $PORT; close every Chromium first"; exit 1; }
trap close_browser EXIT

echo "== A. the person's Browser first"
person act shell open_app name=browser >/dev/null
wait_port || { echo "FAIL  the person's Browser did not open its debugging port"; exit 1; }
sleep 3
person act browser go url=about:blank >/dev/null
BEFORE=$(tabs_json)
echo "the person's tabs: $BEFORE"
for act in "go url=https://example.com/" tabs read; do
  out=$(mind act browser $act)
  check "a Mind's '$act' is refused (nothing was done)" \
    sh -c 'echo "$1" | grep -q "Nothing was done"' _ "$out"
  check "and shows no tab or page ($act)" \
    sh -c '! echo "$1" | grep -qi "about:blank\|their tab\|\"tabs\""' _ "$out"
done
check "the person's tabs are unchanged" [ "$(tabs_json)" = "$BEFORE" ]
check "the person's own act still works" sh -c 'yos act browser tabs 2>&1 | grep -q about:blank'
close_browser

echo "== B. a cold start: Mind View's browser"
out=$(mind act shell open_app name=browser)
wait_port || { echo "FAIL  Mind View's browser did not open its debugging port ($out)"; exit 1; }
sleep 3
check "its profile is Mind View's" \
  sh -c 'tr "\0" "\n" < /proc/$1/cmdline | grep -q "^--user-data-dir=.*/mind-view-browser/chromium$"' _ "$(listener_pid)"
out=$(mind act browser go url=about:blank)
check "a Mind's go works" sh -c '! echo "$1" | grep -q "Nothing was done"' _ "$out"
check "a Mind's tabs works" sh -c 'sudo -n -u "$1" env YANTRIK_MIND_RUN="$2" yos act browser tabs 2>&1 | grep -q about:blank' _ "$MIND_USER" "$MIND_RUN"
check "a Mind's read works" sh -c '! sudo -n -u "$1" env YANTRIK_MIND_RUN="$2" yos act browser read 2>&1 | grep -q "Nothing was done"' _ "$MIND_USER" "$MIND_RUN"
out=$(person act browser go url=about:blank)
check "the person's own go is refused while Mind View's browser holds the port" \
  sh -c 'echo "$1" | grep -q "Nothing was done"' _ "$out"
close_browser

echo
if [ "$FAILS" -eq 0 ]; then echo "ALL PASS"; else echo "$FAILS FAILED"; exit 1; fi
