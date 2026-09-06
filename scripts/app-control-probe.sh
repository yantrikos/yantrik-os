#!/bin/bash
# Prove that our apps can account for themselves.
#
# Starts every app that publishes a control surface, then talks to their sockets from an unrelated
# process — the same position the companion is in. A pass means the mind can read what a Yantrik
# app is showing and steer it without a screenshot, a vision model, or a synthetic click.
#
# Five apps, deliberately different shapes: one open document (notes), a folder with a list and a
# selection (email), a grid of dates (calendar), a machine that changes under you every two
# seconds (system-monitor), and a reading of somewhere else (weather). One trait carries all five.
set -u

export DISPLAY=:0 SLINT_BACKEND=winit-software
unset XDG_RUNTIME_DIR          # so the sockets land in /tmp/yantrik-<uid>, which we can glob
TARGET=${TARGET:-/home/yantrik/target-yantrik/fast}
cd /home/yantrik/yantrik-run || exit 1

APPS="notes email calendar system-monitor weather"

# Matched by path, not by name: pgrep/pkill compare against a 15-character process name, so
# `yantrik-system-monitor` matches nothing at all — silently, which once made this script report
# two running apps as dead and leave them running afterwards.
for app in $APPS; do
  pkill -f "$TARGET/yantrik-$app" 2>/dev/null
done
sleep 1
rm -f /tmp/yantrik-*/app-*.sock

for app in $APPS; do
  # Email demo mode: this box has no mail account, and a surface with nothing behind it proves
  # nothing. Every other app here has real data or a real service.
  if [ "$app" = "email" ]; then
    YANTRIK_EMAIL_DEMO=1 setsid nohup "$TARGET/yantrik-email" > "control_$app.log" 2>&1 &
  else
    setsid nohup "$TARGET/yantrik-$app" > "control_$app.log" 2>&1 &
  fi
done
sleep 9

alive=""
for app in $APPS; do
  alive="$alive $app:$(pgrep -cf "$TARGET/yantrik-$app")"
done
echo "running:$alive"

python3 - <<'PY'
import glob, json, socket, sys

APPS = ["notes", "email", "calendar", "system-monitor", "weather"]
fails = []

def socket_for(app):
    paths = glob.glob(f"/tmp/yantrik-*/app-{app}.sock")
    return paths[0] if paths else None

def call(path, method, params, timeout=10):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    s.connect(path)
    s.sendall((json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}) + "\n").encode())
    buf = b""
    while b"\n" not in buf:
        chunk = s.recv(65536)
        if not chunk:
            break
        buf += chunk
    s.close()
    return json.loads(buf.decode().splitlines()[0])

def result(r, label):
    """Unwrap the JSON-RPC envelope, then app.act's own {"result": ...} wrapper."""
    if r.get("error") is not None:
        return None
    payload = r["result"]
    if isinstance(payload, dict) and set(payload) == {"result"}:
        return payload["result"]
    return payload

def error_of(r):
    return (r.get("error") or {}).get("message", "")

# ── The contract every surface owes, whatever the app ────────────────
views = {}
for app in APPS:
    path = socket_for(app)
    if not path:
        fails.append(f"{app}: published no control socket")
        continue

    view = result(call(path, "app.describe", {}), "describe")
    if view is None:
        fails.append(f"{app}: app.describe returned an error")
        continue
    views[app] = (path, view)

    print(f"\n── {app} ──")
    print(f"summary : {view['summary']}")
    actions = view.get("actions", [])
    print(f"actions : {[a['name'] for a in actions]}")
    print(f"state   : {len(json.dumps(view['state']))} bytes, keys={sorted(view['state'])[:6]}...")

    if view.get("app") != app:
        fails.append(f"{app}: describes itself as {view.get('app')!r}")
    if not view["summary"].strip():
        fails.append(f"{app}: published an empty summary")
    if not isinstance(view.get("state"), dict) or not view["state"]:
        fails.append(f"{app}: published no state")
    if not actions:
        fails.append(f"{app}: published no actions")

    # Every action declares its risk, and every argument is typed and described.
    for a in actions:
        if a.get("permission") not in ("safe", "standard", "sensitive", "dangerous"):
            fails.append(f"{app}.{a['name']}: risk is {a.get('permission')!r}")
        for name, spec in a["parameters"]["properties"].items():
            if spec.get("type") not in ("string", "number", "integer", "boolean"):
                fails.append(f"{app}.{a['name']}({name}): type is {spec.get('type')!r}")

    # A wrong call must correct itself — a model reads these errors.
    msg = error_of(call(path, "app.act", {"action": "definitely-not-an-action", "args": {}}))
    if not all(a["name"] in msg for a in actions):
        fails.append(f"{app}: an unknown action did not list the real ones — {msg!r}")

    needs_arg = next((a for a in actions if a["parameters"]["required"]), None)
    if needs_arg:
        want = needs_arg["parameters"]["required"][0]
        msg = error_of(call(path, "app.act", {"action": needs_arg["name"], "args": {}}))
        if want not in msg:
            fails.append(f"{app}: a missing `{want}` was not named — {msg!r}")
        else:
            print(f"errors  : {msg}")

