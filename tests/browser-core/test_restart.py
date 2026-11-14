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

    def __init__(self, sock):
        super().__init__(sock)
        self.tabs["T1"]["attached"] = True      # the dead service was on the file: tab too

    def committed(self, tid, session):
        pass


class CommitsLate(RestartedChromium):
    """The reload commits only when `go` is set."""

    def __init__(self, sock):
        super().__init__(sock)
        self.go = threading.Event()
        self.tabs["T1"]["attached"] = True      # the dead service was on the file: tab too

    def committed(self, tid, session):
        threading.Thread(target=lambda: (self.go.wait(10), super(CommitsLate, self).committed(tid, session)),
                         daemon=True).start()


class StopFailsOnce(RestartedChromium):
    """The first `Page.stopLoading` on S-T2 is refused (no T1: only the https tab is adopted)."""

    def __init__(self, sock):
        super().__init__(sock)
        del self.tabs["T1"]
        self.refused = 0

    def answer(self, msg):
        if msg["method"] == "Page.stopLoading" and msg.get("sessionId") == "S-T2" and not self.refused:
            self.refused = 1
            self.log.append((msg["method"], "S-T2", {}))
            self.reply({"id": msg["id"], "error": {"message": "refused"}})
            return
        super().answer(msg)


def wait_for(cond, seconds=5):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if cond():
            return True
        time.sleep(0.02)
    return cond()


class Restarted:
    """The fixture: a service whose person's Chromium (`chromium_class`) kept its tabs."""
    chromium_class = RestartedChromium
    side, place = ports.PERSON, listener.PERSON     # whose browser it is, and the profile it owns

    def setUp(self):
        patch = mock.patch.object(listener, "place_of_pid", lambda proc, pid: self.place)
        patch.start()
        self.addCleanup(patch.stop)
        mine, theirs = socket.socketpair()
        self.chromium = self.chromium_class(theirs)
        self.chromium.start()
        self.holder = pipes.Pipes()
        g = guard.Guard(identify=lambda port: (_ for _ in ()).throw(listener.Unidentified("nothing", nobody=True)),
                        uid=base.ME, side_of=lambda: self.side, pipes=self.holder)
        self.driver = drv.Driver(judge=False, guard_=g, file_rule=lambda path: True,
                                 home=lambda: "/home/someone")
        self.mine = mine

    def the_browser(self):
        self.assertTrue(wait_for(lambda: self.side in self.driver.sides.browsers))
        return self.driver.sides.browsers[self.side]

    def hand_over(self):
        self.holder.give(self.side, self.mine, os.getpid())


class TheServiceRestarts(Restarted, unittest.TestCase):

    def test_every_tab_is_fenced_and_the_file_tab_reloaded_with_no_call_at_all(self):
        self.hand_over()                      # the shell offers the stream to the new service: no call follows
        c = self.chromium
        self.assertTrue(wait_for(lambda: c.methods("Page.reload")), "the file: tab was not reloaded unprompted")
        for tid in ("T1", "T2"):
            self.assertTrue(wait_for(lambda: c.methods("Fetch.enable", "S-" + tid)), "tab %s was not fenced" % tid)
            self.assertEqual(c.methods("Fetch.enable", "S-" + tid)[0][2], {"patterns": [{"urlPattern": "file://*"}]})
        reloads = c.methods("Page.reload")
        self.assertEqual([s for _, s, _ in reloads], ["S-T1"], "only the tab holding a file: frame is reloaded")
        self.assertTrue(c.methods("Page.stopLoading", "S-T2"), "the pre-attached https tab is stopped, not reloaded")
        self.assertLess(c.index("Fetch.enable", "S-T1"), c.index("Page.reload", "S-T1"), "reload is behind the fence")

    def test_what_the_reloaded_page_asks_is_decided_and_never_left_paused(self):
        self.hand_over()
        c = self.chromium
        decided = lambda: c.methods("Fetch.continueRequest") + c.methods("Fetch.failRequest")  # noqa: E731
        self.assertTrue(wait_for(decided), "a request paused behind the fence was left hanging")
        self.assertTrue(wait_for(lambda: "S-T1" in {x[1] for x in decided()}), "S-T1's request was not decided")

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


