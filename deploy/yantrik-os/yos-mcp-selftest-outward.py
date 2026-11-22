#!/usr/bin/env python3
"""Outward acts ask by effect (#164): the bridge's side of the rule.

    python3 deploy/yantrik-os/yos-mcp-selftest-outward.py

The shell's gate is the guard (`always_asked.rs`, with its own tests). This file checks what the
bridge hands the taint: only the named silent set is not outbound, and an action nobody named is
outbound (it fails closed). Command output taints too, as a second message and never the guard.
"""
import importlib.util
import os
import sys
from importlib.machinery import SourceFileLoader

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
os.environ.setdefault("YOS_MCP_NO_TAINT", "0")
loader = SourceFileLoader("yosmcp_outward", os.path.join(HERE, "yos-mcp"))
spec = importlib.util.spec_from_loader("yosmcp_outward", loader)
mcp = importlib.util.module_from_spec(spec)
loader.exec_module(mcp)

FAILED = []


def check(name, ok):
    print(("ok   " if ok else "FAIL ") + name)
    if not ok:
        FAILED.append(name)


def outbound(action, args=None):
    tool = {"name": "os_act"}
    call = {"app": "browser", "action": action, "args": args or {}}
    return mcp.outbound_of(tool, call)[0]


for action in sorted(mcp.BROWSER_SILENT):
    check("silent browser action %s is not outbound" % action, outbound(action) == "os_act")
for action in ("click", "type", "select", "dialog", "drag", "context_click", "commit", "forward",
               "reload", "switch_tab", "close_tab", "an_action_added_later"):
    check("browser %s is outbound: it asks" % action, outbound(action) == "web_type")
check("a key with no text is silent", outbound("press", {"key": "Enter"}) == "os_act")
check("a key with a letter is outbound", outbound("press", {"key": "s"}) == "web_type")
check("go is read as an address", outbound("go", {"url": "https://a.example/"}) == "web_go")
sys.exit(1 if FAILED else 0)
