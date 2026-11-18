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
import socket
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


PLACES = {}        # pid -> the place the profile of that process puts it in (what listener.place_of_pid reads)


def setUpModule():
    patch = mock.patch.object(listener, "place_of_pid", lambda proc, pid, environ=None: PLACES.get(pid, listener.PERSON))
    patch.start()
    unittest.addModuleCleanup(patch.stop)


class FakeSlot:
    """One side's holder: a browser held (or not), and nothing else of pipes.py."""

    def __init__(self, pid, held, serial=1):
        self.pid, self.held, self.serial = pid, held, serial

    def open(self):
        raise cdp.BrowserClosed()

    def current(self):
        return type("Held", (), {"pid": self.pid, "serial": self.serial})() if self.held else None


class FakePipes:
    """The hand-over's two holders. `held`/`place` are the person's Browser; `mind`/`mind_place`
    Mind View's (not held unless asked, as when it is not open)."""
    proc = "/nonexistent-proc"

    def __init__(self, held=True, uid=ME, place=listener.PERSON, mind=False, mind_place=listener.MIND_VIEW):
        self.uid = uid
        self.slots = {ports.PERSON: FakeSlot(4242, held), ports.MIND: FakeSlot(4343, mind)}
        PLACES[4242], PLACES[4343] = place, mind_place

    def holder(self, side):
        return self.slots[side]

    @property
    def held(self):
        return self.slots[ports.PERSON].held

    @held.setter
    def held(self, value):
        self.slots[ports.PERSON].held = value

    def uid_of(self, pid):
        return self.uid


def minds_browser(**kw):
    """A Guard whose Mind View browser is held over its pipe (and the person's is not)."""
    pipes = FakePipes(held=False, mind=True, **kw)
    return guard.Guard(identify=holding(), uid=ME, pipes=pipes), pipes


def persons_browser(pipes):
    """A Guard whose person-side browser is whatever `pipes` holds."""
    return guard.Guard(identify=holding(), uid=ME, pipes=pipes)


def driver_for(pipes, side):
    """A driver whose callers are all `side`, over fake pipes."""
    g = guard.Guard(identify=holding(), uid=ME, side_of=lambda: side, pipes=pipes)
    return drv.Driver(judge=False, guard_=g)


class WhoIsCalling(unittest.TestCase):
    def side(self, **kw):
        kw.setdefault("ask_shell", lambda pid: "person")
        kw.setdefault("is_shell", lambda pid: False)
        kw.setdefault("uid", ME)
        kw.setdefault("proc", "/nonexistent-proc")
        return guard.side_of_caller(**kw)

    def test_the_person_is_only_a_caller_the_shell_calls_the_person(self):
        me = PeerCred(4242, ME, ME)
        self.assertEqual(self.side(peer=me, token=None), ports.PERSON)
        self.assertEqual(self.side(peer=me, token=None, ask_shell=lambda pid: "mind"), ports.MIND)

    def test_the_shell_is_not_asked_about_itself(self):
        """It reads a grade from here on the thread that would answer: asking back is a wait on
        itself until its lookup gives up, and the card is refused (#1080)."""
        def never(pid):
            raise AssertionError("the shell was asked about itself")
        me = PeerCred(4242, ME, ME)
        # A mind, as `caller_class` says of the shell's own pid (its test `the_shell_itself_is_a_mind`):
        # a mind reaches this browser through the shell, by a glance or a notification's button.
        self.assertEqual(self.side(peer=me, token=None, ask_shell=never, is_shell=lambda pid: pid == 4242),
                         ports.MIND)
        # Another process of the person's is still asked about.
        self.assertEqual(self.side(peer=me, token=None, is_shell=lambda pid: pid == 1), ports.PERSON)

    def test_what_is_a_mind_by_its_own_marks_is_never_taken_for_the_shell(self):
        me = PeerCred(4242, ME, ME)
        always = lambda pid: True  # noqa: E731
        self.assertEqual(self.side(peer=me, token="agent-token", is_shell=always), ports.MIND)
        self.assertEqual(self.side(peer=None, token=None, is_shell=always), ports.MIND)
        self.assertEqual(self.side(peer=PeerCred(4242, ME + 1, ME + 1), token=None, is_shell=always), ports.MIND)

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


