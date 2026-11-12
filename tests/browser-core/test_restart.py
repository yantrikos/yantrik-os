#!/usr/bin/env python3
"""The browser service restarts with the person's tabs open (gate 5, bar 4).

    python3 tests/browser-core/test_restart.py

The person's Chromium outlives the service: the pipe stays open in the shell, so what the dead
service's sessions had paused (`file:` requests held by Fetch for a decider that is gone) stays
paused, and the new service must not wait for a call to find the tab. A small CDP speaker plays the
browser over a real socket pair; the service's driver, guard, sides, cdp and fence are the real ones.
"""

import os
import socket
import sys
import threading
import time
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import test_pipe as base  # noqa: E402,F401  (puts apps/browser on the path)
from yantrik_browser import cdp, driver as drv, guard, listener, pipes, ports  # noqa: E402

FILE_URL = "file:///home/someone/p1.html"
TABS = {
    "T1": {"targetId": "T1", "type": "page", "url": FILE_URL, "title": "p1"},
    "T2": {"targetId": "T2", "type": "page", "url": "https://home.example/", "title": "Home"},
}


class RestartedChromium(base.FakeChromium):
    """A browser whose tabs were open before this service: T1 holds a `file:` frame, and when it is
    reloaded its page asks for an image, which the fence's session must decide."""

    def __init__(self, sock):
        super().__init__(sock)
        self.tabs = {k: dict(v) for k, v in TABS.items()}
        self.log = []                  # (method, session, params), in the order they arrived

    def answer(self, msg):
        method, params, session = msg["method"], msg.get("params", {}), msg.get("sessionId")
        self.log.append((method, session, params))
        if method == "Page.getFrameTree":
            tid = session[2:]
            self.reply({"id": msg["id"], "result": {"frameTree": {"frame": {"id": "F-" + tid, "url": self.tabs[tid]["url"]}}}})
            return
        super().answer(msg)
        if method == "Page.reload":
            tid = session[2:]
            for event, p in (("Page.frameStartedLoading", {"frameId": "F-" + tid}),
                             ("Page.frameNavigated", {"frame": {"id": "F-" + tid, "url": self.tabs[tid]["url"]}}),
                             ("Fetch.requestPaused", {"requestId": "R1", "request": {"url": FILE_URL + ".png"}}),
                             ("Page.loadEventFired", {})):
                self.reply({"method": event, "params": p, "sessionId": session})

    def methods(self, name, session=None):
        return [(m, s, p) for m, s, p in self.log if m == name and (session is None or s == session)]

    def index(self, name, session):
        return next(i for i, (m, s, _) in enumerate(self.log) if m == name and s == session)


def wait_for(cond, seconds=5):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if cond():
            return True
        time.sleep(0.02)
    return cond()


class TheServiceRestarts(unittest.TestCase):
    def setUp(self):
        patch = mock.patch.object(listener, "place_of_pid", lambda proc, pid: listener.PERSON)
        patch.start()
        self.addCleanup(patch.stop)
        mine, theirs = socket.socketpair()
        self.chromium = RestartedChromium(theirs)
        self.chromium.start()
        self.holder = pipes.PipeHolder()
        g = guard.Guard(identify=lambda port: (_ for _ in ()).throw(listener.Unidentified("nothing", nobody=True)),
                        uid=base.ME, side_of=lambda: ports.PERSON, pipes=self.holder)
        self.driver = drv.Driver(judge=False, guard_=g, file_rule=lambda path: True,
                                 home=lambda: "/home/someone")
        self.mine = mine

    def hand_over(self):
        self.holder.give(self.mine, os.getpid())

    def test_every_tab_is_fenced_and_the_file_tab_reloaded_with_no_call_at_all(self):
        self.hand_over()                      # the shell offers the stream to the new service: no call follows
        c = self.chromium
        self.assertTrue(wait_for(lambda: c.methods("Page.reload")), "the file: tab was not reloaded unprompted")
        for tid in ("T1", "T2"):
            self.assertTrue(wait_for(lambda: c.methods("Fetch.enable", "S-" + tid)), "tab %s was not fenced" % tid)
            self.assertEqual(c.methods("Fetch.enable", "S-" + tid)[0][2], {"patterns": [{"urlPattern": "file://*"}]})
        reloads = c.methods("Page.reload")
        self.assertEqual([s for _, s, _ in reloads], ["S-T1"], "only the tab holding a file: frame is reloaded")
        self.assertLess(c.index("Fetch.enable", "S-T1"), c.index("Page.reload", "S-T1"), "reload is behind the fence")

    def test_what_the_reloaded_page_asks_is_decided_and_never_left_paused(self):
        self.hand_over()
        c = self.chromium
        decided = lambda: c.methods("Fetch.continueRequest") + c.methods("Fetch.failRequest")  # noqa: E731
        self.assertTrue(wait_for(decided), "a request paused behind the fence was left hanging")
        self.assertEqual(decided()[0][1], "S-T1")

    def test_nothing_is_reloaded_when_no_tab_holds_a_file_frame(self):
        del self.chromium.tabs["T1"]
        self.hand_over()
        self.assertTrue(wait_for(lambda: self.chromium.methods("Fetch.enable", "S-T2")))
        time.sleep(0.3)
        self.assertEqual(self.chromium.methods("Page.reload"), [])

    def test_a_reconnect_adopts_the_tabs_too(self):
        """Not only a hand-over: any connection the person's side makes adopts every open tab."""
        self.hand_over()
        self.assertTrue(wait_for(lambda: self.chromium.methods("Page.reload")))
        browser = self.driver.sides.browsers[ports.PERSON]
        self.assertTrue(wait_for(lambda: all(t.ready for t in browser.pages())))
        browser.close()
        before = len(self.chromium.methods("Fetch.enable"))
        self.driver.tabs()                                    # a call reconnects
        self.assertGreater(len(self.chromium.methods("Fetch.enable")), before)
        self.assertTrue(all(t.ready for t in browser.pages()))

    def test_the_person_is_never_held_and_no_tcp_is_used(self):
        for target in ("urllib.request.urlopen", "socket.create_connection"):
            p = mock.patch(target, side_effect=AssertionError("TCP was used: " + target))
            p.start()
            self.addCleanup(p.stop)
        self.hand_over()
        self.assertTrue(wait_for(lambda: self.chromium.methods("Page.reload")))
        self.assertFalse(self.driver.sides.browsers[ports.PERSON].held_at_start)
        self.assertEqual(self.chromium.methods("Target.setAutoAttach"), [], "no window of theirs is held")
        self.assertEqual(self.chromium.methods("Target.closeTarget"), [])


class MindViewsBrowserIsStillHeld(unittest.TestCase):
    def test_a_held_connection_does_not_attach_to_tabs_it_has_not_been_given(self):
        mine, theirs = socket.socketpair()
        chromium = RestartedChromium(theirs)
        chromium.start()
        browser = cdp.Browser(opener=lambda: base.transport.PipeTransport(os.dup(mine.fileno())), held_at_start=True)
        browser.connect()
        time.sleep(0.3)
        self.assertEqual(chromium.methods("Target.attachToTarget"), [], "auto-attach hands Mind View its pages")
        self.assertEqual(len(chromium.methods("Target.setAutoAttach")), 1)
        browser.close()


if __name__ == "__main__":
    unittest.main()
