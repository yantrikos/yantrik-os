#!/bin/bash
# Prove that our apps can account for themselves.
#
# Starts Notes and Email, then talks to their sockets from an unrelated process — the same
# position the companion is in. A pass means the mind can read what a Yantrik app is showing and
# steer it without a screenshot, a vision model, or a synthetic click.
#
# Two apps, deliberately: Notes has one open document, Email has a folder, a list, a selection and
# a draft. If one trait carries both, it will carry the rest.
set -u

export DISPLAY=:0 SLINT_BACKEND=winit-software
unset XDG_RUNTIME_DIR          # so the sockets land in /tmp/yantrik-<uid>, which we can glob
TARGET=${TARGET:-/home/yantrik/target-yantrik/fast}
cd /home/yantrik/yantrik-run || exit 1

pkill -f "fast/yantrik-notes" 2>/dev/null
pkill -f "fast/yantrik-email" 2>/dev/null
sleep 1
rm -f /tmp/yantrik-*/app-notes.sock /tmp/yantrik-*/app-email.sock

setsid nohup "$TARGET/yantrik-notes" > notes_control.log 2>&1 &
# Demo mode: this box has no mail account, and a surface with nothing behind it proves nothing.
YANTRIK_EMAIL_DEMO=1 setsid nohup "$TARGET/yantrik-email" > email_control.log 2>&1 &
sleep 7
echo "notes: $(pgrep -cf 'fast/yantrik-notes') alive, email: $(pgrep -cf 'fast/yantrik-email') alive"

python3 - <<'PY'
import glob, json, socket, sys

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
        print(f"  {label}: ERROR {r['error']['message']}")
        return None
    payload = r["result"]
    if isinstance(payload, dict) and set(payload) == {"result"}:
        return payload["result"]
    return payload

def error_of(r):
    return (r.get("error") or {}).get("message", "")

# ── Notes ────────────────────────────────────────────────────────────
print("── notes ──")
path = socket_for("notes")
if not path:
    sys.exit("FAIL: notes published no control socket")

view = result(call(path, "app.describe", {}), "describe")
if view is None:
    sys.exit("FAIL: notes app.describe returned an error")
print(f"summary : {view['summary']}")
print(f"actions : {[a['name'] for a in view['actions']]}")
titles = [n["title"] for n in view["state"]["notes"]]
print(f"vault   : {view['state']['note_count']} notes, e.g. {titles[:3]}")

# A wrong call must correct itself rather than fail blankly — a model reads these.
msg = error_of(call(path, "app.act", {"action": "open_note", "args": {}}))
print(f"no arg  -> {msg!r}")
if "title" not in msg:
    fails.append("notes: a missing argument was not named")

msg = error_of(call(path, "app.act", {"action": "nope", "args": {}}))
print(f"bad act -> {msg!r}")
if "open_note" not in msg:
    fails.append("notes: an unknown action did not list the real ones")

if titles:
    target = titles[0]
    opened = result(call(path, "app.act", {"action": "open_note", "args": {"title": target}}), "open_note")
    print(f"open    -> {opened}")
    after = result(call(path, "app.describe", {}), "describe")
    print(f"now     : {after['summary']}")
    if after["state"]["title"] != target:
        fails.append(f"notes: opened {after['state']['title']!r}, asked for {target!r}")
else:
    print("(vault is empty — skipping the open leg)")

found = result(call(path, "app.act", {"action": "search", "args": {"query": "zzz-no-such-note"}}), "search")
print(f"search  -> {found}")
if found is None or found.get("matched") != 0:
    fails.append("notes: a search for nothing still matched something")
call(path, "app.act", {"action": "search", "args": {"query": ""}})

# ── Email ────────────────────────────────────────────────────────────
print()
print("── email ──")
path = socket_for("email")
if not path:
    fails.append("email published no control socket")
else:
    view = result(call(path, "app.describe", {}), "describe")
    if view is None:
        fails.append("email app.describe returned an error")
    else:
        print(f"summary : {view['summary']}")
        print(f"actions : {[a['name'] for a in view['actions']]}")
        state = view["state"]
        subjects = [m["subject"] for m in state.get("messages", [])]
        print(f"mailbox : {state.get('folder')}, {state.get('unread')} unread of {state.get('total')}")
        print(f"subjects: {subjects[:3]}")

        if not state.get("has_account"):
            fails.append("email: demo mode did not produce an account")
        elif subjects:
            want = subjects[0]
            opened = result(call(path, "app.act", {"action": "open_message", "args": {"which": want}}), "open_message")
            print(f"open    -> {opened}")
            after = result(call(path, "app.describe", {}), "describe")
            print(f"now     : {after['summary']}")
            body = (after["state"].get("open_message") or {}).get("body", "")
            print(f"body    : {len(body)} chars readable without a screenshot")
            if (after["state"].get("open_message") or {}).get("subject") != want:
                fails.append(f"email: open_message did not open {want!r}")
            if not body:
                fails.append("email: the open message reported no body")

        # Composing must fill a draft and stop there.
        drafted = result(call(path, "app.act", {"action": "compose", "args": {
            "to": "someone@example.com", "subject": "Probe", "body": "Drafted by the probe."}}), "compose")
        print(f"compose -> {drafted}")
        after = result(call(path, "app.describe", {}), "describe")
        if not after["state"].get("composing"):
            fails.append("email: compose did not open the composer")

        msg = error_of(call(path, "app.act", {"action": "send", "args": {}}))
        print(f"send    -> {msg!r}")
        if "unknown action" not in msg:
            fails.append("email: sending must not be on this surface")

print()
if fails:
    print("FAILED:")
    for f in fails:
        print("  -", f)
    sys.exit(1)
print("PASS: both apps describe themselves and take instruction over the bus")
PY
STATUS=$?

pkill -f "fast/yantrik-notes" 2>/dev/null
pkill -f "fast/yantrik-email" 2>/dev/null
exit $STATUS
