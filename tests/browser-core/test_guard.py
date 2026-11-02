#!/usr/bin/env python3
"""Which browser a call gets, and when it is refused (#1085): the port and profile choice per
caller, with no browser running.

    python3 tests/browser-core/test_guard.py

Linux only (the surface SDK reads the kernel's peer credentials). The browsers are fakes: a
function that says what holds each port, and a Browser whose connect() would fail the test if a
refused call ever reached it. test_ports_live.py does the same against real Chromiums.
"""

import os
import shutil
import sys
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]

from yantrik_browser import cdp, driver as drv, guard, listener, ports  # noqa: E402
from yantrik_surface import PeerCred, mind_door  # noqa: E402

ME = os.getuid()


def holding(**by_port):
    """An `identify` that answers from a table: port -> place (or an Unidentified)."""
    table = {int(k): v for k, v in by_port.items()}

    def identify(port):
        what = table.get(port)
        if what is None:
            raise listener.Unidentified("nothing is listening there", nobody=True)
        if isinstance(what, Exception):
            raise what
        return listener.Listener(port, ME, 1000 + port, (port,), what)
    return identify


def everywhere(place):
    """An `identify` under which every port is held by a browser on this profile."""
    return lambda port: listener.Listener(port, ME, 1, (1,), place)


def driver_for(identify, side):
    """A driver whose callers are all `side`, over fake listeners."""
    g = guard.Guard(identify=identify, uid=ME, side_of=lambda: side)
    return drv.Driver(judge=False, guard_=g)


class WhoIsCalling(unittest.TestCase):
    def side(self, **kw):
        kw.setdefault("ask_shell", lambda pid: "person")
        kw.setdefault("uid", ME)
        kw.setdefault("proc", "/nonexistent-proc")
        return guard.side_of_caller(**kw)

    def test_the_person_is_only_a_caller_the_shell_calls_the_person(self):
        me = PeerCred(4242, ME, ME)
        self.assertEqual(self.side(peer=me, token=None), ports.PERSON)
        self.assertEqual(self.side(peer=me, token=None, ask_shell=lambda pid: "mind"), ports.MIND)

    def test_everything_unclear_is_a_mind(self):
        me = PeerCred(4242, ME, ME)
        for answer in (None, "", "nobody", "Person", 1):
            self.assertEqual(self.side(peer=me, token=None, ask_shell=lambda pid, a=answer: a), ports.MIND, answer)
        self.assertEqual(self.side(peer=None, token=None), ports.MIND, "no peer")
        self.assertEqual(self.side(peer=PeerCred(4242, ME + 1, ME), token=None), ports.MIND, "another account")

    def test_an_agent_token_makes_a_mind_whoever_the_account_is(self):
        self.assertEqual(self.side(peer=PeerCred(4242, ME, ME), token="tok"), ports.MIND)

    def test_the_mind_account_is_a_mind(self):
        with mock.patch.object(mind_door, "is_mind", return_value=True):
            self.assertEqual(self.side(peer=PeerCred(4242, ME, ME), token=None), ports.MIND)

    def test_a_caller_below_the_mcp_bridge_is_a_mind_without_asking_anyone(self):
        proc = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, proc, True)
        for pid, ppid, cmd in ((300, 200, b"python3\0yos\0act\0"), (200, 100, b"/opt/yantrik/bin/yos-mcp\0"), (100, 1, b"hermes\0")):
            os.makedirs(os.path.join(proc, str(pid)))
            with open(os.path.join(proc, str(pid), "cmdline"), "wb") as f:
                f.write(cmd)
            with open(os.path.join(proc, str(pid), "stat"), "w") as f:
                f.write("%d (x y) S %d 1 1" % (pid, ppid))

        def never(pid):
            raise AssertionError("the shell was asked")
        self.assertEqual(guard.side_of_caller(peer=PeerCred(300, ME, ME), token=None, ask_shell=never, uid=ME, proc=proc), ports.MIND)


