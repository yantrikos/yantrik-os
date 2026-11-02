#!/usr/bin/env python3
"""The port and profile choice against two real Chromiums and the real /proc (#1085).

    python3 tests/browser-core/test_ports_live.py

Needs google-chrome or chromium on PATH, python3-websocket and Linux. Skips (exit 0, says so)
without them. Real: both browsers, their sockets, /proc, the browser service's driver and its
checks. A stand-in: the two ports are free ones this test picks (9222 and 9223 may be a desktop's
own), set through ports.py as a test of the same code; and "who is calling" is a function, since
the mind door and the shell are not here (test_guard.py covers that half; live-proof-1085b.sh
covers it with real Mind and Hermes turns on a VM).

The person's Browser is started with no --user-data-dir at all (the default profile, as it is launched
before and after this change) and, in one test, with a profile named some other way: both must be
the person's. Mind View's is started with its profile. In every start order a mind lands in Mind View's
browser and the person in theirs.
"""

import functools
import http.server
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]


def chrome():
    for name in ("google-chrome", "chromium", "chromium-browser"):
        path = shutil.which(name)
        if path:
            return path
    return None


try:
    from yantrik_browser import cdp, driver as drv, guard, listener, ports  # noqa: E402
    import websocket  # noqa: F401,E402
    SUPPORTED = sys.platform.startswith("linux") and chrome() is not None
except ImportError:
    SUPPORTED = False


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Fixture:
    """A home, two ports, a site with a page per name, and the browsers started on demand."""

    def __init__(self):
        self.home = tempfile.mkdtemp(prefix="yb-live-1085-")
        self.env = {"HOME": self.home, "XDG_DATA_HOME": os.path.join(self.home, ".local", "share"),
                    "XDG_CONFIG_HOME": os.path.join(self.home, ".config")}
        self.saved = {k: os.environ.get(k) for k in self.env}
        os.environ.update(self.env)
        self.person_port, self.mind_port = free_port(), free_port()
        self.patched = (ports.PERSON_PORT, ports.MIND_VIEW_PORT)
        ports.PERSON_PORT, ports.MIND_VIEW_PORT = self.person_port, self.mind_port
        root = os.path.join(self.home, "site")
        os.makedirs(root)
        for name in ("person-a", "person-b", "mind-a", "mind-b"):
            with open(os.path.join(root, name + ".html"), "w") as f:
                f.write("<!doctype html><title>%s</title><p>%s</p>" % (name.upper(), name))
        handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=root)
        handler.log_message = lambda *a, **k: None
        self.http = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=self.http.serve_forever, daemon=True).start()
        self.site = "http://127.0.0.1:%d" % self.http.server_address[1]
        self.procs = {}

    def start(self, who, profile="default", port=None):
        """Start the person's or the mind's Chromium. `profile`: "default" (no flag: the person's
        Browser as it is launched, before and after this change), "other" (a profile named some
        other way), or "mind" (Mind View's)."""
        port = port or (self.person_port if who == "person" else self.mind_port)
        args = [chrome(), "--headless=new", "--no-first-run", "--no-default-browser-check", "--disable-gpu",
                "--no-sandbox", "--disable-background-networking", "--disable-component-update",
                "--remote-debugging-address=127.0.0.1", "--remote-debugging-port=%d" % port,
                "--remote-allow-origins=http://127.0.0.1:%d" % port]
        if profile == "other":
            args.append("--user-data-dir=" + os.path.join(self.home, ".config", "person-profile"))
        elif profile == "mind":
            args.append("--user-data-dir=" + ports.mind_view_profile())
        args.append("about:blank")
        env = dict(os.environ)
        log = open(os.path.join(self.home, "%s-%d.log" % (who, port)), "wb")
        proc = subprocess.Popen(args, stdout=log, stderr=log, env=env, stdin=subprocess.DEVNULL)
        self.procs[(who, port)] = proc
        deadline = time.monotonic() + 60
        last = None
        while time.monotonic() < deadline:
            try:
                listener.identify(port)
                return proc
            except listener.Unidentified as e:
                last = e
                time.sleep(0.2)
        log.flush()
        with open(log.name, "rb") as f:
            tail = f.read()[-600:].decode("utf-8", "replace")
        raise RuntimeError("the %s's browser did not listen on %d (%s; exit %s): %s" % (who, port, last, proc.poll(), tail))

    def stop(self, who):
        for (w, port), proc in list(self.procs.items()):
            if w == who:
                proc.kill()
                proc.wait()
                del self.procs[(w, port)]
        time.sleep(0.5)

    def titles(self, port):
        """The page titles of the browser on `port`, straight from its own /json."""
        import json
        import urllib.request
        with urllib.request.urlopen("http://127.0.0.1:%d/json" % port, timeout=5) as r:
            return sorted(t["title"] for t in json.load(r) if t["type"] == "page")

    def close(self):
        for proc in self.procs.values():
            proc.kill()
            proc.wait()
        self.http.shutdown()
        ports.PERSON_PORT, ports.MIND_VIEW_PORT = self.patched
        for k, v in self.saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        shutil.rmtree(self.home, ignore_errors=True)


