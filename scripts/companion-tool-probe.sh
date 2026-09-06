#!/bin/bash
# Prove the companion's tools are reachable without a language model, and that they can read our
# own apps without a screenshot.
#
# The ~178 tools could only be invoked by persuading an LLM to choose one mid-answer.
# `companion.tool` calls them by name. This drives the whole chain from outside the process:
#
#   socket → companion RPC → worker → ToolRegistry (permission gate, audit) → app_ui tool
#          → app-notes.sock → the Notes window's own callbacks
#
# Nothing here asks the model for anything, but the worker is single-threaded, so the first call
# still queues behind the startup brief. That is why the first timeout is generous.
set -u

export DISPLAY=:0 SLINT_BACKEND=winit-software
unset XDG_RUNTIME_DIR
TARGET=${TARGET:-/home/yantrik/target-yantrik/fast}
CONFIG=${CONFIG:-/home/yantrik/yantrik-run/config.yaml}
cd /home/yantrik/yantrik-run || exit 1

pkill -x yantrik-ui 2>/dev/null
pkill -x yantrik-notes 2>/dev/null
sleep 2
rm -f /tmp/yantrik-*/companion.sock /tmp/yantrik-*/app-notes.sock

setsid nohup "$TARGET/yantrik-ui" "$CONFIG" > shell_tool.log 2>&1 &
setsid nohup "$TARGET/yantrik-notes" > notes_tool.log 2>&1 &
sleep 12
echo "shell: $(pgrep -cx yantrik-ui) alive, notes: $(pgrep -cx yantrik-notes) alive"

python3 - <<'PY'
import glob, json, socket, sys, time

paths = glob.glob("/tmp/yantrik-*/companion.sock")
if not paths:
    sys.exit("FAIL: the shell published no companion socket")
path = paths[0]

def call(method, params, timeout=200):
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

fails = []

# 1. The catalogue. This is also the queue test: it waits out the startup brief.
t0 = time.time()
r = call("companion.tools", {"timeout_ms": 180000})
if r.get("error"):
    sys.exit(f"FAIL: companion.tools → {r['error']['message']}")
catalog = r["result"]
tools, ceiling = catalog["tools"], catalog["ceiling"]
names = {t["name"] for t in tools}
cats = sorted({t["category"] for t in tools})
print(f"waited          : {time.time() - t0:.1f}s for the worker (startup brief in front of it)")
print(f"tools reachable : {len(tools)} in {len(cats)} categories, ceiling={ceiling}")
print(f"app tools       : {sorted(n for n in names if n.startswith(('list_apps', 'describe_app', 'app_action')))}")
if len(tools) < 100:
    fails.append(f"only {len(tools)} tools in the catalogue")
for want in ("list_apps", "describe_app"):
    if want not in names:
        fails.append(f"{want} is not registered")

def run(name, args, timeout_ms=60000):
    r = call("companion.tool", {"name": name, "args": args, "timeout_ms": timeout_ms})
    if r.get("error"):
        return f"ERROR: {r['error']['message']}"
    return r["result"]["result"]

# 2. A plain tool, no model involved. Picked from the catalogue rather than hardcoded, so the
#    probe cannot fail merely because a tool was renamed.
plain = next((t["name"] for t in tools if t["category"] == "time"), None)     or next((t["name"] for t in tools if t["category"] == "calculator"), None)
if plain:
    out = run(plain, {})
    print(f"{plain:<16}: {out.strip()[:90]}")
    if out.startswith(("Unknown tool", "ERROR")):
        fails.append(f"{plain} is in the catalogue but did not run: {out[:120]}")
else:
    print("(no time or calculator tool in the catalogue to sample)")

# 3. The mind reads its own desktop — semantically, not from pixels.
out = run("list_apps", {})
print("list_apps       :")
for line in out.splitlines():
    print(f"    {line}")
if "notes:" not in out:
    fails.append("list_apps did not see the running Notes window")

view = None
out = run("describe_app", {"app": "notes"})
try:
    view = json.loads(out)
    print(f"describe_app    : {view['summary']}")
    print(f"                  {view['state']['note_count']} notes, actions={[a['name'] for a in view['actions']]}")
except (ValueError, KeyError):
    fails.append(f"describe_app did not return a view: {out[:200]}")

# 4. And drives it — if the configured ceiling allows. app_action is Standard because it changes
#    what the user is looking at; reading state is Safe. A `safe` ceiling must refuse it, and that
#    refusal is as much a pass as the action succeeding.
if view and view["state"]["notes"]:
    target = view["state"]["notes"][0]["title"]
    out = run("app_action", {"app": "notes", "action": "open_note", "args": {"title": target}})
    print(f"app_action      : {out.strip()[:140]}")
    if ceiling == "safe":
        if "Permission denied" not in out:
            fails.append("a Standard tool ran under a safe ceiling — the gate did not hold")
    elif target not in out:
        fails.append(f"app_action did not open {target!r}: {out[:200]}")

# 5. A wrong app name explains itself instead of failing blankly.
out = run("describe_app", {"app": "no-such-app"})
print(f"missing app     : {out.strip()[:120]}")
if "notes" not in out:
    fails.append("an unknown app did not say which apps are open")

print()
if fails:
    print("FAILED:")
    for f in fails:
        print("  -", f)
    sys.exit(1)
print(f"PASS: tools callable by name under the {ceiling} ceiling, and the mind reads its own apps")
PY
STATUS=$?

pkill -x yantrik-ui 2>/dev/null
pkill -x yantrik-notes 2>/dev/null
exit $STATUS
