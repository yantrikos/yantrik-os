"""A bare DevTools client for the tests that need a party on the browser before the surface: no
fence, no auto-attach, nothing of the surface. It stands in for a page's script run as a click, and
for whatever is open when the surface connects: after a restart of it, or in the gap between a
browser's start and its hand-over (#1170, #1200)."""

import json
import os

from yantrik_browser import transport


class Raw:
    """A bare client over a browser's DevTools pipe. A pipe is one stream: it speaks while the
    service is not connected (the gap before the hand-over, or a restart) and is closed before the
    service connects, which leaves the stream open for it."""

    def __init__(self, holder):
        self.t = transport.PipeTransport(os.dup(holder.held.sock.fileno()))
        self.ids = 0

    def call(self, method, params=None, session=None):
        self.ids += 1
        msg = {"id": self.ids, "method": method, "params": params or {}}
        if session:
            msg["sessionId"] = session
        self.t.send(json.dumps(msg))
        while True:
            got = json.loads(self.t.recv())
            if got.get("id") == self.ids:
                return got.get("result", {})

    def open(self, url="about:blank"):
        """A new tab at `url`, attached: its target id and session."""
        target = self.call("Target.createTarget", {"url": url})["targetId"]
        return target, self.call("Target.attachToTarget", {"targetId": target, "flatten": True})["sessionId"]

    def evaluate(self, session, expression, gesture=False):
        got = self.call("Runtime.evaluate", {"expression": expression, "userGesture": gesture,
                                             "returnByValue": True}, session)
        return (got.get("result") or {}).get("value")

    def close(self):
        self.t.close()
