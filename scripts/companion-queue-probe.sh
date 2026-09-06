#!/bin/bash
# Is a request accepted quickly, and does the caller learn where it stands?
#
# The thing being replaced: every call into the companion blocked until it was finished. With one
# worker lane that means a one-millisecond tool call arriving mid-generation waits for the whole
# answer — measured at thirty-six to fifty seconds — and learns nothing while it waits.
#
# This drives the busiest moment the shell has: the startup brief is still generating. Four checks:
#
#   1. submit returns in milliseconds even then
#   2. it says how many jobs are ahead, and what is running
#   3. await reports progress before there is an answer, and wakes on the change
#   4. a queued job can be taken back
set -u

export DISPLAY=:0 SLINT_BACKEND=winit-software
unset XDG_RUNTIME_DIR
TARGET=${TARGET:-/home/yantrik/target-yantrik/fast}
CONFIG=${CONFIG:-/home/yantrik/yantrik-run/config.yaml}
cd /home/yantrik/yantrik-run || exit 1

pkill -x yantrik-ui 2>/dev/null
sleep 2
rm -f /tmp/yantrik-*/companion.sock

setsid nohup "$TARGET/yantrik-ui" "$CONFIG" > shell_queue.log 2>&1 &
# Deliberately short: the point is to arrive while the startup brief is still generating, which is
# exactly the case the old blocking API handled worst.
sleep 14
echo "shell alive   : $(pgrep -cf "$TARGET/yantrik-ui")"

python3 - <<'PY'
import glob, json, socket, sys, time

paths = glob.glob("/tmp/yantrik-*/companion.sock")
if not paths:
    sys.exit("FAIL: the shell published no companion socket")
path = paths[0]
fails = []

def call(method, params, timeout=200):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    s.connect(path)
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

# ── 1. Accepted quickly, while the model is busy ─────────────────────
t0 = time.perf_counter()
first, err = call("companion.submit", {"kind": "tool", "name": "list_apps", "args": {}})
accepted_ms = (time.perf_counter() - t0) * 1000
if err:
    print(f"FAIL: companion.submit -> {err}")
    sys.exit(1)
print(f"accepted in   : {accepted_ms:.1f} ms")
print(f"ticket        : {first['ticket']}  lane={first['lane']}  ahead={first['ahead']}  active={first['active']}")
print(f"estimate      : {first['eta_seconds']}  ({first['eta_basis']})")
if accepted_ms > 250:
    fails.append(f"acceptance took {accepted_ms:.0f} ms; the point of a ticket is that it does not")

# ── 2. The board says what is going on ───────────────────────────────
board, err = call("companion.jobs", {})
if err:
    fails.append(f"companion.jobs -> {err}")
else:
    for lane in board["lanes"]:
        working = [w["kind"] for w in lane["working_on"]]
        print(f"lane {lane['lane']:<8}: {lane['queued']} queued, {lane['active']} active {working}")
    model = next((l for l in board["lanes"] if l["lane"] == "model"), None)
    if not model:
        fails.append("the model lane was not reported at all")
    elif first["ahead"] == 0 and model["active"] == 0 and model["queued"] > 0:
        # The trap this check exists for: a job sitting behind work the board never recorded. If
        # something is queued and nothing is running, either the lane is genuinely idle — in which
        # case the queued job should have started — or the board is not seeing the lane's own
        # work, which is worse than having no board.
        fails.append("something is queued while the board claims nothing is running")
    elif model["active"] == 0 and model["queued"] == 0:
        print("note          : the lane was already idle, so the busy case was not exercised")

# ── 3. Progress before an answer, and a wake on change ───────────────
second, err = call("companion.submit", {"kind": "ask", "prompt": "Reply with exactly: QUEUE OK"})
if err:
    fails.append(f"submitting an ask -> {err}")
else:
    print(f"second ticket : {second['ticket']}  ahead={second['ahead']}")
    if second["ahead"] < 1 and first["ahead"] == 0:
        # The first submission should still be in front of this one unless it has already run.
        state, _ = call("companion.await", {"ticket": first["ticket"], "wait_ms": 0})
        if state and state["state"] == "queued":
            fails.append("two queued jobs, and the second was told nothing was ahead of it")

    t0 = time.perf_counter()
    status, err = call("companion.await", {"ticket": second["ticket"], "wait_ms": 120000})
    waited = time.perf_counter() - t0
    if err:
        fails.append(f"companion.await -> {err}")
    else:
        print(f"awaited       : {waited:.1f}s -> {status['state']}, ahead={status['ahead']}")
        if status["partial"]:
            print(f"partial       : {status['partial'][:60]!r}")
        if status["state"] == "queued" and waited > 100:
            fails.append("await returned still-queued after two minutes")
        # The wake must come from the change, not from the timeout expiring.
        if waited > 115:
            fails.append(f"await sat out its whole timeout ({waited:.0f}s)")

    # Follow it to the end, however long the model takes.
    for _ in range(6):
        status, err = call("companion.await", {"ticket": second["ticket"], "wait_ms": 60000})
        if err or status["state"] != "running" and status["state"] != "queued":
            break
    if status:
        print(f"final         : {status['state']}  ran_for={status['ran_for_seconds']}")
        if status["state"] == "done":
            print(f"answer        : {(status['result'] or '')[:70]!r}")
        elif status["state"] == "failed":
            print(f"error         : {status['error']}")

# ── 4. A queued job can be taken back ────────────────────────────────
a, _ = call("companion.submit", {"kind": "ask", "prompt": "one"})
b, _ = call("companion.submit", {"kind": "ask", "prompt": "two"})
if b:
    result, err = call("companion.cancel", {"ticket": b["ticket"]})
    print(f"cancelled     : {result}" if not err else f"cancel failed : {err}")
    if err:
        fails.append(f"cancel -> {err}")
    else:
        after, _ = call("companion.await", {"ticket": b["ticket"], "wait_ms": 0})
        if after and after["state"] != "cancelled":
            fails.append(f"a cancelled job reports {after['state']}")
if a:
    call("companion.cancel", {"ticket": a["ticket"]})

# An estimate must never be invented; it is either grounded or absent.
board, _ = call("companion.jobs", {})
if board:
    print(f"typical       : {board['typical']}")

print()
if fails:
    print("FAILED:")
    for f in fails:
        print("  -", f)
    sys.exit(1)
print("PASS: requests are accepted in milliseconds, and say where they stand")
PY
STATUS=$?

pkill -x yantrik-ui 2>/dev/null
exit $STATUS