class IsTheShell(unittest.TestCase):
    """Who holds the shell's socket, read from the kernel on a real socket."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="guard-shell-")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.path = os.path.join(self.dir, "app-shell.sock")
        patcher = mock.patch.object(guard.wire, "default_socket_path", lambda service: self.path)
        patcher.start()
        self.addCleanup(patcher.stop)

    def listening(self):
        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.addCleanup(server.close)
        server.bind(self.path)
        server.listen(1)
        return server

    def test_only_the_process_on_the_socket_running_the_shells_binary(self):
        self.listening()
        with mock.patch.object(guard.gate, "must_be_the_shell", lambda peer: None):
            self.assertTrue(guard.is_the_shell(os.getpid()))
            self.assertFalse(guard.is_the_shell(os.getpid() + 1), "someone else's pid")
        with mock.patch.object(guard.gate, "must_be_the_shell", lambda peer: "not the shell."):
            self.assertFalse(guard.is_the_shell(os.getpid()), "the socket is held by another binary")

    def test_no_shell_is_no_answer(self):
        self.assertFalse(guard.is_the_shell(os.getpid()))

    def test_a_process_that_is_not_the_shells_binary_is_not_the_shell(self):
        self.listening()        # the real rule: this test's own process is not `yantrik-ui`
        self.assertFalse(guard.is_the_shell(os.getpid()))

    def test_nothing_is_written_to_it(self):
        server = self.listening()
        with mock.patch.object(guard.gate, "must_be_the_shell", lambda peer: None):
            self.assertTrue(guard.is_the_shell(os.getpid()))
        conn, _ = server.accept()
        self.addCleanup(conn.close)
        conn.settimeout(2)
        self.assertEqual(conn.recv(1), b"", "closed having sent nothing")


class Vetting(unittest.TestCase):
    def test_each_side_accepts_its_own_browser(self):
        mg, _ = minds_browser()
        held = mg.vet(ports.MIND)
        self.assertIsNotNone(held)
        self.assertEqual((held.port, held.place), (0, listener.MIND_VIEW), "Mind View's browser is a pipe, not a port")
        g = persons_browser(FakePipes())
        held = g.vet(ports.PERSON)
        self.assertIsNotNone(held)
        self.assertEqual(held.port, 0, "the person's Browser is a pipe, not a port")

    def test_a_mind_is_refused_a_browser_that_is_not_on_mind_views_profile(self):
        for place in (listener.PERSON, listener.IDENTITY):
            g, _ = minds_browser(mind_place=place)
            with self.assertRaises(guard.Denied) as refused:
                g.vet(ports.MIND)
            text = str(refused.exception)
            self.assertIn("Nothing was done", text)
            self.assertIn("no page was opened, read or changed and no tab list is shown", text)
            self.assertIn("Only the person can change this", text)

    def test_the_person_is_refused_a_minds_browser_handed_over_as_theirs(self):
        g = persons_browser(FakePipes(place=listener.MIND_VIEW))
        with self.assertRaises(guard.Denied) as refused:
            g.vet(ports.PERSON)
        self.assertIn("Mind View's own", str(refused.exception))

    def test_another_account_is_refused_and_nothing_held_is_not_a_refusal(self):
        mg, _ = minds_browser()
        mg.pipes.uid = ME + 1
        with self.assertRaises(guard.Denied) as refused:
            mg.vet(ports.MIND)
        self.assertIn("another account", str(refused.exception))
        pg = persons_browser(FakePipes(uid=ME + 1))
        with self.assertRaises(guard.Denied):
            pg.vet(ports.PERSON)
        for side in (ports.PERSON, ports.MIND):
            nothing = guard.Guard(identify=holding(), uid=ME, pipes=FakePipes(held=False))
            self.assertIsNone(nothing.vet(side), "nothing there is not a refusal: the browser is simply not open")

    def test_mind_view_is_never_asked_of_a_port(self):
        """#1200: whatever listens on whatever port, Mind View's side reads its pipe and only that."""
        asked = []
        g = guard.Guard(identify=lambda port: asked.append(port), uid=ME, pipes=FakePipes(held=False, mind=True))
        self.assertIsNotNone(g.vet(ports.MIND))
        self.assertEqual(asked, [], "no port was identified for a mind")

    def test_a_browser_an_older_build_left_on_the_old_port_is_refused_by_name_never_used(self):
        asked = []

        def identify(port):
            asked.append(port)
            return listener.Listener(port, ME, 1, (1,), listener.PERSON)
        g = guard.Guard(identify=identify, uid=ME, pipes=FakePipes(held=False))
        with self.assertRaises(guard.Denied) as refused:
            g.vet(ports.PERSON)
        self.assertIn("started before this update", str(refused.exception))
        self.assertIn("Close it and open your Browser again", str(refused.exception))

    def test_a_port_beside_the_pipe_is_refused_loudly_on_every_call(self):
        """L6: a Browser that holds the pipe and a port too is not left to be driven."""
        g = persons_browser(FakePipes(held=True))
        g.identify = holding(**{str(ports.PERSON_PORT): listener.PERSON})
        for _ in range(2):
            with self.assertRaises(guard.Denied) as refused:
                g.vet(ports.PERSON)
            self.assertIn("lets any process of this account drive it", str(refused.exception))

    def test_a_port_that_cannot_be_identified_is_refused_too(self):
        g = persons_browser(FakePipes(held=True))
        g.identify = holding(**{str(ports.PERSON_PORT): listener.Unidentified("two different sockets listen there")})
        with self.assertRaises(guard.Denied):
            g.vet(ports.PERSON)

    def test_with_the_pipe_and_no_port_the_browser_is_used(self):
        g = persons_browser(FakePipes(held=True))
        self.assertIsNotNone(g.vet(ports.PERSON))

    def test_neither_side_has_a_port_to_fall_back_to(self):
        for side in (ports.PERSON, ports.MIND):
            d = drv.Driver(judge=False, guard_=guard.Guard(identify=holding(), uid=ME, side_of=lambda: side,
                                                           pipes=FakePipes(held=False)))
            browser = d.sides.browsers[side]
            self.assertIsNone(browser.address)
            with mock.patch("urllib.request.urlopen", side_effect=AssertionError("TCP was tried")):
                with mock.patch.object(cdp.Browser, "_version", side_effect=AssertionError("the port was asked")):
                    with self.assertRaises(cdp.BrowserClosed):
                        browser._open()


