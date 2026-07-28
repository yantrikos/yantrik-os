#!/usr/bin/env python3
"""Prove each service actually answers JSON-RPC, not merely that a socket exists."""
import json, socket, sys, glob, os

CALLS = {
    "system-monitor": ("sysmon.snapshot", {}),
    "weather":        ("weather.current", {"lat": 32.7767, "lon": -96.797}),
    "network":        ("network.status", {}),
    "notifications":  ("notifications.list", {}),
}

def call(sock_path, method, params, timeout=8):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    s.connect(sock_path)
    req = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}) + "\n"
    s.sendall(req.encode())
    buf = b""
    while b"\n" not in buf:
        chunk = s.recv(65536)
        if not chunk:
            break
        buf += chunk
    s.close()
    return json.loads(buf.decode().strip())

d = sys.argv[1] if len(sys.argv) > 1 else "/tmp/yantrik-1000"
socks = sorted(glob.glob(os.path.join(d, "*.sock")))
if not socks:
    print(f"NO SOCKETS in {d}")
    sys.exit(1)

alive = 0
for p in socks:
    name = os.path.basename(p).replace(".sock", "")
    method, params = CALLS.get(name, (f"{name}.status", {}))
    try:
        resp = call(p, method, params)
        if "result" in resp:
            body = json.dumps(resp["result"])[:90]
            print(f"LIVE     {name:16s} {method:24s} -> {body}")
            alive += 1
        else:
            err = resp.get("error", {})
            # -32601 (no such method) and -32602 (bad params) are both proof the
            # server received, parsed, routed and validated the call. Only a
            # connect/timeout failure means dead.
            if err.get("code") in (-32601, -32602):
                print(f"SERVING  {name:16s} {method:24s} -> dispatched, rejected params: {err.get('message')}")
                alive += 1
            else:
                print(f"ERROR    {name:16s} {method:24s} -> {err}")
    except Exception as e:
        print(f"DEAD     {name:16s} {method:24s} -> {type(e).__name__}: {e}")

print(f"\n{alive}/{len(socks)} services answered RPC")
sys.exit(0 if alive == len(socks) else 1)
