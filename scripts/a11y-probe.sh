#!/bin/bash
# Can the companion read — and press — a window we did not write?
#
# Starts a real GTK application, then talks to the accessibility service from an unrelated process
# and checks three things:
#
#   1. the window is found, named, and attributed to a pid
#   2. its contents come back as labels, fields and buttons, not as pixels
#   3. a button can be pressed by name, through the same interface a screen reader uses
#
# The third is the one that matters. It is the app control surface argument extended to software
# we did not write: no synthetic pointer, no coordinates, and it works on a window that is not
# even on top.
#
# The accessibility bus is normally part of a desktop session. This script starts one, because a
# dev box has no session — which is also why the service connects lazily and retries.
set -u

TARGET=${TARGET:-/home/yantrik/target-yantrik/fast}
BIN="$TARGET/a11y-service"
LAB=/tmp/a11y-probe

[ -x "$BIN" ] || { echo "FAIL: $BIN is not built"; exit 1; }
command -v zenity >/dev/null || { echo "SKIP: no GTK application to read (apt install zenity)"; exit 0; }
command -v dbus-launch >/dev/null || { echo "SKIP: no dbus-launch (apt install dbus-x11)"; exit 0; }

pkill -f "$BIN" 2>/dev/null
pkill -f "zenity --forms" 2>/dev/null
pkill -f at-spi-bus-launcher 2>/dev/null
sleep 1
rm -rf "$LAB"; mkdir -p "$LAB"

# A session, an accessibility bus, and an application with the bridge loaded.
eval "$(dbus-launch --sh-syntax)"
export DBUS_SESSION_BUS_ADDRESS DBUS_SESSION_BUS_PID
/usr/libexec/at-spi-bus-launcher --launch-immediately > "$LAB/a11y-bus.log" 2>&1 &
sleep 2

export DISPLAY=${DISPLAY:-:0}
export GTK_MODULES=gail:atk-bridge
export NO_AT_BRIDGE=0
zenity --forms --title="Expenses" --text="March claim" \
  --add-entry="Merchant" --add-entry="Amount" > "$LAB/zenity-result.txt" 2>/dev/null &
ZPID=$!
sleep 4
if ! kill -0 "$ZPID" 2>/dev/null; then
  echo "SKIP: the GTK application would not start (no display?)"
  exit 0
fi
echo "zenity pid    : $ZPID"

# The service inherits this session, which is how it finds the accessibility bus at all.
setsid nohup env RUST_LOG=info "$BIN" > "$LAB/service.log" 2>&1 &
sleep 3

# The socket lands wherever the transport found a writable directory, which on a desktop is
# $XDG_RUNTIME_DIR. Ask the log rather than guess at it.
SOCK=$(grep -ao "/[^ ]*/a11y.sock" "$LAB/service.log" | head -1)
[ -n "$SOCK" ] || SOCK=$(ls "${XDG_RUNTIME_DIR:-/nonexistent}/yantrik/a11y.sock" \
  /run/yantrik/a11y.sock /tmp/yantrik-*/a11y.sock 2>/dev/null | head -1)
if [ -z "$SOCK" ]; then
  echo "FAIL: no a11y socket"
  cat "$LAB/service.log"
  kill $ZPID 2>/dev/null; pkill -f "$BIN"
  exit 1
fi
echo "socket        : $SOCK"

SOCK="$SOCK" ZPID="$ZPID" LAB="$LAB" python3 - <<'PY'
import json, os, socket, sys, time

sock_path, zpid, lab = os.environ["SOCK"], int(os.environ["ZPID"]), os.environ["LAB"]
fails = []

def call(method, params, timeout=40):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    s.connect(sock_path)
    s.sendall((json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}) + "\n").encode())
    buf = b""
    while b"\n" not in buf:
        chunk = s.recv(1 << 20)
        if not chunk:
            break
        buf += chunk
    s.close()
    reply = json.loads(buf.decode().splitlines()[0])
    if reply.get("error"):
        return None, reply["error"]["message"]
    return reply["result"], None