class CallerGone(unittest.TestCase):
    """A call is for the caller it came from, decided before it waits, and not carried out for one
    that has gone: a killed Hermes' queued web_go must not land in the person's tab (#1085)."""

    class Peer:
        def __init__(self, gone=False):
            self.flag = gone

        def gone(self):
            return self.flag

    def driver(self, side, peer, asked):
        def side_of():
            asked.append(1)
            return side
        g = guard.Guard(identify=holding(), uid=ME, side_of=side_of, watch=lambda who: peer,
                        pipes=FakePipes(held=False, mind=True))
        return drv.Driver(judge=False, guard_=g)

    def test_the_side_is_decided_before_the_wait_and_a_caller_that_left_while_waiting_does_nothing(self):
        import threading
        import time
        peer, asked = self.Peer(), []
        d = self.driver(ports.MIND, peer, asked)
        d.lock.acquire()                                   # another act is running
        result = []

        def queued():
            try:
                with d.held():
                    result.append("ran")
            except drv.PageRefused as e:
                result.append(str(e))
        t = threading.Thread(target=queued)
        t.start()
        time.sleep(0.3)
        self.assertEqual(asked, [1], "who it is for was decided before it waited for the lock")
        peer.flag = True                                   # the caller is killed while it waits
        d.lock.release()
        t.join(10)
        self.assertEqual(len(result), 1)
        self.assertIn("gone away", result[0])
        self.assertNotEqual(result[0], "ran")

    def test_a_caller_already_gone_when_the_call_is_handled_is_a_mind_and_does_nothing(self):
        d = self.driver(ports.PERSON, self.Peer(gone=True), [])
        side, gone = d.sides.decide()
        self.assertEqual(side, ports.MIND, "an exited caller is not shown to be the person")
        self.assertTrue(gone())
        with self.assertRaises(drv.PageRefused):
            with d.held():
                pass

    def test_a_live_caller_goes_on(self):
        d = self.driver(ports.MIND, self.Peer(), [])
        with d.held():
            pass

    def test_the_real_watch_sees_a_process_exit(self):
        import subprocess
        p = subprocess.Popen(["sleep", "60"])
        self.addCleanup(p.kill)
        w = guard.Watch(p.pid)
        self.assertFalse(w.gone())
        p.kill()
        p.wait()
        self.assertTrue(w.gone())
        self.assertTrue(guard.Watch(None).gone())
        self.assertTrue(guard.Watch(2 ** 22 + 12345).gone(), "a pid nothing has")

    def test_the_mcp_bridge_is_found_as_a_real_script_shows_in_the_process_list(self):
        proc = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, proc, True)
        for pid, ppid, cmd in ((300, 200, b"python3\0yos\0"), (200, 100, b"/usr/bin/python3\0/opt/yantrik/bin/yos-mcp\0"), (100, 1, b"hermes\0")):
            os.makedirs(os.path.join(proc, str(pid)))
            with open(os.path.join(proc, str(pid), "cmdline"), "wb") as f:
                f.write(cmd)
            with open(os.path.join(proc, str(pid), "stat"), "w") as f:
                f.write("%d (x) S %d 1 1" % (pid, ppid))
        self.assertTrue(guard.is_bridge_ancestor(300, proc))
        self.assertFalse(guard.is_bridge_ancestor(100, proc))