class Vetting(unittest.TestCase):
    def test_each_side_accepts_its_own_browser(self):
        self.assertIsNotNone(guard.Guard(identify=everywhere(listener.MIND_VIEW), uid=ME).vet(ports.MIND))
        for place in (listener.PERSON,):     # the person's, either way it was started
            self.assertIsNotNone(guard.Guard(identify=everywhere(place), uid=ME).vet(ports.PERSON), place)

    def test_a_mind_is_refused_whatever_else_holds_its_port(self):
        for place in (listener.PERSON, listener.IDENTITY):
            with self.assertRaises(guard.Denied) as refused:
                guard.Guard(identify=everywhere(place), uid=ME).vet(ports.MIND)
            text = str(refused.exception)
            self.assertIn("Nothing was done", text)
            self.assertIn("no page was opened, read or changed and no tab list is shown", text)
            self.assertIn("Only the person can change this", text)

    def test_the_person_is_refused_a_minds_browser_on_their_port(self):
        with self.assertRaises(guard.Denied) as refused:
            guard.Guard(identify=everywhere(listener.MIND_VIEW), uid=ME).vet(ports.PERSON)
        self.assertIn("Mind View's own browser", str(refused.exception))

    def test_another_account_and_what_cannot_be_established_are_refused_for_both(self):
        for side in (ports.PERSON, ports.MIND):
            g = guard.Guard(identify=lambda p: listener.Listener(p, ME + 1, 1, (1,), listener.MIND_VIEW), uid=ME)
            with self.assertRaises(guard.Denied):
                g.vet(side)
            nothing = guard.Guard(identify=holding(), uid=ME)
            self.assertIsNone(nothing.vet(side), "nothing listening is not a refusal: the browser is simply not open")

            def unclear(port):
                raise listener.Unidentified("two different sockets listen there")
            with self.assertRaises(guard.Denied) as refused:
                guard.Guard(identify=unclear, uid=ME).vet(side)
            self.assertIn("could not be established", str(refused.exception))


class FirstCallAndOldAddress(unittest.TestCase):
    """A mind's first web call starts Mind View's browser; a person's browser on an old address is
    not "not open"."""

    def setUp(self):
        patch = mock.patch.object(cdp.Browser, "connect", lambda self: None)
        patch.start()
        self.addCleanup(patch.stop)

    def driver(self, side, listening, starter, old=False):
        def identify(port):
            if not listening[0] or port != ports.MIND_VIEW_PORT:
                raise listener.Unidentified("nothing is listening there", nobody=True)
            return listener.Listener(port, ME, 1, (1,), listener.MIND_VIEW)
        g = guard.Guard(identify=identify, uid=ME, side_of=lambda: side, starter=starter, old_address=lambda p: old)
        return drv.Driver(judge=False, guard_=g)

    def test_the_first_call_starts_mind_views_browser_once(self):
        up, calls = [False], []

        def starter():
            calls.append(1)
            up[0] = True
        d = self.driver(ports.MIND, up, starter)
        with d.held():
            self.assertEqual(d.browser.address, "http://127.0.0.1:9223")
        with d.held():
            pass
        self.assertEqual(calls, [1])

    def test_it_cannot_be_started_and_the_sentence_is_the_shells_reason(self):
        def cannot():
            raise guard.Starting("the setting is off")
        d = self.driver(ports.MIND, [False], cannot)
        with self.assertRaises(drv.PageRefused) as refused:
            with d.held():
                pass
        self.assertIn("could not be started: the setting is off", str(refused.exception))

    def test_a_browser_asked_for_that_never_opens_its_port_is_said_so(self):
        d = self.driver(ports.MIND, [False], lambda: None)
        with mock.patch.object(guard.time, "sleep", lambda s: None):
            with mock.patch.object(guard.time, "monotonic", side_effect=[0, 0, 100, 100]):
                with self.assertRaises(drv.PageRefused) as refused:
                    with d.held():
                        pass
        self.assertIn("did not open its debugging port", str(refused.exception))

    def test_the_person_and_a_status_look_start_nothing(self):
        def never():
            raise AssertionError("started a browser")
        with self.driver(ports.PERSON, [False], never).held():
            pass                                  # no browser of theirs is opened for them
        self.driver(ports.MIND, [False], never).status()      # a look starts nothing

    def test_a_persons_browser_on_the_old_address_is_not_called_closed(self):
        d = self.driver(ports.PERSON, [False], None, old=True)
        for act in (lambda: d.read(), lambda: d.tabs()):
            with self.assertRaises(drv.PageRefused) as refused:
                act()
            self.assertIn("old address ([::1])", str(refused.exception))
            self.assertIn("Close it and open it again", str(refused.exception))
            self.assertNotIn("not open", str(refused.exception))
        # A mind's side is not told about the person's browser at all.
        d = self.driver(ports.MIND, [True], None, old=True)
        with d.held():
            pass