class MindViewChromium(RestartedChromium):
    """Mind View's browser across a restart of the service: its stream outlived the dead service
    (it is the shell's), and T1 is a `file:` page the dead service's session had a request paused
    on, which is paused for good. T2 is the blank page the browser was started on."""

    def __init__(self, sock):
        super().__init__(sock)
        self.tabs = {"T1": {"targetId": "T1", "type": "page", "url": FILE_URL, "title": "p1", "attached": True},
                     "T2": {"targetId": "T2", "type": "page", "url": "about:blank", "title": "about:blank"}}

    made = 0

    def answer(self, msg):
        if msg["method"] == "Target.createTarget":
            self.log.append((msg["method"], None, msg.get("params", {})))
            self.made += 1
            tid = "N%d" % self.made          # never an id a page that is closed here already had
            self.tabs[tid] = {"targetId": tid, "type": "page", "url": msg["params"]["url"], "title": "New"}
            self.reply({"id": msg["id"], "result": {"targetId": tid}})
            self.reply({"method": "Target.targetCreated", "params": {"targetInfo": self.tabs[tid]}})
            return
        if msg["method"] == "Target.closeTarget":
            self.log.append((msg["method"], None, msg.get("params", {})))
            self.tabs.pop(msg["params"]["targetId"], None)
            self.reply({"id": msg["id"], "result": {"success": True}})
            self.reply({"method": "Target.targetDestroyed", "params": {"targetId": msg["params"]["targetId"]}})
            return
        super().answer(msg)


class MindViewRestarts(Restarted, unittest.TestCase):
    """#1200 review, LOW-7: Mind View on a pipe is held from its start, so none of the person's
    adoption (a reload, stale sessions learned and swept) applies. What a page left paused is not
    adopted: the page is closed, with no call, and so is every other page that was open before the
    connection, `about:blank` or not: a fresh blank page is made first, and it is the one left."""
    chromium_class = MindViewChromium
    side, place = ports.MIND, listener.MIND_VIEW

    def test_the_page_the_dead_service_left_paused_is_closed_and_one_fresh_blank_page_is_left(self):
        self.hand_over()
        c = self.chromium
        self.assertTrue(wait_for(lambda: c.methods("Target.closeTarget")), "the page was not closed unprompted")
        self.assertEqual([p for _, _, p in c.methods("Target.closeTarget")], [{"targetId": "T1"}, {"targetId": "T2"}])
        self.assertEqual([p for _, _, p in c.methods("Target.createTarget")], [{"url": "about:blank"}])
        self.assertLess(next(i for i, (m, _, _) in enumerate(c.log) if m == "Target.setAutoAttach"),
                        next(i for i, (m, _, _) in enumerate(c.log) if m == "Target.createTarget"),
                        "the fence is up before the fresh page is opened")
        self.assertLess(next(i for i, (m, _, _) in enumerate(c.log) if m == "Target.createTarget"),
                        next(i for i, (m, _, _) in enumerate(c.log) if m == "Target.closeTarget"),
                        "the fresh page is opened before any old one is closed")
        self.assertTrue(c.methods("Target.setAutoAttach"), "held from its start from here on")
        self.assertEqual(c.methods("Page.reload"), [], "closed, not reloaded for a later touch to find")
        browser = self.the_browser()
        self.assertTrue(wait_for(lambda: [t.id for t in browser.pages()] == ["N1"]), "only the fresh blank page is left")
        self.assertEqual(browser.stale, set(), "no stale session is learned: nothing is left that could hold one")

    def test_a_second_restart_with_nothing_but_a_blank_page_still_replaces_it(self):
        """A blank page is not known to be untouched (a `file:` page may have written into it), so
        it is closed as well and one fresh page is made."""
        del self.chromium.tabs["T1"]
        self.hand_over()
        c = self.chromium
        self.assertTrue(wait_for(lambda: c.methods("Target.setAutoAttach")))
        browser = self.the_browser()
        self.assertTrue(wait_for(lambda: [t.id for t in browser.pages()] == ["N1"]))
        self.assertEqual([p for _, _, p in c.methods("Target.closeTarget")], [{"targetId": "T2"}])
        self.assertEqual(len(c.methods("Target.createTarget")), 1)


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

    def sees_old_session(self, reloaded=True):
        c = self.chromium
        self.assertTrue(wait_for(lambda: c.methods("Page.reload" if reloaded else "Page.stopLoading")))
        c.reply({"method": "Network.loadingFailed", "sessionId": "S-OLD", "params": {"requestId": "R-OLD"}})
        return self.driver.sides.browsers[ports.PERSON]

    def test_a_pre_attached_https_tab_is_fenced_and_stopped_not_reloaded_and_its_stale_session_goes(self):
        """MEDIUM-1: a reload re-POSTs silently; with no file: frame it buys no safety."""
        del self.chromium.tabs["T1"]
        self.hand_over()
        browser = self.sees_old_session(reloaded=False)
        c = self.chromium
        self.assertTrue(wait_for(lambda: c.methods("Target.detachFromTarget")), "the stale session was never detached")
        self.assertTrue(c.methods("Fetch.enable", "S-T2"))
        self.assertTrue(c.methods("Page.stopLoading", "S-T2"))
        self.assertEqual(c.methods("Page.reload"), [], "an https tab was reloaded")
        self.assertFalse(browser.tabs["T2"].gap)

    def test_a_gap_tab_whose_adoption_failed_is_retried_from_the_cleanup(self):
        """LOW-1: a gap tab left unready is adopted again by the next sweep, and held back till then."""
        from yantrik_browser import adopt
        real = cdp.Browser.attach
        fails = [True]

        def flaky(browser, tab, wait=cdp.CALL_TIMEOUT):
            if fails[0] and tab.id == "T2":
                raise cdp.CdpError("adoption failed")
            return real(browser, tab, wait)
        with mock.patch.object(cdp.Browser, "attach", flaky):
            self.hand_over()
            browser = self.sees_old_session()
            self.assertTrue(wait_for(lambda: browser.adopted and "S-OLD" in browser.stale))
            time.sleep(0.3)
            self.assertTrue(browser.tabs["T2"].gap)
            self.assertEqual(self.chromium.methods("Target.detachFromTarget"), [], "detached with a gap tab unadopted")
            fails[0] = False
            adopt.after_commit(browser)
            self.assertTrue(wait_for(lambda: self.chromium.methods("Target.detachFromTarget")),
                            "the gap tab was never adopted again")

    def test_a_gap_on_a_devtools_page_does_not_hold_the_cleanup_back(self):
        self.hand_over()
        browser = self.sees_old_session()
        self.assertTrue(wait_for(lambda: self.chromium.methods("Target.detachFromTarget")))
        from yantrik_browser import adopt
        with browser.cond:
            browser._saw_target({"targetId": "D1", "type": "page", "url": "devtools://x", "attached": True})
        browser.stale.add("S-OLD3")
        adopt.sweep(browser)
        self.assertEqual(len(self.chromium.methods("Target.detachFromTarget")), 2)

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
        self.holder.holder(ports.PERSON).on_give = None            # the arrival lost the race: the first call is a status
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