class FirstCallAndOldAddress(unittest.TestCase):
    """A mind's first web call starts Mind View's browser."""

    def setUp(self):
        patch = mock.patch.object(cdp.Browser, "connect", lambda self: None)
        patch.start()

    def driver(self, side, listening, starter):
        """`listening[0]` is whether Mind View's browser has been handed over yet."""
        pipes = FakePipes(held=False)
        slot = pipes.slots[ports.MIND]

        def start():
            starter()
            slot.held = listening[0]
        g = guard.Guard(identify=holding(), uid=ME, side_of=lambda: side, starter=start, pipes=pipes)
        slot.held = listening[0]
        d = drv.Driver(judge=False, guard_=g)
        d.fake_slot = slot
        return d

    def test_a_minds_first_call_being_tabs_starts_it_too(self):
        """520: with the person's Browser open first, a mind's web_tabs was told "the browser is not
        open. Open it: act shell open_app name=browser", because only some calls started one."""
        up, calls = [False], []

        def starter():
            calls.append(1)
            up[0] = True
        d = self.driver(ports.MIND, up, starter)
        self.assertTrue(d.tabs()["open"])
        self.assertEqual(calls, [1])
        d.tabs()
        self.assertEqual(calls, [1])
        # The person's tabs never start one, and a look at the view (describe) starts none.
        never = self.driver(ports.PERSON, [False], lambda: self.fail("started"))
        never.tabs()
        self.driver(ports.MIND, [False], lambda: self.fail("started")).status()

    def test_the_first_call_starts_mind_views_browser_once(self):
        up, calls = [False], []

        def starter():
            calls.append(1)
            up[0] = True
        d = self.driver(ports.MIND, up, starter)
        with d.held():
            self.assertIsNotNone(d.browser.opener, "Mind View's pipe")
            self.assertIsNone(d.browser.address)
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

    def test_a_browser_asked_for_that_is_never_handed_over_is_said_so(self):
        d = self.driver(ports.MIND, [False], lambda: None)
        with mock.patch.object(guard.time, "sleep", lambda s: None):
            with mock.patch.object(guard.time, "monotonic", side_effect=[0, 0, 100, 100]):
                with self.assertRaises(drv.PageRefused) as refused:
                    with d.held():
                        pass
        self.assertIn("was not handed over", str(refused.exception))

    def test_the_person_and_a_status_look_start_nothing(self):
        def never():
            raise AssertionError("started a browser")
        with self.driver(ports.PERSON, [False], never).held():
            pass                                  # no browser of theirs is opened for them
        self.driver(ports.MIND, [False], never).status()      # a look starts nothing

    def test_a_minds_first_call_gets_its_own_whatever_the_persons_browser_is_doing(self):
        """The 2 Nov bug (#1137): a mind's web_go landed in the person's browser. Whether the person's
        Browser is open or not, in either start order, a mind's call is Mind View's, over its pipe."""
        for persons_open in (True, False):
            calls = []
            pipes = FakePipes(held=persons_open)
            mind = pipes.slots[ports.MIND]
            g = guard.Guard(identify=holding(), uid=ME, side_of=lambda: ports.MIND,
                            starter=lambda: (calls.append(1), setattr(mind, "held", True)), pipes=pipes)
            d = drv.Driver(judge=False, guard_=g)
            with d.held():
                self.assertIs(d.browser, d.sides.browsers[ports.MIND], persons_open)
                self.assertIsNone(d.browser.address)
            self.assertEqual(calls, [1], "Mind View's was started for it")
            # The person's Browser opening afterwards changes nothing for the next mind call.
            pipes.held = True
            with d.held():
                self.assertIs(d.browser, d.sides.browsers[ports.MIND])


