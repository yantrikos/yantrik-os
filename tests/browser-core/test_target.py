#!/usr/bin/env python3
"""Which browser a caller may use (#1085): a fake listener for every case, no real browser.

    python3 -m unittest discover -s tests/browser-core -p 'test_target.py' -v

Linux only (it builds a fake /proc with symlinks). Each case is a browser on the DevTools port as
the machine would show it — the socket table, the process holding the socket, its command line and
working directory — and a caller (a Mind, or the person). What is asserted is what the caller is
let do: refused with nothing sent to the browser, or allowed and reaching it.
"""

import json
import os
import queue
import shutil
import sys
import tempfile
import threading
import types
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]

from yantrik_browser import cdp, driver as drv, service, target  # noqa: E402

ADDRESS = "http://127.0.0.1:9222"
ME = os.getuid()


class FakeProc:
    """A /proc with listeners in it."""

    def __init__(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = os.path.join(self.tmp.name, "proc")
        os.makedirs(os.path.join(self.root, "net"))
        self.rows = {"tcp": [], "tcp6": []}
        self.home = os.path.join(self.tmp.name, "home")
        self.mind_view = os.path.join(self.home, ".local/share/yantrik/mind-view-browser/chromium")
        self.persons = os.path.join(self.home, ".config/chromium")
        for d in (self.mind_view, self.persons):
            os.makedirs(d)
        self.write_tables()

    def write_tables(self):
        head = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n"
        for table, rows in self.rows.items():
            with open(os.path.join(self.root, "net", table), "w") as f:
                f.write(head + "".join(rows))

    def listener(self, pid, argv, *, port=9222, addr="0100007F", table="tcp", uid=ME, inode=None,
                 cwd=None, holds=True, start=100, state="0A"):
        inode = inode or 5000 + pid
        self.rows[table].append("   0: %s:%04X 00000000:0000 %s 00000000:00000000 00:00000000 "
                                "00000000 %5d        0 %d 1 0\n" % (addr, port, state, uid, inode))
        self.write_tables()
        if holds:
            self.process(pid, argv, inode=inode, cwd=cwd, start=start)
        return inode

    def process(self, pid, argv, *, inode, cwd=None, start=100):
        p = os.path.join(self.root, str(pid))
        os.makedirs(os.path.join(p, "fd"), exist_ok=True)
        os.symlink("socket:[%d]" % inode, os.path.join(p, "fd", "7"))
        with open(os.path.join(p, "cmdline"), "wb") as f:
            f.write(b"\0".join(a.encode() for a in argv) + b"\0")
        os.symlink(cwd or "/", os.path.join(p, "cwd"))
        with open(os.path.join(p, "stat"), "w") as f:
            f.write("%d (chromium) S 1 %s\n" % (pid, " ".join(["0"] * 17 + [str(start)])))

    def clear(self):
        """No listeners and no processes; the directories stay."""
        shutil.rmtree(self.root)
        os.makedirs(os.path.join(self.root, "net"))
        self.rows = {"tcp": [], "tcp6": []}
        self.write_tables()

    def identify(self, address):
        return target.identify(address, proc=self.root, mind_profile=self.mind_view)


class FakeWs:
    """The browser's end of a DevTools connection: answers what the connection asks, remembers it."""

    def __init__(self, log):
        self.log, self.q, self.closed = log, queue.Queue(), False

    def send(self, raw):
        m = json.loads(raw)
        self.log.append((self, m["method"]))
        res = {}
        if m["method"] == "Target.getTargets":
            res = {"targetInfos": [{"targetId": "TAB1", "type": "page", "url": "https://their.example/",
                                    "title": "Their tab"}]}
        elif m["method"] == "Target.attachToTarget":
            res = {"sessionId": "S1"}
        elif m["method"] == "Page.getFrameTree":
            res = {"frameTree": {"frame": {"id": "F1"}}}
        self.q.put(json.dumps({"id": m["id"], "result": res}))

    def recv(self):
        v = self.q.get()
        if v is None:
            raise OSError("closed")
        return v

    def close(self):
        self.closed = True
        self.q.put(None)


# What each action is called with. Every one must be refused alike.
ACTIONS = {
    "read": lambda d: d.read(), "find": lambda d: d.find("x"), "text": lambda d: d.text(),
    "tabs": lambda d: d.tabs(), "media": lambda d: d.media(), "wait": lambda d: d.wait("x", 0.5),
    "wait-settle": lambda d: d.wait(None, 0.5), "scroll": lambda d: d.scroll(),
    "go": lambda d: d.go("https://example.com/"), "go-new-tab": lambda d: d.go("https://example.com/", new_tab=True),
    "back": lambda d: d.history(-1), "forward": lambda d: d.history(1), "reload": lambda d: d.reload(),
    "click": lambda d: d.click("e1"), "type": lambda d: d.type("e1", "x"), "press": lambda d: d.press("Enter"),
    "select": lambda d: d.select("e1", "o"), "dialog": lambda d: d.dialog(),
    "switch_tab": lambda d: d.switch("TAB1"), "close_tab": lambda d: d.close_tab("TAB1"),
    "commit": lambda d: d.commit("e1", "Buy", "shop.example"),
}


class Case(unittest.TestCase):
    """A fake machine, and a driver for whoever is `self.person` (False: a Mind)."""

    def setUp(self):
        self.proc = FakeProc()
        self.addCleanup(self.proc.tmp.cleanup)
        self.person = False
        self.log = []
        self.opened = []
        fake = types.SimpleNamespace(create_connection=self.create_connection)
        for p in (mock.patch.dict(sys.modules, {"websocket": fake}),
                  mock.patch.object(cdp.Browser, "_version",
                                    lambda s: {"webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/browser/x"}),
                  mock.patch.object(drv.hands, "settle", lambda *a, **k: (0.0, "ok")),
                  mock.patch.object(drv.Driver, "_consequence", lambda *a, **k: {"did": "it"})):
            p.start()
            self.addCleanup(p.stop)
        self.browser = cdp.Browser(ADDRESS, identify=self.proc.identify)
        self.addCleanup(self.browser.close)
        self.guard = target.Guard(is_person=lambda: self.person)
        self.d = drv.Driver(self.browser, judge=False, guard=self.guard)

    def create_connection(self, url, **kw):
        ws = FakeWs(self.log)
        self.opened.append(ws)
        return ws

    # the browsers. What they were started with is whatever the argv says; what they own is the
    # profile their SingletonLock names (or the LOCK file they hold).
    def owns(self, pid, profile):
        lock = os.path.join(profile, "SingletonLock")
        if os.path.lexists(lock):
            os.remove(lock)
        os.symlink("somehost-%d" % pid, lock)

    def persons_browser(self, pid=10, argv=None, **kw):
        inode = self.proc.listener(pid, argv or ["/usr/bin/chromium", "--remote-debugging-port=9222"], **kw)
        self.owns(pid, self.proc.persons)
        return inode

    def mind_view_browser(self, pid=20, argv=None, **kw):
        inode = self.proc.listener(pid, argv or ["/usr/bin/chromium", "--remote-debugging-port=9222"], **kw)
        self.owns(pid, self.proc.mind_view)
        return inode

    # what a caller is let do
    def refused(self, action, fn=None):
        with self.assertRaises(drv.PageRefused, msg=action) as got:
            (fn or ACTIONS[action])(self.d)
        self.assertIn("Nothing was done", str(got.exception), action)
        return str(got.exception)

    def assert_every_action_refused(self):
        for name in ACTIONS:
            self.refused(name)
        status = self.d.status()
        self.assertTrue(status.get("refused") and "tabs" not in status, status)
        self.assertEqual(self.opened, [], "a connection was opened to a browser the caller may not use")
        self.assertEqual(self.log, [], "something was sent to a browser the caller may not use")

    def assert_allowed(self):
        shown = self.d.tabs()
        self.assertEqual([t["url"] for t in shown["tabs"]], ["https://their.example/"])
        self.d.go("https://example.com/")      # reaches the browser (settle and the consequence are faked)
        self.assertTrue(self.opened)


class TestOtherBrowsers(Case):
    def test_the_persons_profile_is_refused_to_a_mind_in_every_action_and_open_to_the_person(self):
        self.persons_browser()
        self.assert_every_action_refused()
        self.person = True
        self.assert_allowed()

    def test_a_browser_with_no_user_data_dir_is_the_persons_default_profile(self):
        self.proc.listener(10, ["/usr/bin/chromium", "--remote-debugging-address=127.0.0.1",
                                "--remote-debugging-port=9222"])       # owns no profile of Mind View's
        self.assert_every_action_refused()
        self.person = True
        self.assert_allowed()

    def test_mind_views_profile_is_open_to_a_mind_and_not_to_the_person(self):
        self.mind_view_browser()
        self.assert_allowed()
        self.person = True
        self.browser.close()
        for name in ACTIONS:
            self.refused(name)
        self.assertIn("Mind View", self.refused("go"))

    def test_another_accounts_browser_is_refused_to_everyone(self):
        self.mind_view_browser(uid=ME + 1)
        self.assert_every_action_refused()
        self.person = True
        self.assertIn("another account", self.refused("read"))

    def test_a_refusal_names_the_reason_and_a_remedy_only_the_person_can_take(self):
        self.persons_browser()
        said = self.refused("go")
        self.assertIn("person's own", said)
        self.assertIn("close that browser", said)
        self.assertNotIn("open_app", said)
        self.assertNotIn("error", said.lower())


class TestUnidentifiable(Case):
    """A browser the check cannot establish is refused, to a Mind and to the person."""

    def both_refused(self):
        for person in (False, True):
            self.person = person
            self.assert_every_action_refused()

    def test_a_socket_no_process_holds(self):
        self.proc.listener(10, ["x"], holds=False)
        self.both_refused()

    @unittest.skipIf(os.getuid() == 0, "root reads everything")
    def test_a_process_whose_files_cannot_be_listed(self):
        self.mind_view_browser()
        fds = os.path.join(self.proc.root, "20", "fd")
        os.chmod(fds, 0)
        self.addCleanup(os.chmod, fds, 0o700)
        self.both_refused()

    def test_two_sockets_on_the_port(self):
        self.persons_browser(10)
        self.mind_view_browser(20)
        self.both_refused()

    def test_an_address_that_is_not_a_loopback_literal(self):
        self.mind_view_browser()
        self.browser.address = "http://localhost:9222"
        self.both_refused()

    def test_a_table_that_cannot_be_read(self):
        self.persons_browser()
        with open(os.path.join(self.proc.root, "net", "tcp"), "w"):
            pass
        os.chmod(os.path.join(self.proc.root, "net", "tcp"), 0)
        self.addCleanup(os.chmod, os.path.join(self.proc.root, "net", "tcp"), 0o600)
        if os.getuid() == 0:
            self.skipTest("root reads everything")
        self.both_refused()


class TestWhatMakesAProcessMindViews(Case):
    """Not its arguments: the profile's SingletonLock names it, or it holds one of its LOCK files."""

    def mind_may_use(self, build):
        self.proc.clear()
        for d in (self.proc.mind_view, self.proc.persons):
            for name in os.listdir(d):
                path = os.path.join(d, name)
                os.remove(path) if not os.path.isdir(path) or os.path.islink(path) else shutil.rmtree(path)
        build()
        try:
            self.d.tabs()
            return True
        except drv.PageRefused:
            return False
        finally:
            self.browser.close()

    def hold(self, pid, path):
        """The process holds `path` open (a file made there if it is not)."""
        os.makedirs(os.path.dirname(path), exist_ok=True)
        open(path, "a").close()
        os.symlink(path, os.path.join(self.proc.root, str(pid), "fd", "9"))

    def test_the_singleton_lock_naming_the_listener_is_enough(self):
        self.assertTrue(self.mind_may_use(lambda: self.mind_view_browser(20)))

    def test_a_lock_file_held_open_under_the_profile_is_enough(self):
        def build():
            self.proc.listener(20, ["chromium"])
            self.hold(20, os.path.join(self.proc.mind_view, "Default", "shared_proto_db", "metadata", "LOCK"))
        self.assertTrue(self.mind_may_use(build))

    def test_a_cookie_database_held_open_is_not_enough(self):
        def build():
            self.proc.listener(20, ["chromium"])
            self.hold(20, os.path.join(self.proc.mind_view, "Default", "Cookies"))
        self.assertFalse(self.mind_may_use(build))

    def test_a_lock_file_in_another_profile_is_not_enough(self):
        def build():
            self.proc.listener(20, ["chromium"])
            self.hold(20, os.path.join(self.proc.persons, "Default", "LOCK"))
        self.assertFalse(self.mind_may_use(build))

    def test_a_singleton_lock_naming_another_process_is_not_enough(self):
        def build():
            self.proc.listener(20, ["chromium"])
            self.owns(21, self.proc.mind_view)
        self.assertFalse(self.mind_may_use(build))

    def test_a_lock_file_outside_any_profile_named_lock_is_not_enough(self):
        def build():
            self.proc.listener(20, ["chromium"])
            self.hold(20, os.path.join(self.proc.tmp.name, "elsewhere", "LOCK"))
        self.assertFalse(self.mind_may_use(build))

    def test_a_link_to_the_profile_that_holds_the_lock_is_the_profile(self):
        link = os.path.join(self.proc.tmp.name, "mv-link")
        os.symlink(self.proc.mind_view, link)
        self.browser.identify = lambda address: target.identify(address, proc=self.proc.root, mind_profile=link)
        self.assertTrue(self.mind_may_use(lambda: self.mind_view_browser(20)))

    def test_a_lock_file_is_found_when_the_profile_is_named_by_a_link(self):
        link = os.path.join(self.proc.tmp.name, "mv-link")
        os.symlink(self.proc.mind_view, link)
        self.browser.identify = lambda address: target.identify(address, proc=self.proc.root, mind_profile=link)

        def build():
            self.proc.listener(20, ["chromium"])
            self.hold(20, os.path.join(self.proc.mind_view, "Default", "x", "LOCK"))
        self.assertTrue(self.mind_may_use(build))

    def test_any_process_sharing_the_socket_may_be_the_one_that_owns_the_profile(self):
        for owner, other in ((21, 22), (22, 21)):
            def build():
                inode = self.proc.listener(owner, ["chromium"])
                self.proc.process(other, ["chromium"], inode=inode)
                self.owns(owner, self.proc.mind_view)
            self.assertTrue(self.mind_may_use(build), owner)

    def test_a_lock_file_reached_through_dots_and_links_is_resolved(self):
        def build():
            self.proc.listener(20, ["chromium"])
            self.hold(20, os.path.join(self.proc.persons, "Default", "LOCK"))
            os.remove(os.path.join(self.proc.root, "20", "fd", "9"))
            sneaky = os.path.join(self.proc.mind_view, "..", "..", "..", "..", ".config", "chromium", "Default", "LOCK")
            os.symlink(sneaky, os.path.join(self.proc.root, "20", "fd", "9"))
        self.assertFalse(self.mind_may_use(build))

    def test_the_arguments_prove_nothing_either_way(self):
        mv, theirs = self.proc.mind_view, self.proc.persons
        spoofs = [
            ["chromium", "--user-data-dir=" + mv],                              # the flag, on the person's
            ["chromium", "-user-data-dir=" + mv, "--remote-debugging-port=9222"],
            ["chromium", "https://x.example/?a=--user-data-dir=" + mv],         # inside a URL
            ["chromium --user-data-dir=%s --remote-debugging-port=9222" % mv],  # one rewritten string
            ["chromium", "--", "--user-data-dir=" + mv],
            ["chromium", "--user-data-dir=" + theirs, "--user-data-dir=" + mv],
        ]
        for argv in spoofs:
            self.assertFalse(self.mind_may_use(lambda: self.persons_browser(20, argv)), argv)
        honest_but_rewritten = [
            ["chromium --remote-debugging-port=9222 --user-data-dir=%s about:blank" % mv],  # as Chromium leaves it
            ["chromium"], ["chromium", "--user-data-dir=" + theirs],                       # the arguments claim otherwise
        ]
        for argv in honest_but_rewritten:
            self.assertTrue(self.mind_may_use(lambda: self.mind_view_browser(20, argv)), argv)


class TestAddresses(Case):
    def test_a_browser_on_ipv6_loopback_only_is_not_what_the_service_connects_to(self):
        self.persons_browser(table="tcp6", addr="00000000000000000000000001000000")
        self.assertEqual(self.d.status(), {"open": False})
        with self.assertRaises(drv.Refused):
            self.d.read()
        self.assertEqual(self.opened, [])

    def test_mind_views_ipv4_browser_is_the_one_reached_whatever_is_on_ipv6(self):
        self.mind_view_browser(10)
        self.persons_browser(20, table="tcp6", addr="00000000000000000000000001000000")
        self.assert_allowed()
        self.person = True
        self.browser.close()
        self.assertIn("Mind View", self.refused("read"))

    def test_the_exact_address_wins_over_a_wildcard(self):
        self.persons_browser(10, addr="00000000")
        self.mind_view_browser(20)
        self.assert_allowed()

    def test_a_wildcard_listener_answers_when_it_is_alone(self):
        self.persons_browser(10, addr="00000000")
        self.assert_every_action_refused()
        self.proc.clear()
        self.persons_browser(10, table="tcp6", addr="0" * 32)       # `::`, dual-stack
        self.person = False
        self.assert_every_action_refused()

    def test_a_listener_that_is_not_listening_is_not_a_browser(self):
        self.proc.listener(10, ["chromium"], state="01")
        self.assertEqual(self.d.status(), {"open": False})


class TestKernelsAndMappedAddresses(Case):
    def test_a_kernel_with_no_ipv6_table_still_finds_the_ipv4_browser(self):
        self.mind_view_browser(20)
        os.remove(os.path.join(self.proc.root, "net", "tcp6"))
        self.assert_allowed()

    def test_a_missing_ipv4_table_is_not_guessed_at(self):
        self.mind_view_browser(20)
        os.remove(os.path.join(self.proc.root, "net", "tcp"))
        self.assert_every_action_refused()

    def test_an_ipv4_mapped_loopback_listener_is_the_exact_address(self):
        self.persons_browser(10, addr="0" * 16 + "FFFF0000" + "0100007F", table="tcp6")
        self.mind_view_browser(20, addr="00000000")      # a wildcard would lose to it
        self.assert_every_action_refused()


class TestTheTargetChanges(Case):
    def test_a_browser_that_replaces_the_one_a_mind_was_using_is_judged_afresh(self):
        self.mind_view_browser(20)
        self.assertTrue(self.d.tabs()["tabs"])
        first = self.opened[0]
        sent = len(self.log)
        # Mind View's browser goes, and the person's starts on the same port
        self.proc.clear()
        self.persons_browser(30)
        self.refused("go")
        self.assertTrue(first.closed)
        self.assertEqual(len(self.log), sent, "something was sent after the browser changed")
        self.assertEqual(len(self.opened), 1)

    def test_a_restart_of_the_same_kind_of_browser_is_a_new_listener_and_a_new_connection(self):
        self.mind_view_browser(20, start=100)
        self.d.tabs()
        self.proc.clear()
        self.mind_view_browser(21, start=900)
        self.d.tabs()
        self.assertEqual(len(self.opened), 2)
        self.assertTrue(self.opened[0].closed)

    def test_a_reused_pid_is_not_the_same_listener(self):
        self.mind_view_browser(20, start=100)
        before = self.browser.who()
        self.proc.clear()
        self.mind_view_browser(20, start=900)
        self.assertNotEqual(before, self.browser.who())

    def test_the_port_changing_hands_while_the_connection_is_made_is_refused(self):
        """The check saw Mind View's browser, and judged it fit; by the time the socket was open
        the person's was on the port. The connection is dropped before one command goes over it."""
        self.mind_view_browser(20)
        mv = self.browser.who()
        self.proc.clear()
        self.persons_browser(30)
        theirs = self.browser.who()
        # admit, then the connection's own look before opening, then its look after
        self.browser.identify = mock.Mock(side_effect=[mv, mv, theirs])
        self.refused("read")
        self.assertTrue(self.opened and all(ws.closed for ws in self.opened))
        self.assertEqual(self.log, [], "something was sent over a connection that was not kept")

    def test_the_port_changing_hands_between_the_check_and_the_connection_never_opens_a_socket(self):
        """admit saw Mind View's browser; the connection's own look finds the person's."""
        self.mind_view_browser(20)
        mv = self.browser.who()
        self.proc.clear()
        self.persons_browser(30)
        theirs = self.browser.who()
        self.browser.identify = mock.Mock(side_effect=[mv, theirs, theirs])
        self.refused("read")
        self.assertEqual(self.opened, [])

    def test_a_browser_that_goes_away_is_closed_on_and_the_next_action_says_it_is_not_open(self):
        self.mind_view_browser(20)
        self.d.tabs()
        self.proc.clear()
        with self.assertRaises(drv.Refused) as got:
            self.d.read()
        self.assertNotIsInstance(got.exception, drv.PageRefused)
        self.assertTrue(self.opened[0].closed)

    def test_one_connection_serves_consecutive_actions(self):
        self.mind_view_browser(20)
        self.d.tabs()
        self.d.tabs()
        self.d.go("https://example.com/")
        self.assertEqual(len(self.opened), 1)

    def test_a_connection_made_for_the_person_is_not_kept_for_a_mind(self):
        self.persons_browser(10)
        self.person = True
        self.d.tabs()
        self.person = False
        self.refused("tabs")
        self.refused("read")

    def test_a_connection_judged_for_a_mind_is_judged_again_for_the_person(self):
        self.mind_view_browser(10)
        self.d.tabs()
        self.person = True
        self.refused("tabs")


class TestWhoIsAMind(unittest.TestCase):
    def verdict(self, peer, token=None, is_mind=False, proven=True):
        with mock.patch.object(target, "caller", lambda: peer),              mock.patch.object(target, "agent_token", lambda: token),              mock.patch.object(target.mind_door, "is_mind", lambda uid: is_mind),              mock.patch.object(target.who, "proves_person", lambda pid: proven):
            return target.caller_is_person()

    def test_only_the_persons_own_untokened_call_with_a_proving_ancestry_is_the_person(self):
        from yantrik_surface import PeerCred
        self.assertTrue(self.verdict(PeerCred(1, ME, ME)))

    def test_every_caller_it_cannot_attribute_is_treated_as_a_mind(self):
        from yantrik_surface import PeerCred
        self.assertFalse(self.verdict(None))
        self.assertFalse(self.verdict(PeerCred(1, ME, ME), token="agent-token"))
        self.assertFalse(self.verdict(PeerCred(1, ME + 1, ME)))
        self.assertFalse(self.verdict(PeerCred(1, ME + 1, ME), is_mind=True))
        self.assertFalse(self.verdict(PeerCred(1, ME, ME), is_mind=True))

    def test_a_tokenless_caller_on_the_persons_uid_whose_ancestry_does_not_prove_it_is_a_mind(self):
        from yantrik_surface import PeerCred
        self.assertFalse(self.verdict(PeerCred(1, ME, ME), proven=False))


class TestViews(Case):
    def test_a_mind_is_shown_no_tab_list_in_a_description_or_when_the_browser_is_busy(self):
        self.persons_browser(10)
        self.person = True
        self.d.status()                          # the person's tabs are now what it last knew
        self.person = False
        said = service.build(self.d)[0].snapshot()
        self.assertIn("Nothing was done", said[0])
        self.assertNotIn("Their tab", json.dumps(said))
        with self.d.lock:                        # another thread holds the browser
            seen = []
            t = threading.Thread(target=lambda: seen.append(self.d.status()))
            t.start()
            t.join(5)
        self.assertEqual(seen, [{"busy": True}])

    def test_a_busy_browser_is_busy_not_closed(self):
        self.mind_view_browser(10)
        with self.d.lock:
            seen = []
            t = threading.Thread(target=lambda: seen.append(service.build(self.d)[0].snapshot()[0]))
            t.start()
            t.join(5)
        self.assertEqual(seen, ["Browser — busy"])

    def test_the_person_is_told_when_the_browser_they_reach_is_mind_views(self):
        self.mind_view_browser(10)
        self.person = True
        self.assertIn("Mind View's own browser", service.build(self.d)[0].snapshot()[0])


class TestOneDefinition(unittest.TestCase):
    def test_the_profile_is_the_one_the_shell_starts_mind_views_browser_with(self):
        with open(os.path.join(ROOT, "crates/yantrik-ui/src/wire/dock.rs"), encoding="utf-8") as f:
            rust = f.read()
        self.assertRegex(rust, r'base\.join\("yantrik"\)\.join\("mind-view-browser"\)')
        self.assertRegex(rust, r'profile\.join\("chromium"\)\.display\(\)\)')
        self.assertRegex(rust, r'std::env::var_os\("XDG_DATA_HOME"\)')
        with mock.patch.dict(os.environ, {"XDG_DATA_HOME": "/data"}):
            self.assertEqual(target.mind_view_profile(), "/data/yantrik/mind-view-browser/chromium")


class TestARealListener(unittest.TestCase):
    @unittest.skipUnless(os.path.exists("/proc/net/tcp"), "needs /proc")
    def test_this_very_process_is_found_listening_on_loopback(self):
        import socket
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            s.listen(1)
            port = s.getsockname()[1]
            got = target.identify("http://127.0.0.1:%d" % port)
        self.assertEqual(got.pids, (os.getpid(),))
        self.assertEqual(got.uid, ME)
        self.assertFalse(got.mind_view)
        self.assertIsNotNone(target.decide(got, False))
        self.assertIsNone(target.decide(got, True))


if __name__ == "__main__":
    unittest.main()
