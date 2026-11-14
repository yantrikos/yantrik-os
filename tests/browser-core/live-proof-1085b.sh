#!/usr/bin/env bash
# #1085, the straight fix, on a desktop with REAL Mind and Hermes turns and real tokens: a mind's web
# tools drive Mind View's browser (its port, its profile) and never the person's, in every startup order;
# the person's own `yos web` / `yos act browser` reach theirs; an update or a rollback leaves a working
# browser for both and loses nothing of the person's.
#
#   bash tests/browser-core/live-proof-1085b.sh          # as the person, in their session, on a VM
#
# The ports and the profile come from browsers.conf, the file the code reads, so this checks the
# system and not a second copy of its numbers.
#
# Real turns. The mind is asked in the person's chat (`yos act shell send_message`), the way the person
# asks it, and whatever mind is answering takes the turn: Hermes (no token, the person's account, found by
# its ancestry) or the Yantrik mind (an agent token, the mind account). Run it once for each; it prints
# which one answered. The script never calls `yos` as the mind account: the shell refuses that without a
# token before any surface sees it (the 520 gate found this on #1088), so only a turn the mind takes
# itself reaches the browser as a mind does. Real tokens: with sudo, the answering mind's own token is
# read from its process and used on a direct `yos act browser` call from the person's account, which must
# still be treated as a mind (section F); without sudo that section says SKIP.
#
# Orders (every one ends with the person's tabs as they were and the mind's pages in Mind View's browser):
#   A  the person's Browser first, then a mind turn         D  an update: Mind View's browser left on the
#   B  Mind View's browser first, then the person's             person's port by the old build, the person's
#   C  cold start: the mind turn starts Mind View's browser     Browser on the old flags, then the update step
#   E  a rollback: Mind View's browser on its port, then the rollback step     F  a real token, directly
#
# Needs: yos on PATH, python3, curl, ss, google-chrome or chromium, a mind answering turns, and no Chromium
# open. Leaves none open. PASS or FAIL per check, SKIP where it cannot run; exits non-zero on any FAIL.
# A turn that does not finish within TURN_SECONDS (default 180) fails the check it belongs to.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
CONF=""
for c in /opt/yantrik/share/browser/yantrik_browser/browsers.conf "$HERE/../../crates/yantrik-browser-ports/browsers.conf"; do
  [ -f "$c" ] && CONF=$c && break
done
[ -n "$CONF" ] || { echo "FAIL  browsers.conf not found"; exit 1; }
conf() { sed -n "s/^$1 *= *//p" "$CONF" | head -1; }
PERSON=$(conf person_port)
MIND=$(conf mind_view_port)
[ -n "$MIND" ] || { echo "SKIP  Mind View's browser has no DevTools port any more (#1200), so this proof, which reaches it by one, cannot run; see \"What VM 520 must prove\" on the #1200 PR"; exit 0; }
MV_PROFILE_TAIL=$(conf mind_view_profile)
STOP=$(dirname "$CONF")/stop_mind_view.py
TURN_SECONDS=${TURN_SECONDS:-180}
SITE_PORT=${SITE_PORT:-18791}
FAILS=0

pass() { echo "PASS  $*"; }
fail() { echo "FAIL  $*"; FAILS=$((FAILS + 1)); }
skip() { echo "SKIP  $*"; }
check() { local what=$1; shift; if "$@"; then pass "$what"; else fail "$what"; fi; }

command -v yos >/dev/null || { echo "FAIL  yos is not on PATH"; exit 1; }
CHROME=$(command -v chromium || command -v google-chrome || command -v chromium-browser) || { echo "FAIL  no Chromium"; exit 1; }
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
MV_PROFILE=$DATA/$MV_PROFILE_TAIL

