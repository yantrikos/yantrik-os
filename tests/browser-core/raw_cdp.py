"""A bare DevTools client for the tests that need a second party on the browser: no fence, no
auto-attach, nothing of the surface. It stands in for the person's own browser window, for a page's
script run as a click, and for whatever else is open when the surface connects (#1170)."""

import json
import urllib.request


class Raw:
    def __init__(self, address):
        from websocket import create_connection
        with urllib.request.urlopen(address + "/json/version", timeout=3) as r:
            url = json.load(r)["webSocketDebuggerUrl"]
        self.ws = create_connection(url, origin=address, suppress_origin=False, timeout=20)
        self.ids = 0

    def call(self, method, params=None, session=None):
        self.ids += 1
        msg = {"id": self.ids, "method": method, "params": params or {}}
        if session:
            msg["sessionId"] = session
        self.ws.send(json.dumps(msg))
        while True:
            got = json.loads(self.ws.recv())
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
        self.ws.close()