class TheDriverSendsEachCallerToItsOwnBrowser(unittest.TestCase):
    """The port choice per caller, with the person's Browser open on its port in every case."""

    def setUp(self):
        # A connect() reaching a refused browser is a failure of the guard, not a "browser closed".
        patch = mock.patch.object(cdp.Browser, "connect", side_effect=AssertionError("a refused call reached a browser"))
        patch.start()
        self.addCleanup(patch.stop)

    def test_a_mind_gets_mind_views_port_and_the_person_theirs(self):
        table = {ports.PERSON_PORT: listener.PERSON, ports.MIND_VIEW_PORT: listener.MIND_VIEW}
        for side, port in ((ports.MIND, 9223), (ports.PERSON, 9222)):
            d = driver_for(lambda p, t=table: listener.Listener(p, ME, 1, (1,), t[p]), side)
            with mock.patch.object(cdp.Browser, "connect", lambda self: None):
                with d.held():
                    self.assertEqual(d.browser.address, "http://127.0.0.1:%d" % port, side)

    def test_a_mind_never_gets_the_persons_port_even_when_mind_views_browser_is_closed(self):
        d = driver_for(holding(**{str(ports.PERSON_PORT): listener.PERSON}), ports.MIND)
        with self.assertRaises(drv.PageRefused):          # it tries to start Mind View's own, and cannot here
            with d.held():
                pass
        self.assertEqual(d.browser.address, "http://127.0.0.1:9223", "never the person's port")

    def test_a_refused_call_opens_nothing_and_says_so(self):
        # The person's Browser is on Mind View's port: a mind's every action is refused.
        d = driver_for(everywhere(listener.PERSON), ports.MIND)
        for act in (lambda: d.go("https://example.com/"), lambda: d.read(), lambda: d.tabs(), lambda: d.text(),
                    lambda: d.click("e1"), lambda: d.close_tab("x")):
            with self.assertRaises(drv.PageRefused) as refused:
                act()
            self.assertTrue(str(refused.exception).startswith("PAGE: Nothing was done."), refused.exception)

    def test_status_of_a_refused_mind_shows_nothing_and_the_person_is_not_shown_the_minds(self):
        d = driver_for(everywhere(listener.PERSON), ports.MIND)
        d.sides.last[ports.PERSON] = {"open": True, "tabs": [{"tab": "aa", "title": "bank", "url": "https://bank.example/", "active": True}]}
        out = d.status()
        self.assertFalse(out["open"])
        self.assertIn("Nothing was done", out["refused"])
        self.assertNotIn("tabs", out)
        # Busy: the answer is the caller's own side's last, never the other's.
        d.lock.acquire()
        try:
            t = __import__("threading").Thread(target=lambda: out.update(d.status()))
            t.start()
            t.join(5)
        finally:
            d.lock.release()
        self.assertTrue(out.get("busy"))
        self.assertNotIn("bank", repr(out))

    def test_a_connection_is_dropped_when_the_port_changes_hands(self):
        state = {"place": listener.MIND_VIEW, "inode": 1}

        def identify(port):
            return listener.Listener(port, ME, state["inode"], (1,), state["place"])
        d = driver_for(identify, ports.MIND)
        browser = d.sides.browsers[ports.MIND]
        with mock.patch.object(cdp.Browser, "connect", lambda self: None):
            with d.held():
                pass
        browser.alive = True                       # a connection made to inode 1
        state["inode"] = 2                         # the port is now another socket
        closed = []
        browser.close = lambda: closed.append(True)
        with mock.patch.object(cdp.Browser, "connect", lambda self: None):
            with d.held():
                pass
        self.assertTrue(closed, "the connection to the old listener must be closed")

    def test_the_connections_own_check_stops_a_swap_between_the_check_and_the_connect(self):
        answers = [listener.Listener(9223, ME, 1, (1,), listener.MIND_VIEW)]

        def identify(port):
            return answers[-1]
        d = driver_for(identify, ports.MIND)
        with d.held():
            check = d.browser.check
        check()                                    # unchanged: fine
        answers.append(listener.Listener(9223, ME, 2, (2,), listener.MIND_VIEW))
        with self.assertRaises(cdp.BrowserClosed):
            check()
        answers.append(listener.Listener(9223, ME, 1, (1,), listener.PERSON))
        with self.assertRaises(guard.Denied):
            check()


if __name__ == "__main__":
    unittest.main()