# ── What each one specifically owes ──────────────────────────────────
print()

if "notes" in views:
    path, view = views["notes"]
    titles = [n["title"] for n in view["state"]["notes"]]
    if titles:
        result(call(path, "app.act", {"action": "open_note", "args": {"title": titles[0]}}), "open")
        after = result(call(path, "app.describe", {}), "describe")
        print(f"notes           : opened {after['state']['title']!r}")
        if after["state"]["title"] != titles[0]:
            fails.append(f"notes: opened {after['state']['title']!r}, asked for {titles[0]!r}")

if "email" in views:
    path, view = views["email"]
    subjects = [m["subject"] for m in view["state"].get("messages", [])]
    if subjects:
        result(call(path, "app.act", {"action": "open_message", "args": {"which": subjects[0]}}), "open")
        after = result(call(path, "app.describe", {}), "describe")
        body = (after["state"].get("open_message") or {}).get("body", "")
        print(f"email           : {len(body)} chars of body readable without a screenshot")
        if not body:
            fails.append("email: the open message reported no body")
    # Drafting is offered; sending is not, and that is the point.
    if any(a["name"] == "send" for a in view["actions"]):
        fails.append("email: sending must not be on this surface")

if "calendar" in views:
    path, view = views["calendar"]
    before = view["state"]["month"]
    moved = result(call(path, "app.act", {"action": "show_month", "args": {"direction": "next"}}), "month")
    print(f"calendar        : {before} → {moved['showing'] if moved else '?'}")
    if not moved or moved["showing"] == before:
        fails.append("calendar: show_month did not move the month")
    result(call(path, "app.act", {"action": "go_to_today", "args": {}}), "today")

if "system-monitor" in views:
    path, view = views["system-monitor"]
    st = view["state"]
    top = st.get("top_processes", [])
    print(f"system-monitor  : {st.get('health')}, {len(top)} processes named, {len(st.get('disks', []))} disks")
    if not top:
        fails.append("system-monitor: named no processes")
    elif not all(p.get("pid") and p.get("name") for p in top):
        fails.append("system-monitor: a process had no pid or name")
    # kill_process must be declared dangerous, not ride in as an ordinary view change.
    kill = next((a for a in view["actions"] if a["name"] == "kill_process"), None)
    if not kill or kill.get("permission") != "dangerous":
        fails.append("system-monitor: kill_process is not declared dangerous")
    else:
        print("                : kill_process declared dangerous")

if "weather" in views:
    path, view = views["weather"]
    st = view["state"]
    print(f"weather         : {st.get('location')}, {st.get('temperature')}, units={st.get('units')}")
    set_units = result(call(path, "app.act", {"action": "set_units", "args": {"units": "fahrenheit"}}), "units")
    again = result(call(path, "app.act", {"action": "set_units", "args": {"units": "fahrenheit"}}), "units")
    print(f"                : set_units → {set_units}, asked again → {again}")
    # Both halves matter: it must do what was asked, and asking twice must not undo it.
    if not set_units or set_units.get("units") != "fahrenheit":
        fails.append(f"weather: set_units did not reach fahrenheit — {set_units}")
    if set_units != again:
        fails.append(f"weather: set_units is not idempotent — {set_units} then {again}")
    result(call(path, "app.act", {"action": "set_units", "args": {"units": "celsius"}}), "units")

print()
if fails:
    print("FAILED:")
    for f in fails:
        print("  -", f)
    sys.exit(1)
print(f"PASS: {len(views)} apps describe themselves and take instruction over the bus")
PY
STATUS=$?

for app in $APPS; do
  pkill -f "$TARGET/yantrik-$app" 2>/dev/null
done
exit $STATUS
