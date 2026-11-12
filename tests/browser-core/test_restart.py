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
    "T2": {"targetId": "T2", "type": "page", "url": "https://home.example/", "title": "Home", "attached": True},
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
            self.committed(session[2:], session)

    def committed(self, tid, session):
        if True:
            for event, p in (("Page.frameStartedLoading", {"frameId": "F-" + tid}),
                             ("Page.frameNavigated", {"frame": {"id": "F-" + tid, "url": self.tabs[tid]["url"]}}),
                             ("Fetch.requestPaused", {"requestId": "R1", "request": {"url": FILE_URL + ".png"}}),
                             ("Page.loadEventFired", {})):
                self.reply({"method": event, "params": p, "sessionId": session})

    def methods(self, name, session=None):
        return [(m, s, p) for m, s, p in self.log if m == name and (session is None or s == session)]

    def index(self, name, session):
        return next(i for i, (m, s, _) in enumerate(self.log) if m == name and s == session)


class NeverCommits(RestartedChromium):
    """`Page.reload` is answered and no navigation follows (a stalled beforeunload, a busy renderer)."""

    def committed(self, tid, session):
        pass


class CommitsLate(RestartedChromium):
    """The reload commits only when `go` is set."""

    def __init__(self, sock):
        super().__init__(sock)
        self.go = threading.Event()

    def committed(self, tid, session):
        threading.Thread(target=lambda: (self.go.wait(10), super(CommitsLate, self).committed(tid, session)),
                         daemon=True).start()


def wait_for(cond, seconds=5):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if cond():
            return True
        time.sleep(0.02)
    return cond()


class TheServiceRestarts(unittest.TestCase):
    chromium_class = RestartedChromium

    def setUp(self):
        patch = mock.patch.object(listener, "place_of_pid", lambda proc, pid: listener.PERSON)
        patch.start()
        self.addCleanup(patch.stop)
        mine, theirs = socket.socketpair()
        self.chromium = self.chromium_class(theirs)
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
        self.assertEqual(sorted(s for _, s, _ in reloads), ["S-T1", "S-T2"],
                         "the tab holding a file: frame and the tab another session was on are reloaded")
        self.assertLess(c.index("Fetch.enable", "S-T2"), c.index("Page.reload", "S-T2"))
        self.assertLess(c.index("Fetch.enable", "S-T1"), c.index("Page.reload", "S-T1"), "reload is behind the fence")

    def test_what_the_reloaded_page_asks_is_decided_and_never_left_paused(self):
        self.hand_over()
        c = self.chromium
        decided = lambda: c.methods("Fetch.continueRequest") + c.methods("Fetch.failRequest")  # noqa: E731
        self.assertTrue(wait_for(decided), "a request paused behind the fence was left hanging")
        self.assertTrue(wait_for(lambda: "S-T1" in {x[1] for x in decided()}), "S-T1's request was not decided")

    def test_nothing_is_reloaded_when_no_tab_holds_a_file_frame_or_had_another_session(self):
        del self.chromium.tabs["T1"]
        del self.chromium.tabs["T2"]["attached"]
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


