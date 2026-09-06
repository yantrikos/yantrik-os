#!/bin/bash
# Does the periphery actually see, and does the eyelid actually close?
#
# Starts the perception service privileged, then checks four things from outside it:
#
#   1. it reports what happens, and names who did it
#   2. it says nothing about what is out of its scope
#   3. it cannot read outside its scope, as a kernel guarantee rather than a promise
#   4. it is no longer privileged while it goes on watching
#
# The last two are the ones worth the trouble. A watching daemon that asks to be trusted is a
# different kind of thing from one that has given its privileges back and can prove it.
#
# The process source is checked conditionally, and deliberately so: the kernel reports pids in the
# initial namespace, and inside a container or a WSL distro those cannot be resolved. Where that is
# the case the service must say so plainly, and the probe asserts *that* instead — a limitation
# stated is a pass; a limitation hidden is not.
set -u

TARGET=${TARGET:-/home/yantrik/target-yantrik/fast}
BIN="$TARGET/perception-service"
LAB=/tmp/perception-probe
CONFIG=$LAB/scope.yaml

command -v sudo >/dev/null || { echo "FAIL: needs sudo to open the privileged descriptors"; exit 1; }
[ -x "$BIN" ] || { echo "FAIL: $BIN is not built"; exit 1; }

sudo pkill -f "$BIN" 2>/dev/null
sleep 1
rm -rf "$LAB"
mkdir -p "$LAB/watched/nested" "$LAB/watched/secret" "$LAB/unwatched"

cat > "$CONFIG" <<YAML
watch:
  - $LAB/watched
never:
  - $LAB/watched/secret
enforce: true
YAML

sudo -b env -u XDG_RUNTIME_DIR YANTRIK_PERCEPTION_CONFIG="$CONFIG" RUST_LOG=info \
  "$BIN" > "$LAB/service.log" 2>&1
sleep 3

# The socket lands wherever the transport found a writable directory — as root that is
# /run/yantrik, which a user session cannot create. Ask the log rather than guess.
SOCK=$(grep -ao "/[^ ]*/perception.sock" "$LAB/service.log" | head -1)
[ -n "$SOCK" ] || SOCK=$(ls /run/yantrik/perception.sock /tmp/yantrik-*/perception.sock 2>/dev/null | head -1)
if [ -z "$SOCK" ]; then
  echo "FAIL: no perception socket"
  cat "$LAB/service.log"
  sudo pkill -f "$BIN"
  exit 1
fi
echo "socket        : $SOCK"

# The activity the service is supposed to notice, generated after it is listening.
( cd "$LAB/watched" && /bin/echo probe-marker > note.txt )
echo nested > "$LAB/watched/nested/deep.txt"
echo secret > "$LAB/watched/secret/private.txt"
echo elsewhere > "$LAB/unwatched/ignored.txt"
/bin/sleep 1 &
sleep 2

SOCK="$SOCK" LAB="$LAB" sudo -E python3 - <<'PY'
import json, os, socket, subprocess, sys, threading, time

sock_path, lab = os.environ["SOCK"], os.environ["LAB"]
fails = []

def call(method, params, timeout=40):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    s.connect(sock_path)
    s.sendall((json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}) + "\n").encode())
    buf = b""
    while b"\n" not in buf:
        chunk = s.recv(65536)
        if not chunk:
            break
        buf += chunk
    s.close()
    reply = json.loads(buf.decode().splitlines()[0])
    if reply.get("error"):
        sys.exit(f"FAIL: {method} -> {reply['error']['message']}")
    return reply["result"]

# ── 3 and 4: what is holding it ──────────────────────────────────────
scope = call("perception.scope", {})
snap = call("perception.snapshot", {})
pid = snap["pid"]

print(f"pid           : {pid}")
print(f"enforcement   : {scope['enforcement']}")
print(f"verified      : {scope['verified']}")
print(f"capabilities  : {scope['capabilities']}")

if "landlock ABI" not in scope["enforcement"]:
    fails.append(f"Landlock was not applied: {scope['enforcement']}")
# The canary is the real test: a ruleset the kernel accepted is not a ruleset that denies anything.
if not scope["verified"].startswith("confirmed"):
    fails.append(f"the eyelid did not close: {scope['verified']}")