class TheReloadNeverCommits(Restarted, unittest.TestCase):
    chromium_class = NeverCommits

    def sees_old_session(self):
        return TheDeadServicesSessionsAndTheRaces.sees_old_session(self)

    def test_a_reload_that_never_commits_leaves_the_stale_session_in_place(self):
        """M-A: the gap's paused request is held on S-OLD; detaching it would let that request go."""
        with mock.patch("yantrik_browser.adopt.ADOPT_WAIT", 0.3):
            self.hand_over()
            browser = self.sees_old_session()
            self.assertTrue(wait_for(lambda: browser.adopted))
            self.assertTrue(wait_for(lambda: "S-OLD" in browser.stale))
            time.sleep(0.5)
            self.assertEqual(self.chromium.methods("Target.detachFromTarget"), [],
                             "detached while the old document may still hold a paused request")
            self.assertIn("S-OLD", browser.stale)


class TheReloadCommitsLate(Restarted, unittest.TestCase):
    chromium_class = CommitsLate

    def sees_old_session(self):
        return TheDeadServicesSessionsAndTheRaces.sees_old_session(self)

    def test_a_reload_that_commits_late_detaches_then(self):
        with mock.patch("yantrik_browser.adopt.ADOPT_WAIT", 0.3):
            self.hand_over()
            browser = self.sees_old_session()
            self.assertTrue(wait_for(lambda: browser.adopted))
            time.sleep(0.3)
            self.assertEqual(self.chromium.methods("Target.detachFromTarget"), [])
            self.chromium.go.set()
            self.assertTrue(wait_for(lambda: self.chromium.methods("Target.detachFromTarget")),
                            "never detached after the reload committed")


class MindViewsBrowserIsStillHeld(unittest.TestCase):
    def test_a_held_connection_does_not_attach_to_tabs_it_has_not_been_given(self):
        mine, theirs = socket.socketpair()
        chromium = MindViewChromium(theirs)           # one page that drew before the hold, one blank
        chromium.start()
        browser = cdp.Browser(opener=lambda: base.transport.PipeTransport(os.dup(mine.fileno())), held_at_start=True)
        browser.connect()
        time.sleep(0.3)
        self.assertEqual(chromium.methods("Target.attachToTarget"), [], "auto-attach hands Mind View its pages")
        self.assertEqual(len(chromium.methods("Target.setAutoAttach")), 1)
        browser.close()