class TheDriverSendsEachCallerToItsOwnBrowser(unittest.TestCase):
    """The choice per caller, with the person's Browser open in every case."""

    def setUp(self):
        # A connect() reaching a refused browser is a failure of the guard, not a "browser closed".
        patch = mock.patch.object(cdp.Browser, "connect", side_effect=AssertionError("a refused call reached a browser"))
        patch.start()

    def test_a_mind_gets_mind_views_pipe_and_the_person_theirs(self):
        for side in (ports.MIND, ports.PERSON):
            g = guard.Guard(identify=holding(), uid=ME, side_of=lambda s=side: s,
                            pipes=FakePipes(held=True, mind=True))
            d = drv.Driver(judge=False, guard_=g)
            with mock.patch.object(cdp.Browser, "connect", lambda self: None):
                with d.held():
                    self.assertIsNone(d.browser.address, "neither browser has an address")
                    self.assertIs(d.browser.opener.__self__, g.pipes.holder(side), "it is that side's pipe")

    def test_a_mind_never_gets_the_persons_browser_even_when_mind_views_is_closed(self):
        g = guard.Guard(identify=holding(), uid=ME, side_of=lambda: ports.MIND, pipes=FakePipes())
        d = drv.Driver(judge=False, guard_=g)
        with self.assertRaises(drv.PageRefused):          # it tries to start Mind View's own, and cannot here
            with d.held():
                pass
        self.assertIs(d.browser, d.sides.browsers[ports.MIND], "never the person's")
        self.assertIsNot(d.browser.opener.__self__, g.pipes.holder(ports.PERSON))

    def test_a_refused_call_opens_nothing_and_says_so(self):
        # What was handed over as Mind View's is the person's browser: a mind's every action is refused.
        d = driver_for(FakePipes(held=False, mind=True, mind_place=listener.PERSON), ports.MIND)
        for act in (lambda: d.go("https://example.com/"), lambda: d.read(), lambda: d.tabs(), lambda: d.text(),
                    lambda: d.click("e1"), lambda: d.close_tab("x")):
            with self.assertRaises(drv.PageRefused) as refused:
                act()
            self.assertTrue(str(refused.exception).startswith("PAGE: Nothing was done."), refused.exception)

    def test_status_of_a_refused_mind_shows_nothing_and_the_person_is_not_shown_the_minds(self):
        d = driver_for(FakePipes(held=False, mind=True, mind_place=listener.PERSON), ports.MIND)
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

    def test_a_connection_is_dropped_when_the_pipe_changes_hands(self):
        pipes = FakePipes(held=False, mind=True)
        d = driver_for(pipes, ports.MIND)
        browser = d.sides.browsers[ports.MIND]
        with mock.patch.object(cdp.Browser, "connect", lambda self: None):
            with d.held():
                pass
        browser.alive = True                       # a connection made to hand-over 1
        pipes.slots[ports.MIND].serial = 2         # Mind View's browser was handed over again
        closed = []
        browser.close = lambda: closed.append(True)
        with mock.patch.object(cdp.Browser, "connect", lambda self: None):
            with d.held():
                pass
        self.assertTrue(closed, "the connection to the old browser must be closed")

    def test_the_connections_own_check_stops_a_swap_between_the_check_and_the_connect(self):
        pipes = FakePipes(held=False, mind=True)
        d = driver_for(pipes, ports.MIND)
        with d.held():
            check = d.browser.check
        check()                                    # unchanged: fine
        pipes.slots[ports.MIND].serial = 2
        with self.assertRaises(cdp.BrowserClosed):
            check()
        pipes.slots[ports.MIND].serial = 1
        PLACES[4343] = listener.PERSON             # and the browser held is not Mind View's after all
        with self.assertRaises(guard.Denied):
            check()


if __name__ == "__main__":
    unittest.main()