# ── 1. The window is found and attributed ────────────────────────────
result, err = call("a11y.windows", {})
if err:
    print(f"FAIL: a11y.windows -> {err}")
    sys.exit(1)
windows = result["windows"]
print(f"windows       : {result['count']}")
for w in windows:
    print(f"  {w['role']:<10} {w['title']!r} in {w['app']!r} (pid {w['pid']})")

target = next((w for w in windows if w["title"] == "Expenses"), None)
if not target:
    fails.append(f"the GTK window was not found: {[w['title'] for w in windows]}")
    print("FAILED:\n  -", fails[0])
    sys.exit(1)
if target["pid"] != zpid:
    fails.append(f"attributed to pid {target['pid']}, but the window belongs to {zpid}")

# ── 2. Its contents come back as meaning ─────────────────────────────
result, err = call("a11y.describe", {"window": target["id"]})
if err:
    print(f"FAIL: a11y.describe -> {err}")
    sys.exit(1)
print(f"summary       : {result['summary']}")
elements = result["elements"]
print(f"elements      : {len(elements)}")
for e in elements[:10]:
    bits = [f"{e['role']}"]
    if e.get("name"):
        bits.append(repr(e["name"]))
    if e.get("text"):
        bits.append(f"text={e['text']!r}")
    if e.get("actions"):
        bits.append(f"can={e['actions']}")
    print("   ", " ".join(bits))

names = {e.get("name", "") for e in elements}
for expected in ("Merchant", "Amount", "Cancel", "OK"):
    if expected not in names:
        fails.append(f"{expected!r} was not reported; got {sorted(n for n in names if n)}")

# Scaffolding must not survive the walk. A raw tree of this dialog is mostly unnamed panels.
scaffolding = [e for e in elements if not e.get("name") and not e.get("text") and not e.get("actions")]
if scaffolding:
    fails.append(f"{len(scaffolding)} elements carry nothing worth saying")

# The size of the answer is the whole argument against a screenshot.
size = len(json.dumps(result))
print(f"cost          : {size} bytes, no GPU, no model")
if size > 20000:
    fails.append(f"a glance should not cost {size} bytes")

# ── 3. A button can be pressed by name ───────────────────────────────
cancel = next((e for e in elements if e.get("name") == "Cancel" and e.get("actions")), None)
if not cancel:
    fails.append("the Cancel button published no action, so it could not be pressed")
else:
    # First, while the application is still there: a wrong action must correct itself the way
    # our own surfaces do. After Cancel the window is gone and every call fails at the D-Bus
    # layer instead, which would test nothing.
    _, err = call("a11y.act", {"element": cancel["id"], "action": "definitely-not-an-action"})
    print(f"bad action    : {err}")
    if not err or "offers" not in err:
        fails.append(f"a wrong action did not list the real ones: {err}")

    print(f"pressing      : {cancel['name']} via {cancel['actions']}")
    result, err = call("a11y.act", {"element": cancel["id"], "action": cancel["actions"][0]})
    if err:
        fails.append(f"a11y.act failed: {err}")
    else:
        print(f"acted         : {result['did']}")
        # The proof is not the reply; it is that the application did something.
        for _ in range(20):
            time.sleep(0.25)
            if not os.path.exists(f"/proc/{zpid}"):
                break
        if os.path.exists(f"/proc/{zpid}"):
            fails.append("the dialog is still open, so nothing was actually pressed")
        else:
            print("effect        : the dialog closed — a foreign window driven with no pointer")

print()
if fails:
    print("FAILED:")
    for f in fails:
        print("  -", f)
    sys.exit(1)
print("PASS: a window we did not write, read and driven without a single pixel")
PY
STATUS=$?

kill $ZPID 2>/dev/null
pkill -f "$BIN" 2>/dev/null
pkill -f at-spi-bus-launcher 2>/dev/null
[ -n "${DBUS_SESSION_BUS_PID:-}" ] && kill "$DBUS_SESSION_BUS_PID" 2>/dev/null
exit $STATUS
