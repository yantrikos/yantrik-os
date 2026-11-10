#!/usr/bin/env python3
"""A stand-in for the shell's control surface, for the placement check.

Answers `app.act` on `$XDG_RUNTIME_DIR/yantrik/app-shell.sock` the way the shell does for the Mind
desk: `mind_desk` with a snapshot (a Mind at work, with a tool, counts and a model), then holding
the next ask for its wait, as the real one does. Everything else is refused. It checks nobody's pid:
the placement check is about where the desk's windows land, not about who may ask.

    fake_shell.py [state]        state: running (default), waiting, idle
"""

import json
import os
import socketserver
import sys
import threading
import time

STATE = sys.argv[1] if len(sys.argv) > 1 else "running"

SNAPSHOT = {
    "tag": 1,
    "as_of": "14:32",
    "mind": {
        "agent": "hermes:c-1",
        "name": "Hermes",
        "state": {"running": "running_tool", "waiting": "waiting_for_you", "idle": "idle"}[STATE],
        "since": 100,
        "stop": None,
        "tool": {"name": "blender.add_object", "evidence": "reported", "outcome": "running"},
        "counts": {"calls": 12, "failed": 1, "interrupted": 1},
        "elapsed_secs": 272,
        "model": "deepseek-v4.1-flash via ollama-cloud",
        "account": "person",
    },
    "apps": ["Blender", "Terminal", "Files"],
}


class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        for line in self.rfile:
            try:
                request = json.loads(line)
            except ValueError:
                continue
            params = request.get("params") or {}
            args = params.get("args") or {}
            if request.get("method") == "app.act" and params.get("action") == "mind_desk":
                # The first ask is answered at once; one that has the snapshot already waits.
                if args.get("since") == SNAPSHOT["tag"]:
                    time.sleep(min(int(args.get("wait_seconds", 0)), 25))
                reply = {"jsonrpc": "2.0", "id": request["id"], "result": {"result": SNAPSHOT}}
            else:
                reply = {"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32000, "message": "refused"}}
            self.wfile.write((json.dumps(reply) + "\n").encode())
            self.wfile.flush()


class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True


def main():
    directory = os.path.join(os.environ["XDG_RUNTIME_DIR"], "yantrik")
    os.makedirs(directory, exist_ok=True)
    path = os.path.join(directory, "app-shell.sock")
    if os.path.exists(path):
        os.unlink(path)
    server = Server(path, Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    print("fake shell listening", flush=True)
    while True:
        time.sleep(3600)


if __name__ == "__main__":
    main()