class TheDeadServicesSessionsAndTheRaces(TheServiceRestarts):
    """The stale session (L1, L2) and the two races (M1, M2) of the review of 1f32cfb."""

    def decided(self):
        c = self.chromium
        return c.methods("Fetch.continueRequest") + c.methods("Fetch.failRequest")

    def test_a_pause_on_the_dead_sessions_id_is_decided_there_and_then_it_is_detached(self):
        self.hand_over()
        c = self.chromium
        self.assertTrue(wait_for(lambda: c.methods("Page.reload")))
        browser = self.driver.sides.browsers[ports.PERSON]
        self.assertTrue(wait_for(lambda: browser.adopted and all(t.ready for t in browser.pages())))
        c.reply({"method": "Fetch.requestPaused", "sessionId": "S-OLD",
                 "params": {"requestId": "R-OLD", "request": {"url": FILE_URL + ".png"}}})
        self.assertTrue(wait_for(lambda: [x for x in self.decided() if x[1] == "S-OLD"]),
                        "a request paused on the stale session was not decided on it")
        self.assertTrue(wait_for(lambda: c.methods("Target.detachFromTarget")), "the stale session was never detached")
        self.assertEqual(c.methods("Target.detachFromTarget")[0][2], {"sessionId": "S-OLD"})
        old = next(i for i, (m, s, _) in enumerate(c.log) if m.startswith("Fetch.") and s == "S-OLD" and m != "Fetch.enable")
        gone = next(i for i, (m, _, _) in enumerate(c.log) if m == "Target.detachFromTarget")
        self.assertLess(old, gone, "detached before what was paused on it was decided")
        detached = [p["sessionId"] for _, _, p in c.methods("Target.detachFromTarget")]
        self.assertNotIn("S-T1", detached)
        self.assertNotIn("S-T2", detached)

    def sees_old_session(self):
        c = self.chromium
        self.assertTrue(wait_for(lambda: c.methods("Page.reload")))
        c.reply({"method": "Network.loadingFailed", "sessionId": "S-OLD", "params": {"requestId": "R-OLD"}})
        return self.driver.sides.browsers[ports.PERSON]

    def test_a_reload_that_never_commits_leaves_the_stale_session_in_place(self):
        """M-A: the gap's paused request is held on S-OLD; detaching it would let that request go."""
        self.chromium_class = NeverCommits
        with mock.patch("yantrik_browser.adopt.ADOPT_WAIT", 0.3):
            self.setUp()
            self.hand_over()
            browser = self.sees_old_session()
            self.assertTrue(wait_for(lambda: browser.adopted))
            self.assertTrue(wait_for(lambda: "S-OLD" in browser.stale))
            time.sleep(0.5)
            self.assertEqual(self.chromium.methods("Target.detachFromTarget"), [],
                             "detached while the old document may still hold a paused request")
            self.assertIn("S-OLD", browser.stale)

    def test_a_reload_that_commits_late_detaches_then(self):
        self.chromium_class = CommitsLate
        with mock.patch("yantrik_browser.adopt.ADOPT_WAIT", 0.3):
            self.setUp()
            self.hand_over()
            browser = self.sees_old_session()
            self.assertTrue(wait_for(lambda: browser.adopted))
            time.sleep(0.3)
            self.assertEqual(self.chromium.methods("Target.detachFromTarget"), [])
            self.chromium.go.set()
            self.assertTrue(wait_for(lambda: self.chromium.methods("Target.detachFromTarget")),
                            "never detached after the reload committed")

    def test_a_tab_opened_after_adoption_does_not_hold_the_cleanup_back(self):
        self.hand_over()
        browser = self.sees_old_session()
        self.assertTrue(wait_for(lambda: self.chromium.methods("Target.detachFromTarget")))
        from yantrik_browser import adopt
        browser.stale.add("S-OLD2")
        with browser.cond:
            browser._saw_target({"targetId": "T9", "type": "page", "url": "about:blank"})
        self.assertFalse(browser.tabs["T9"].gap)
        adopt.sweep(browser)
        self.assertEqual(len(self.chromium.methods("Target.detachFromTarget")), 2)

    def test_a_tab_another_session_was_on_is_stopped_after_its_fence_is_up(self):
        self.hand_over()
        c = self.chromium
        self.assertTrue(wait_for(lambda: c.methods("Page.stopLoading", "S-T2")), "the gap's navigation was left")
        self.assertLess(c.index("Fetch.enable", "S-T2"), c.index("Page.stopLoading", "S-T2"))
        self.assertEqual(c.methods("Page.stopLoading", "S-T1"), [], "a tab no session was on is not stopped")

    def test_status_that_connects_first_does_not_send_the_file_tab_to_access_denied(self):
        self.holder.on_give = None            # the arrival lost the race: the first call is a status
        self.driver._asset_gate = lambda: (lambda url, agent: url)     # a gate that allows what it is asked
        self.hand_over()
        self.driver.status()
        c = self.chromium
        self.assertTrue(wait_for(self.decided))
        self.assertEqual(c.methods("Fetch.failRequest"), [], "the page's own request was refused: no gate")
        self.assertTrue(c.methods("Fetch.continueRequest"))

    def test_the_arrival_is_not_dropped_while_a_long_act_holds_the_lock(self):
        held, release = threading.Event(), threading.Event()

        def hold():
            with self.driver.lock:
                held.set()
                release.wait(10)
        t = threading.Thread(target=hold, daemon=True)
        t.start()
        held.wait(2)
        # The lock is held for longer than the old arrival waited for it (BUSY_WAIT), so only a
        # retrying arrival gets the lock at all.
        with mock.patch("yantrik_browser.adopt.ARRIVAL_RETRY", 0.05, create=True), \
                mock.patch.object(drv, "BUSY_WAIT", 0.1):
            self.hand_over()
            time.sleep(0.5)
            self.assertEqual(self.chromium.methods("Page.reload"), [], "ran under another's lock")
            release.set()
            self.assertTrue(wait_for(lambda: self.chromium.methods("Page.reload")),
                            "the arrival was dropped while the lock was held")
        t.join(2)


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
