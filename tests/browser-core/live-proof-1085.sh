#!/usr/bin/env bash
# #1085 on a desktop, with REAL Mind turns: with the person's Browser open first, a Mind's
# web_go, web_tabs and web_read are refused and the person's tabs do not change; on a cold start
# they work, in Mind View's own browser.
#
#   bash tests/browser-core/live-proof-1085.sh          # as the person, in their session
#
# How it must be run. The Mind is asked, in the person's chat (`yos act shell send_message`), the
# way the person asks it, and whatever mind is answering takes the turn. It must be attached and
# able to use web_go / web_tabs / web_read (Hermes, OpenClaw, Pi, DeepSeek, or the Yantrik mind).
# The script does not call `yos` as the mind account: the shell refuses a mind-uid call that
# carries no token before any surface sees it, so only a turn the mind takes itself reaches the
# browser as a Mind does. The tool results are read back with `chat_view`.
#
# Needs: yos on PATH, python3, curl, ss, a mind answering turns, and no Chromium open. Prints PASS or
# FAIL per check and exits non-zero on any FAIL. Leaves no browser open. A turn that does not
# finish within TURN_SECONDS (default 180) fails the check it belongs to.
set -u

PORT=9222
TURN_SECONDS=${TURN_SECONDS:-180}
FAILS=0

pass() { echo "PASS  $*"; }
fail() { echo "FAIL  $*"; FAILS=$((FAILS + 1)); }
check() { local what=$1; shift; if "$@"; then pass "$what"; else fail "$what"; fi; }

tabs_json() { curl -s --max-time 3 "http://127.0.0.1:$PORT/json/list" | python3 -c '
import json, sys
print(json.dumps(sorted((t["id"], t["url"], t["title"]) for t in json.load(sys.stdin) if t["type"] == "page")))'; }
listener_pid() { ss -ltnpH "sport = :$PORT" 2>/dev/null | sed -n 's/.*pid=\([0-9]*\).*/\1/p' | head -1; }
close_browser() { local p; p=$(listener_pid); [ -n "$p" ] && kill "$p" && for _ in $(seq 50); do [ -z "$(listener_pid)" ] && return; sleep 0.2; done; }
wait_port() { for _ in $(seq 100); do [ -n "$(listener_pid)" ] && return 0; sleep 0.2; done; return 1; }

# One real turn: a new chat, the request, then the chat read back once the mind has gone quiet
# (the transcript stops changing for 15 s) after having used the three tools, or on timeout.
turn() {
  local prompt=$1 out=$2 last="" same=0 start=$SECONDS now
  yos act shell new_chat >/dev/null 2>&1
  yos act shell send_message "text=$prompt" >/dev/null 2>&1 || { echo "could not send the message" >"$out"; return 1; }
  while [ $((SECONDS - start)) -lt "$TURN_SECONDS" ]; do
    sleep 5
    now=$(yos act shell chat_view 2>&1)
    printf '%s' "$now" >"$out"
    if [ "$now" = "$last" ]; then same=$((same + 5)); else same=0; last=$now; fi
    if [ "$same" -ge 15 ] && echo "$now" | grep -q "web_read"; then return 0; fi
  done
  return 0
}

# A real page, not about:blank: a Mind's own outbound check rejects an address that is not a web page.
PAGE_URL=${PAGE_URL:-http://example.com/}
ASK="Use your web tools, in this order: web_go to $PAGE_URL, then web_tabs, then web_read. After each, tell me exactly what the tool answered, word for word. Do not use any other tool and do not try another way if one is refused."

command -v yos >/dev/null || { echo "FAIL  yos is not on PATH"; exit 1; }
[ -z "$(listener_pid)" ] || { echo "FAIL  something already listens on $PORT; close every Chromium first"; exit 1; }
trap close_browser EXIT
W=$(mktemp -d)

echo "== A. the person's Browser first, then a real Mind turn"
yos act shell open_app name=browser >/dev/null 2>&1
wait_port || { echo "FAIL  the person's Browser did not open its debugging port"; exit 1; }
sleep 3
yos act browser go url=about:blank >/dev/null 2>&1
BEFORE=$(tabs_json)
echo "the person's tabs: $BEFORE"
turn "$ASK" "$W/a.txt"
check "the Mind's web tools were refused (Nothing was done)" grep -q "Nothing was done" "$W/a.txt"
check "all three were: at least three refusals in the turn" \
  sh -c '[ "$(grep -o "Nothing was done" "$1" | wc -l)" -ge 3 ]' _ "$W/a.txt"
check "the person's tabs are unchanged" [ "$(tabs_json)" = "$BEFORE" ]
check "the person's own act still works" sh -c 'yos act browser tabs 2>&1 | grep -q about:blank'
close_browser

echo "== B. a cold start: the Mind's own browser"
turn "$ASK" "$W/b.txt"
wait_port || fail "no browser came up for the Mind"
check "the Mind's web tools worked (no refusal in the turn)" sh -c '! grep -q "Nothing was done" "$1"' _ "$W/b.txt"
check "and its browser has Mind View's profile" \
  sh -c 'tr "\0" " " < /proc/$1/cmdline | grep -q "mind-view-browser/chromium"' _ "$(listener_pid)"
check "the person's own go is refused while Mind View's browser holds the port" \
  sh -c 'yos act browser go url=about:blank 2>&1 | grep -q "Nothing was done"'
close_browser

echo
echo "transcripts: $W/a.txt $W/b.txt"
if [ "$FAILS" -eq 0 ]; then echo "ALL PASS"; else echo "$FAILS FAILED"; exit 1; fi