# Read the capabilities of the service itself, by the pid it reported. Matching the binary path
# with pgrep finds the sudo that launched it just as readily, and sudo holds everything.
capeff = ""
with open(f"/proc/{pid}/status") as f:
    for line in f:
        if line.startswith("CapEff:"):
            capeff = line.split()[1]
print(f"CapEff        : {capeff}")
if capeff.strip("0") != "":
    fails.append(f"capabilities were not dropped: CapEff {capeff}")

# ── 1 and 2: what it saw ─────────────────────────────────────────────
page = call("perception.since", {"seq": 0})
print(f"observations  : {len(page['observations'])} (next_seq {page['next_seq']}, missed {page['missed']})")

by_kind = {}
for o in page["observations"]:
    by_kind.setdefault(o["kind"]["type"], []).append(o)

saved = by_kind.get("saved", [])
launched = by_kind.get("launched", [])
failures = by_kind.get("source_failed", [])
for o in saved[:4]:
    print(f"  saved       : {o['summary']}   (salience {o['salience']:.2f})")
for o in launched[:3]:
    print(f"  launched    : {o['summary']}")
for o in failures:
    print(f"  unavailable : {o['summary'][:150]}")

paths = {o["kind"]["path"] for o in saved}
if f"{lab}/watched/note.txt" not in paths:
    fails.append(f"a save in scope was not reported: {sorted(paths)}")
if f"{lab}/watched/nested/deep.txt" not in paths:
    fails.append("a save in a nested watched directory was not reported")

# The two that must be silent, for different reasons: one is outside the watch list entirely, the
# other sits inside it and is excluded by name.
if any(f"{lab}/unwatched" in p for p in paths):
    fails.append("a save outside the scope was reported")
if any("/secret/" in p for p in paths):
    fails.append("a save in an excluded directory was reported")

# Attribution is the whole reason this is fanotify rather than inotify.
named = [o for o in saved if o.get("actor", {}).get("name")]
if not named:
    fails.append("no save named the process that made it")
else:
    print(f"attribution   : {named[0]['actor']['name']} (pid {named[0]['actor']['pid']})")

# The process source, where the kernel's pids mean anything here.
namespaced = any("namespace" in o["kind"]["reason"] for o in failures if o["kind"]["source"] == "processes")
if namespaced:
    reason = next(o["kind"]["reason"] for o in failures if o["kind"]["source"] == "processes")
    print("processes     : unavailable here, and said so")
    if "File events are unaffected" not in reason:
        fails.append("the limitation must not read as though the whole service were blind")
    if launched:
        fails.append("launches were reported despite pids being unresolvable")
elif not launched:
    fails.append("no process launch was reported, and no reason was given for its absence")
elif not any(o.get("actor", {}).get("name") for o in launched):
    fails.append("no launch named its process")

# ── The long poll parks rather than spinning ─────────────────────────
started = time.time()
result = {}

def waiter():
    result["page"] = call("perception.since", {"seq": page["next_seq"], "wait_ms": 10000})

t = threading.Thread(target=waiter)
t.start()
time.sleep(0.5)
# Something the service is certain to notice on any machine: a save inside the scope.
with open(f"{lab}/watched/late.txt", "w") as f:
    f.write("woken\n")
t.join(timeout=15)
elapsed = time.time() - started
woke = result.get("page", {}).get("observations", [])
print(f"long poll     : woke after {elapsed:.1f}s with {len(woke)} new")
if not woke:
    fails.append("a waiting reader was not woken by activity")
elif elapsed > 8:
    fails.append(f"the reader sat out most of its timeout ({elapsed:.1f}s) instead of being woken")

c = call("perception.snapshot", {})["counts"]
print(f"counts        : {c['launched']} launched, {c['saved']} saved, "
      f"{c['dropped_out_of_scope']} refused by scope, {c['held']} held")

print()
if fails:
    print("FAILED:")
    for f in fails:
        print("  -", f)
    sys.exit(1)
print("PASS: it sees what happens and who did it, says nothing outside its scope,")
print("      cannot read outside it, and holds no capabilities while doing so")
PY
STATUS=$?

sudo pkill -f "$BIN" 2>/dev/null
exit $STATUS