@unittest.skipUnless(SUPPORTED, "needs Linux, google-chrome or chromium, and python3-websocket")
class RealBrowsers(unittest.TestCase):
    def setUp(self):
        self.fx = Fixture()
        self.addCleanup(self.fx.close)
        self.caller = {"side": ports.MIND}
        self.starts = []

        def start_mind_view():           # what the shell's open_mind_browser does, for this test
            self.starts.append(1)
            self.fx.start("mind", "mind")
        self.starter = start_mind_view
        self.driver = drv.Driver(judge=False, guard_=guard.Guard(
            side_of=lambda: self.caller["side"], starter=lambda: self.starter()))
        self.addCleanup(lambda: [b.close() for b in self.driver.sides.browsers.values()])

    def as_(self, side):
        self.caller["side"] = side
        return self.driver

    def go(self, side, page):
        out = self.as_(side).go("%s/%s.html" % (self.fx.site, page))
        self.assertTrue(out.get("navigated"), out)

    def both_land_where_they_belong(self):
        fx = self.fx
        self.go(ports.MIND, "mind-a")
        self.assertEqual(fx.titles(fx.mind_port), ["MIND-A"])
        self.assertNotIn("MIND-A", fx.titles(fx.person_port))
        self.go(ports.PERSON, "person-a")
        self.assertEqual(fx.titles(fx.person_port), ["PERSON-A"])
        self.assertEqual(fx.titles(fx.mind_port), ["MIND-A"], "the person's go moved the mind's browser")
        self.go(ports.MIND, "mind-b")
        self.go(ports.PERSON, "person-b")
        self.assertEqual((fx.titles(fx.mind_port), fx.titles(fx.person_port)), (["MIND-B"], ["PERSON-B"]))
        # Each sees only its own tabs, and never the other's.
        mind_tabs = self.as_(ports.MIND).tabs()["tabs"]
        person_tabs = self.as_(ports.PERSON).tabs()["tabs"]
        self.assertEqual([t["title"] for t in mind_tabs], ["MIND-B"])
        self.assertEqual([t["title"] for t in person_tabs], ["PERSON-B"])

    def test_the_persons_browser_first_on_the_old_flags_then_mind_views(self):
        self.fx.start("person", "default")
        held = listener.identify(self.fx.person_port)
        self.assertEqual(held.place, listener.PERSON)
        with open("/proc/%d/cmdline" % held.pids[0], "rb") as f:
            raw = f.read()
        # The 520 gate found Debian's Chromium reading back as one space-joined string; this Chrome
        # may or may not. Either way the profile is read: the point is the line above.
        print("\n  a real main process reads back %s: %r" % ("NUL separated" if b"\0" in raw.rstrip(b"\0") else "as one string", raw[:90]))
        self.fx.start("mind", "mind")
        self.assertEqual(listener.identify(self.fx.mind_port).place, listener.MIND_VIEW)
        self.both_land_where_they_belong()

    def test_mind_views_browser_first_then_the_person_with_a_profile_named_some_other_way(self):
        self.fx.start("mind", "mind")
        self.fx.start("person", "other")
        self.assertEqual(listener.identify(self.fx.person_port).place, listener.PERSON)
        self.both_land_where_they_belong()

    def test_the_persons_browser_closing_and_coming_back_changes_nothing_for_a_mind(self):
        self.fx.start("mind", "mind")
        self.go(ports.MIND, "mind-a")
        self.fx.start("person", "default")
        self.go(ports.PERSON, "person-a")
        self.fx.stop("person")
        self.go(ports.MIND, "mind-b")
        self.fx.start("person", "default")
        self.assertIn(self.fx.titles(self.fx.person_port), (["about:blank"], [""]))
        self.assertEqual(self.fx.titles(self.fx.mind_port), ["MIND-B"])

    def test_a_minds_first_web_call_starts_mind_views_browser_and_carries_on(self):
        """On a cold start, and with only the person's Browser open, the first call used to fail
        "the browser is not open"; it now starts Mind View's own browser and goes on."""
        self.fx.start("person", "default")
        self.go(ports.PERSON, "person-a")
        self.assertFalse(self.as_(ports.MIND).status()["open"], "a status look starts nothing")
        self.assertEqual(self.starts, [])
        self.go(ports.MIND, "mind-a")                      # the FIRST web call
        self.assertEqual(self.starts, [1])
        self.assertEqual(self.fx.titles(self.fx.mind_port), ["MIND-A"])
        self.assertEqual(self.fx.titles(self.fx.person_port), ["PERSON-A"], "the person's tab was touched")
        self.go(ports.MIND, "mind-b")
        self.assertEqual(self.starts, [1], "a browser already open is not started again")

    def test_when_it_cannot_be_started_the_mind_is_told_what_is_true(self):
        def cannot():
            raise guard.Starting("Mind View is not available (no Xwayland)")
        self.starter = cannot
        self.fx.start("person", "default")
        with self.assertRaises(drv.PageRefused) as refused:
            self.as_(ports.MIND).go(self.fx.site + "/mind-a.html")
        text = str(refused.exception)
        self.assertIn("could not be started: Mind View is not available", text)
        self.assertIn("not yours to use", text)
        self.assertNotIn("MIND-A", self.fx.titles(self.fx.person_port))

    def test_the_person_is_never_given_a_browser_they_did_not_open(self):
        with self.assertRaises(drv.Refused) as closed:
            self.as_(ports.PERSON).read()
        self.assertEqual(self.starts, [])
        self.assertIn("not open", str(closed.exception))

    def test_the_persons_browser_on_mind_views_port_is_never_used_by_a_mind(self):
        """The old order that put a mind in the person's browser: theirs holds the port."""
        self.fx.start("person", "default", port=self.fx.mind_port)
        for act in (lambda d: d.go(self.fx.site + "/mind-a.html"), lambda d: d.read(), lambda d: d.tabs(), lambda d: d.text()):
            with self.assertRaises(drv.PageRefused) as refused:
                act(self.as_(ports.MIND))
            self.assertIn("Nothing was done", str(refused.exception))
        self.assertIn(self.fx.titles(self.fx.mind_port), (["about:blank"], [""]), "a page was opened in the person's browser")
        self.assertTrue(all(not b.alive for b in self.driver.sides.browsers.values()))

    def test_a_mind_views_browser_left_on_the_persons_port_is_not_the_persons(self):
        self.fx.start("mind", "mind", port=self.fx.person_port)
        with self.assertRaises(drv.PageRefused) as refused:
            self.as_(ports.PERSON).read()
        self.assertIn("Mind View's own browser", str(refused.exception))

    def test_a_browser_that_takes_the_port_after_a_connection_is_not_followed(self):
        self.fx.start("mind", "mind")
        self.go(ports.MIND, "mind-a")
        self.fx.stop("mind")
        self.fx.start("person", "default", port=self.fx.mind_port)       # the person's browser takes the port
        with self.assertRaises(drv.PageRefused):
            self.as_(ports.MIND).go(self.fx.site + "/mind-b.html")
        self.assertNotIn("MIND-B", self.fx.titles(self.fx.mind_port))

    def test_the_update_step_stops_mind_views_browser_and_never_the_persons(self):
        """What yantrik-update runs on both sides of a restart (an update, and a rollback)."""
        from yantrik_browser import stop_mind_view
        person = self.fx.start("person", "default")
        other = self.fx.start("person", "other", port=free_port())
        legacy = self.fx.start("mind", "mind", port=free_port())
        chosen = set(stop_mind_view.mind_view_mains())
        self.assertTrue(chosen, "Mind View's browser was not found")
        self.assertNotIn(person.pid, chosen)
        self.assertNotIn(other.pid, chosen)
        self.assertEqual(stop_mind_view.main([]), 0)
        legacy.wait(timeout=30)
        self.assertIsNotNone(legacy.poll(), "Mind View's browser should have stopped")
        self.assertIsNone(person.poll(), "the person's Browser was stopped")
        self.assertIsNone(other.poll(), "a browser on the person's other profile was stopped")
        self.assertIn(self.fx.titles(self.fx.person_port), (["about:blank"], [""]))

    def test_the_first_call_after_a_browser_appears_does_not_time_out(self):
        """Issue #1085 saw one first web_go time out on a CDP reply right after a second Chromium
        appeared on the same port. With a port each there is no second Chromium on a port; the
        first call after each browser starts is timed here, and none comes near the 20 s limit."""
        for who, side in (("mind", ports.MIND), ("person", ports.PERSON)):
            self.fx.start(who, "mind" if who == "mind" else "default")
            began = time.monotonic()
            self.go(side, "%s-a" % who)
            self.assertLess(time.monotonic() - began, 10, who)


if __name__ == "__main__":
    unittest.main()
