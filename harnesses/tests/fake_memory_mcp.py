#!/usr/bin/env python3
"""A stand-in for yantrikdb-mcp: the four memory tools (and one it must not offer a mind), kept in
the JSON file $FAKE_MEMORY_FILE so a server started per call still remembers. Every remember's
arguments are kept whole, so a test can see the provenance the harness stamped on them."""

import json
import os
import sys
import uuid

STORE = os.environ.get("FAKE_MEMORY_FILE", "")


def load():
    try:
        with open(STORE, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return []


def save(rows):
    with open(STORE, "w", encoding="utf-8") as f:
        json.dump(rows, f)


TOOLS = [{"name": n, "description": "YantrikDB %s" % n,
          "inputSchema": {"type": "object", "properties": {}}}
         for n in ("remember", "recall", "forget", "correct", "think")]


def call(name, args):
    rows = load()
    if name == "remember":
        rid = uuid.uuid4().hex
        rows.append({"rid": rid, "text": args.get("text", ""), "args": args})
        save(rows)
        return json.dumps({"rid": rid, "status": "recorded"})
    if name == "recall":
        words = set(str(args.get("query", "")).lower().split())
        hits = [r for r in rows if words & set(r["text"].lower().split())]
        return json.dumps({"count": len(hits), "results": hits})
    if name == "forget":
        save([r for r in rows if r["rid"] != args.get("rid")])
        return json.dumps({"forgotten": 1})
    return json.dumps({"ok": True})


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    if "id" not in msg:
        continue
    method = msg.get("method")
    if method == "initialize":
        result = {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                  "serverInfo": {"name": "fake-yantrikdb", "version": "0"}}
    elif method == "tools/list":
        result = {"tools": TOOLS}
    elif method == "tools/call":
        p = msg.get("params") or {}
        result = {"content": [{"type": "text", "text": call(p.get("name"), p.get("arguments") or {})}]}
    else:
        result = {}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}) + "\n")
    sys.stdout.flush()