class TheStopFailsFirst(Restarted, unittest.TestCase):
    chromium_class = StopFailsOnce

    def test_a_gap_is_not_closed_and_nothing_detached_until_a_stop_succeeds(self):
        """MEDIUM-1: a failed Page.stopLoading leaves the gap; the retry sends a second one, and only
        after it succeeds does the stale session go."""
        self.hand_over()
        c = self.chromium
        self.assertTrue(wait_for(lambda: c.methods("Page.stopLoading", "S-T2")))
        browser = self.driver.sides.browsers[ports.PERSON]
        self.assertTrue(wait_for(lambda: browser.adopted))
        c.reply({"method": "Network.loadingFailed", "sessionId": "S-OLD", "params": {"requestId": "R-OLD"}})
        # the retry (driven by the stale session) re-enters attach: a second stop, then the detach
        self.assertTrue(wait_for(lambda: len(c.methods("Page.stopLoading", "S-T2")) >= 2), "no second stop was sent")
        self.assertTrue(wait_for(lambda: c.methods("Target.detachFromTarget")), "never detached after the stop succeeded")
        self.assertEqual(c.refused, 1)
        second = [i for i, (m, s, _) in enumerate(c.log) if m == "Page.stopLoading" and s == "S-T2"][1]
        gone = next(i for i, (m, _, _) in enumerate(c.log) if m == "Target.detachFromTarget")
        self.assertLess(second, gone, "detached before a stop had succeeded")
        self.assertFalse(browser.tabs["T2"].gap)

    def test_with_the_stop_never_succeeding_the_gap_stays_and_nothing_is_detached(self):
        c = self.chromium
        real = c.answer

        def always_refuse(msg):
            if msg["method"] == "Page.stopLoading":
                c.log.append((msg["method"], msg.get("sessionId"), {}))
                c.reply({"id": msg["id"], "error": {"message": "refused"}})
                return
            real(msg)
        c.answer = always_refuse
        self.hand_over()
        self.assertTrue(wait_for(lambda: c.methods("Page.stopLoading", "S-T2")))
        browser = self.driver.sides.browsers[ports.PERSON]
        self.assertTrue(wait_for(lambda: browser.adopted))
        c.reply({"method": "Network.loadingFailed", "sessionId": "S-OLD", "params": {"requestId": "R-OLD"}})
        self.assertTrue(wait_for(lambda: len(c.methods("Page.stopLoading", "S-T2")) >= 2))
        time.sleep(0.3)
        self.assertTrue(browser.tabs["T2"].gap)
        self.assertEqual(c.methods("Target.detachFromTarget"), [])


class TheFileCommitWeDidNotStart(Restarted, unittest.TestCase):

    def test_a_file_commit_on_a_pre_attached_https_tab_opens_the_gap_and_reloads_once(self):
        """LOW-1: a navigation the dead service let through, committing after the gap closed."""
        del self.chromium.tabs["T1"]
        self.hand_over()
        c = self.chromium
        browser = self.the_browser()
        self.assertTrue(wait_for(lambda: "T2" in browser.tabs and browser.tabs["T2"].ready and not browser.tabs["T2"].gap))
        self.assertEqual(c.methods("Page.reload"), [])
        c.tabs["T2"]["url"] = FILE_URL
        c.reply({"method": "Page.frameNavigated", "sessionId": "S-T2",
                 "params": {"frame": {"id": "F-T2", "url": FILE_URL}}})
        self.assertTrue(wait_for(lambda: len(c.methods("Page.reload", "S-T2")) == 1), "the late file: commit was not reloaded")
        self.assertTrue(wait_for(lambda: not browser.tabs["T2"].gap), "the reload's commit did not close the gap")
        time.sleep(0.3)
        self.assertEqual(len(c.methods("Page.reload", "S-T2")), 1, "reloaded more than once")

    def test_a_late_file_commit_holds_the_cleanup_back_until_the_reload_commits(self):
        del self.chromium.tabs["T1"]
        from yantrik_browser import adopt
        self.hand_over()
        c = self.chromium
        browser = self.the_browser()
        self.assertTrue(wait_for(lambda: "T2" in browser.tabs and browser.tabs["T2"].ready and not browser.tabs["T2"].gap))
        with browser.cond:
            tab = browser.tabs["T2"]
            tab.url = FILE_URL
            tab.navigations += 1
            self.assertTrue(adopt.regap(browser, tab))
            self.assertTrue(tab.gap)
        self.assertTrue(wait_for(lambda: not tab.gap))


if __name__ == "__main__":
    unittest.main()