# ── the machine ──
listener_pid() { ss -ltnpH "sport = :$1" 2>/dev/null | sed -n 's/.*pid=\([0-9]*\).*/\1/p' | head -1; }
# A browser is Mind View's by the profile it OWNS: the pid Chromium wrote into the profile's SingletonLock.
# Never by its command line, whose shape varies (one space-joined string, a trailing NUL) and proves nothing.
owns_mind_view() { [ "$(readlink "$MV_PROFILE/SingletonLock" 2>/dev/null | sed 's/.*-//')" = "$1" ]; }
tabs_of() { curl -s --max-time 3 "http://127.0.0.1:$1/json/list" | python3 -c '
import json, sys
print(json.dumps(sorted((t["url"], t["title"]) for t in json.load(sys.stdin) if t["type"] == "page")))' 2>/dev/null; }
wait_port() { for _ in $(seq 150); do [ -n "$(listener_pid "$1")" ] && return 0; sleep 0.2; done; return 1; }
close_port() { local p; p=$(listener_pid "$1"); [ -n "$p" ] || return 0; kill "$p"; for _ in $(seq 60); do [ -z "$(listener_pid "$1")" ] && return 0; sleep 0.2; done; kill -9 "$p"; }
# A stopped browser closes its port before it has finished exiting (and may stay a zombie a moment
# until its parent reaps it): "stopped" is the process gone or a zombie, waited for, not looked at once.
pid_gone() { for _ in $(seq 100); do [ -d "/proc/$1" ] && ! grep -q '^State:.*Z' "/proc/$1/status" 2>/dev/null || return 0; sleep 0.2; done; return 1; }
close_all() { close_port "$PERSON"; close_port "$MIND"; }
refused() { grep -q "Nothing was done" "$1"; }

# Two throwaway pages, signed into nothing.
W=$(mktemp -d)
printf '<!doctype html><title>THROWAWAY-PAGE-A</title><p>a</p>' >"$W/a.html"
printf '<!doctype html><title>THROWAWAY-PAGE-B</title><p>b</p>' >"$W/b.html"
python3 -m http.server "$SITE_PORT" --bind 127.0.0.1 --directory "$W" >/dev/null 2>&1 &
SITE_PID=$!
SITE=http://127.0.0.1:$SITE_PORT
cleanup() { close_all; kill "$SITE_PID" 2>/dev/null; }
trap cleanup EXIT
[ -z "$(listener_pid "$PERSON")$(listener_pid "$MIND")" ] || { echo "FAIL  something already listens on $PERSON or $MIND; close every Chromium first"; exit 1; }

# One real turn: a new chat, the request, then the chat read back once the mind has gone quiet (the
# transcript stops changing for 15 s) after it has used its tools, or on timeout.
turn() { # <prompt> <outfile>
  local prompt=$1 out=$2 last="" same=0 start=$SECONDS now
  yos act shell new_chat >/dev/null 2>&1
  yos act shell send_message "text=$prompt" >/dev/null 2>&1 || { echo "could not send the message" >"$out"; return 1; }
  while [ $((SECONDS - start)) -lt "$TURN_SECONDS" ]; do
    sleep 5
    now=$(yos act shell chat_view 2>&1)
    printf '%s' "$now" >"$out"
    if [ "$now" = "$last" ]; then same=$((same + 5)); else same=0; last=$now; fi
    if [ "$same" -ge 15 ] && echo "$now" | grep -q "web_read\|web_tabs"; then return 0; fi
  done
  return 0
}
ask() { # <page> : the same request every time
  echo "Use your web tools, in this order: web_go to $SITE/$1.html, then web_tabs, then web_read. After each, tell me exactly what the tool answered, word for word. If the browser is not open, os_act shell open_app name=browser first. Do not use any other tool and do not try another way if one is refused."
}
mind_turn() { turn "$(ask "$1")" "$2"; }   # real turn -> transcript

echo "== who answers: $(yos describe shell 2>/dev/null | grep -io '"mind[a-z_]*": *"[^"]*"' | head -2 | tr '\n' ' ')"

# The person's own use: `yos act browser` must land in THEIR browser.
person_go() { yos act browser go "url=$SITE/$1.html" 2>&1; }

# The person's Browser, opened the way the dock opens it.
open_persons() { yos act shell open_app name=browser >/dev/null 2>&1; wait_port "$PERSON" || return 1; sleep 3; }
persons_flags_ok() { # the person's port is held by a browser that does not own Mind View's profile
  local p; p=$(listener_pid "$PERSON"); [ -n "$p" ] && ! owns_mind_view "$p"
}
mind_view_holds() { # the browser on $MIND is Mind View's: its profile, and this account's
  local p; p=$(listener_pid "$MIND"); [ -n "$p" ] && owns_mind_view "$p"
}
# Mind View's browser as an OLD build started it: on the person's port, Mind View's profile.
start_legacy_mind_view() {
  mkdir -p "$MV_PROFILE"
  "$CHROME" --headless=new --no-first-run --no-sandbox --remote-debugging-address=127.0.0.1 \
    --remote-debugging-port="$PERSON" --user-data-dir="$MV_PROFILE" about:blank >/dev/null 2>&1 &
  wait_port "$PERSON"
}

# Both browsers up, one real turn each way, then everything the order must have left true.
settle() { # <label> <transcript> : the mind's page is in Mind View's browser, the person's in theirs
  local label=$1 t=$2
  check "$label: the mind's web tools were not refused" bash -c '! grep -q "Nothing was done" "$1"' _ "$t"
  check "$label: the mind's page is in Mind View's browser" bash -c 'tabs_of "$1" | grep -q THROWAWAY-PAGE-A' _ "$MIND"
  check "$label: Mind View's browser has Mind View's profile" mind_view_holds
  check "$label: and the mind's page is not in the person's browser" bash -c '! tabs_of "$1" | grep -q THROWAWAY-PAGE-A' _ "$PERSON"
}
export -f tabs_of listener_pid owns_mind_view mind_view_holds pid_gone 2>/dev/null
export MIND PERSON MV_PROFILE_TAIL MV_PROFILE

echo "== A. the person's Browser first, then a real mind turn"
open_persons || { echo "FAIL  the person's Browser did not open its debugging port"; exit 1; }
person_go b >/dev/null
check "A: the person's Browser owns no mind's profile" persons_flags_ok
BEFORE=$(tabs_of "$PERSON"); echo "the person's tabs: $BEFORE"
mind_turn a "$W/a.txt"; wait_port "$MIND"
settle A "$W/a.txt"
check "A: the person's tabs are exactly as they were" [ "$(tabs_of "$PERSON")" = "$BEFORE" ]
check "A: the person's own act reaches their browser" bash -c 'yos act browser tabs 2>&1 | grep -q THROWAWAY-PAGE-B'
check "A: and does not move the mind's" bash -c 'tabs_of "$1" | grep -q THROWAWAY-PAGE-A' _ "$MIND"
close_all

echo "== B. Mind View's browser first, then the person's Browser"
mind_turn a "$W/b1.txt"; wait_port "$MIND" || fail "B: no browser came up for the mind"
settle B "$W/b1.txt"
open_persons || { echo "FAIL  the person's Browser did not open its debugging port beside Mind View's"; exit 1; }
check "B: two browsers, two ports, two processes" bash -c '[ "$(listener_pid "$1")" != "$(listener_pid "$2")" ]' _ "$PERSON" "$MIND"
check "B: the person's owns no mind's profile" persons_flags_ok
person_go b >/dev/null
check "B: the person's own go lands in THEIR window, not the mind's" bash -c 'tabs_of "$1" | grep -q THROWAWAY-PAGE-B && ! tabs_of "$2" | grep -q THROWAWAY-PAGE-B' _ "$PERSON" "$MIND"
mind_turn a "$W/b2.txt"
check "B: a second mind turn still works, and still not in the person's" bash -c '! grep -q "Nothing was done" "$1" && ! tabs_of "$2" | grep -q THROWAWAY-PAGE-A' _ "$W/b2.txt" "$PERSON"
close_all

echo "== C. a cold start: the mind turn opens Mind View's browser, the person's was never open"
mind_turn a "$W/c.txt"; wait_port "$MIND" || fail "C: no browser came up for the mind"
settle C "$W/c.txt"
check "C: nothing listens on the person's port" bash -c '[ -z "$(listener_pid "$1")" ]' _ "$PERSON"
check "C: the person's own act is told their browser is not open, and nothing of the mind's is shown" \
  bash -c '! yos act browser tabs 2>&1 | grep -q THROWAWAY-PAGE-A'
close_all

echo "== D. an update: the old build left Mind View's browser on the person's port; the person's Browser is open on the old flags"
# The person's Browser on the old flags: a browser of their own on a different port is not the person's
# port, so the realistic state is built in two steps: Mind View's old browser, the update step, then theirs.
start_legacy_mind_view || { echo "FAIL  could not start the legacy Mind View browser"; exit 1; }
check "D: the legacy Mind View browser holds the person's port" bash -c 'owns_mind_view "$(listener_pid "$1")"' _ "$PERSON"
check "D: the person's own act is refused, not sent to it" bash -c 'yos act browser tabs 2>&1 | grep -q "Mind View"'
OLD_PID=$(listener_pid "$PERSON")
if [ -f "$STOP" ]; then python3 "$STOP" --user "$(id -un)" >/dev/null 2>&1; else fail "D: the update's step $STOP is not shipped"; fi
for _ in $(seq 50); do [ -z "$(listener_pid "$PERSON")" ] && break; sleep 0.2; done
check "D: the update step stopped it" bash -c '[ -z "$(listener_pid "$1")" ] && pid_gone "$2"' _ "$PERSON" "$OLD_PID"
open_persons || { echo "FAIL  the person's Browser did not open after the update step"; exit 1; }
person_go b >/dev/null
SIGNED_IN=$(tabs_of "$PERSON"); PERSON_PID=$(listener_pid "$PERSON")
mind_turn a "$W/d.txt"; wait_port "$MIND"
settle D "$W/d.txt"
check "D: the person's Browser is the same process, with the same tabs" bash -c '[ "$(listener_pid "$1")" = "$2" ] && [ "$(tabs_of "$1")" = "$3" ]' _ "$PERSON" "$PERSON_PID" "$SIGNED_IN"
echo "== E. a rollback: the update step runs before the old build starts, with the new build's Mind View browser up"
check "E: Mind View's browser is up on its own port" mind_view_holds
MIND_PID=$(listener_pid "$MIND")
python3 "$STOP" --user "$(id -un)" >/dev/null 2>&1
for _ in $(seq 50); do [ -z "$(listener_pid "$MIND")" ] && break; sleep 0.2; done
check "E: the rollback step stopped it (the old build starts its own on its own port)" bash -c 'pid_gone "$1"' _ "$MIND_PID"
check "E: and the person's Browser is untouched: same process, same tabs" bash -c '[ "$(listener_pid "$1")" = "$2" ] && [ "$(tabs_of "$1")" = "$3" ]' _ "$PERSON" "$PERSON_PID" "$SIGNED_IN"
check "E: yantrik-update runs that step on both sides of a restart" \
  bash -c 'f=/opt/yantrik/bin/yantrik-update; [ -f "$f" ] || f="$1"; [ "$(grep -c "^  stop_mind_view_browsers" "$f")" -ge 2 ]' _ "$HERE/../../deploy/yantrik-os/yantrik-update"
mind_turn a "$W/e.txt"; wait_port "$MIND"
check "E: a mind turn after the step opens a working Mind View browser again" bash -c '! grep -q "Nothing was done" "$1" && tabs_of "$2" | grep -q THROWAWAY-PAGE-A' _ "$W/e.txt" "$MIND"
close_all

echo "== F. a real token, used directly from the person's account"
TOKEN=""
if sudo -n true 2>/dev/null; then
  for pat in yantrik-mind hermes; do
    for pid in $(pgrep -f "$pat" 2>/dev/null | head -5); do
      TOKEN=$(sudo -n tr '\0' '\n' < "/proc/$pid/environ" 2>/dev/null | sed -n 's/^YANTRIK_AGENT_TOKEN=//p' | head -1)
      [ -n "$TOKEN" ] && break 2
    done
  done
fi
if [ -z "$TOKEN" ]; then
  skip "F: no agent token could be read from a running mind (needs sudo and a mind that holds one)"
else
  open_persons
  BEFORE=$(tabs_of "$PERSON")
  yos act browser go "url=$SITE/a.html" --agent-token "$TOKEN" >/dev/null 2>&1
  check "F: a call with a real token from the person's own account never touches the person's browser" [ "$(tabs_of "$PERSON")" = "$BEFORE" ]
  check "F: it is told Mind View's browser is not open (it is the mind's door, not theirs)" \
    bash -c 'yos act browser tabs --agent-token "$1" 2>&1 | grep -qi "not open\|Nothing was done"' _ "$TOKEN"
  close_all
fi

echo
echo "transcripts: $W/*.txt"
if [ "$FAILS" -eq 0 ]; then echo "ALL PASS"; else echo "$FAILS FAILED"; exit 1; fi
