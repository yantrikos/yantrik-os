#!/usr/bin/env python3
"""The choice of browser per caller against two real Chromiums and the real /proc (#1085, #1200).

    python3 tests/browser-core/test_ports_live.py

Needs google-chrome or chromium on PATH, python3-websocket and Linux. Skips (exit 0, says so)
without them. Real: both browsers, /proc, the browser service's driver and its checks. A stand-in:
"who is calling" is a function, since the mind door and the shell are not here (test_guard.py
covers that half; live-proof-1085b.sh covers it with real Mind and Hermes turns on a VM).

Both browsers are started with --remote-debugging-pipe and no port, as the shell starts them
(pipe_chrome.py), their streams given to the service's pipes.Pipes: the person's with no
--user-data-dir at all (the default profile) and, in one test, with a profile named some other way:
both must be the person's; Mind View's with its own profile. In every start order a mind lands in
Mind View's browser and the person in theirs, and nothing of either listens on TCP.
"""

import functools
import http.server
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [HERE, os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]


def chrome():
    for name in ("google-chrome", "chromium", "chromium-browser"):
        path = shutil.which(name)
        if path:
            return path
    return None


try:
    from yantrik_browser import driver as drv, guard, listener, pipes, ports  # noqa: E402
    import pipe_chrome  # noqa: E402
    import websocket  # noqa: F401,E402
    SUPPORTED = sys.platform.startswith("linux") and chrome() is not None
except ImportError:
    SUPPORTED = False

SIDE = {"person": "person", "mind": "mind"}   # the side a browser is given to (ports.PERSON, ports.MIND)


class Fixture:
    """A home, a site with a page per name, and the browsers started on demand."""

    def __init__(self):
        self.home = tempfile.mkdtemp(prefix="yb-live-1085-")
        self.env = {"HOME": self.home, "XDG_DATA_HOME": os.path.join(self.home, ".local", "share"),
                    "XDG_CONFIG_HOME": os.path.join(self.home, ".config")}
        self.saved = {k: os.environ.get(k) for k in self.env}
        os.environ.update(self.env)
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
        self.procs = []
        self.hub = pipes.Pipes()           # both browsers' streams, as the service holds them

    def start(self, who, profile="default", as_side=None, give=True):
        """Start the person's or the mind's Chromium on a DevTools pipe, given to the hub as
        `as_side` (the side `who` belongs to unless a test says otherwise). `profile`: "default" (no
        flag), "other" (a profile named some other way), or "mind" (Mind View's)."""
        args = ["--headless=new"]
        if profile == "other":
            args.append("--user-data-dir=" + os.path.join(self.home, ".config", "person-profile"))
        elif profile == "mind":
            args.append("--user-data-dir=" + ports.mind_view_profile())
        args.append("about:blank")
        log = open(os.path.join(self.home, "%s-%d.log" % (who, len(self.procs))), "wb")
        proc, mine = pipe_chrome.launch(chrome(), args, log, env=dict(os.environ))
        self.procs.append(proc)
        self.stream = mine                   # our end: kept open while the browser runs
        if give:
            self.hub.give(as_side or SIDE[who], mine, proc.pid)
        return proc

    def stop(self, who):
        """Stop what the hub holds for `who`'s side."""
        held = self.hub.holder(SIDE[who]).held
        for proc in self.procs:
            if held is not None and proc.pid == held.pid:
                proc.kill()
                proc.wait()
        time.sleep(0.5)

    def close(self):
        for proc in self.procs:
            proc.kill()
            proc.wait()
        self.http.shutdown()
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
            side_of=lambda: self.caller["side"], starter=lambda: self.starter(), pipes=self.fx.hub))
        self.addCleanup(lambda: [b.close() for b in self.driver.sides.browsers.values()])

    def as_(self, side):
        self.caller["side"] = side
        return self.driver

    def titles(self, side):
        """The page titles `side` sees: each browser has one stream, so they come through the
        service, as that side."""
        before = self.caller["side"]
        try:
            tabs = self.as_(side).tabs().get("tabs", [])
        finally:
            self.caller["side"] = before
        return sorted(t["title"] for t in tabs)

    def go(self, side, page):
        out = self.as_(side).go("%s/%s.html" % (self.fx.site, page))
        self.assertTrue(out.get("navigated"), out)

    def both_land_where_they_belong(self):
        self.go(ports.MIND, "mind-a")
        self.assertEqual(self.titles(ports.MIND), ["MIND-A"])
        self.assertNotIn("MIND-A", self.titles(ports.PERSON))
        self.go(ports.PERSON, "person-a")
        self.assertEqual(self.titles(ports.PERSON), ["PERSON-A"])
        self.assertEqual(self.titles(ports.MIND), ["MIND-A"], "the person's go moved the mind's browser")
        self.go(ports.MIND, "mind-b")
        self.go(ports.PERSON, "person-b")
        self.assertEqual((self.titles(ports.MIND), self.titles(ports.PERSON)), (["MIND-B"], ["PERSON-B"]))

    def test_the_persons_browser_first_on_its_pipe_then_mind_views(self):
        person = self.fx.start("person", "default")
        self.assertEqual(listener.place_of_pid("/proc", person.pid), listener.PERSON)
        with open("/proc/%d/cmdline" % person.pid, "rb") as f:
            raw = f.read()
        # The 520 gate found Debian's Chromium reading back as one space-joined string; this Chrome
        # may or may not. Either way the profile is read: the point is the line above.
        print("\n  a real main process reads back %s: %r" % ("NUL separated" if b"\0" in raw.rstrip(b"\0") else "as one string", raw[:90]))
        mind = self.fx.start("mind", "mind")
        self.assertEqual(listener.place_of_pid("/proc", mind.pid), listener.MIND_VIEW)
        self.both_land_where_they_belong()

    def test_mind_views_browser_first_then_the_person_with_a_profile_named_some_other_way(self):
        self.fx.start("mind", "mind")
        person = self.fx.start("person", "other")
        self.assertEqual(listener.place_of_pid("/proc", person.pid), listener.PERSON)
        self.both_land_where_they_belong()

    def test_no_devtools_listener_with_both_browsers_up_and_driven(self):
        """#1200: with Mind View's browser open (and the person's) and a page loaded in each, no
        process of either browser listens on any TCP port: raw DevTools has no address to speak to,
        from a job or from anything else. The old numbers are read too, in case something else of
        the machine's holds them."""
        person, mind = self.fx.start("person", "default"), self.fx.start("mind", "mind")
        self.go(ports.MIND, "mind-a")
        self.go(ports.PERSON, "person-a")
        for browser in (person, mind):
            tree = pipe_chrome.process_tree(browser.pid)
            self.assertGreater(len(tree), 1, "a browser has helper processes; they are looked at too")
            self.assertEqual(pipe_chrome.listening_sockets_of(tree), [], "pid %d" % browser.pid)
        for old in (ports.PERSON_PORT, int("92" + "23")):
            with self.assertRaises(listener.Unidentified, msg="something listens on %d" % old) as nothing:
                listener.identify(old)
            self.assertTrue(nothing.exception.nobody)
        # And what listens is read the same way for a browser that does have a port (the check can fail).
        stray = subprocess.Popen([sys.executable, "-c",
                                  "import socket,time;s=socket.socket();s.bind(('127.0.0.1',0));s.listen();time.sleep(30)"])
        self.addCleanup(stray.kill)
        time.sleep(0.5)
        self.assertEqual(len(pipe_chrome.listening_sockets_of([stray.pid])), 1)

    def test_the_hand_over_takes_mind_views_real_browser_from_its_parent_and_from_nobody_else(self):
        """#1200 against a real Chromium and the real checks of handover.py: the shell's identity is
        the one stand-in (there is no installed shell here). The offer of Mind View's browser by a
        process that did not start it is refused (a job cannot hand over, or take, the pipe); the
        offer by its parent, with the stream that really is the browser's fd 3, is taken."""
        import json
        import socket
        from unittest import mock
        path = os.path.join(self.fx.home, "browser-pipe.sock")
        self.fx.hub.serve(path)
        self.addCleanup(self.fx.hub.stop)
        mind = self.fx.start("mind", "mind", give=False)
        stream = self.fx.stream
        script = ("import json,socket,sys\n"
                  "c=socket.socket(socket.AF_UNIX);c.connect(sys.argv[1])\n"
                  "socket.send_fds(c,[json.dumps({'pid':int(sys.argv[2]),'side':'mind'}).encode()+b'\\n'],[int(sys.argv[3])])\n"
                  "print(c.recv(200).decode().strip())\n")
        with mock.patch.object(pipes.handover, "shell_is_installed", lambda *a, **k: None):
            job = subprocess.run([sys.executable, "-c", script, path, str(mind.pid), str(stream.fileno())],
                                 capture_output=True, text=True, timeout=30, pass_fds=[stream.fileno()])
            self.assertIn("not a browser the caller started", job.stdout, job.stdout + job.stderr)
            self.assertIsNone(self.fx.hub.holder(ports.MIND).current(), "a job's offer was taken")
            c = socket.socket(socket.AF_UNIX)
            c.connect(path)
            self.addCleanup(c.close)
            socket.send_fds(c, [json.dumps({"pid": mind.pid, "side": "mind"}).encode() + b"\n"], [stream.fileno()])
            c.settimeout(30)
            self.assertEqual(c.recv(200).decode().strip(), "ok")
        self.assertIsNotNone(self.fx.hub.holder(ports.MIND).current())
        self.go(ports.MIND, "mind-a")
        self.assertEqual(self.titles(ports.MIND), ["MIND-A"])

    def test_the_persons_browser_closing_and_coming_back_changes_nothing_for_a_mind(self):
        self.fx.start("mind", "mind")
        self.go(ports.MIND, "mind-a")
        self.fx.start("person", "default")
        self.go(ports.PERSON, "person-a")
        self.fx.stop("person")
        self.go(ports.MIND, "mind-b")
        self.fx.start("person", "default")
        self.assertIn(self.titles(ports.PERSON), (["about:blank"], [""]))
        self.assertEqual(self.titles(ports.MIND), ["MIND-B"])

    def test_a_minds_first_web_call_starts_mind_views_browser_and_carries_on(self):
        """On a cold start, and with only the person's Browser open, the first call used to fail
        "the browser is not open"; it now starts Mind View's own browser and goes on."""
        self.fx.start("person", "default")
        self.go(ports.PERSON, "person-a")
        self.assertFalse(self.as_(ports.MIND).status()["open"], "a status look starts nothing")
        self.assertEqual(self.starts, [])
        self.go(ports.MIND, "mind-a")                      # the FIRST web call
        self.assertEqual(self.starts, [1])
        self.assertEqual(self.titles(ports.MIND), ["MIND-A"])
        self.assertEqual(self.titles(ports.PERSON), ["PERSON-A"], "the person's tab was touched")
        self.go(ports.MIND, "mind-b")
        self.assertEqual(self.starts, [1], "a browser already open is not started again")

    def test_with_the_persons_browser_open_first_a_minds_first_call_may_be_tabs(self):
        """The 520 order: the person's Browser first; the mind's FIRST call is web_tabs."""
        self.fx.start("person", "default")
        self.go(ports.PERSON, "person-a")
        tabs = self.as_(ports.MIND).tabs()
        self.assertEqual(self.starts, [1])
        self.assertTrue(tabs["open"], tabs)
        self.assertNotIn("PERSON-A", [t["title"] for t in tabs["tabs"]], "the person's tabs were shown to a mind")
        self.go(ports.MIND, "mind-a")
        self.assertEqual(self.titles(ports.MIND), ["MIND-A"])
        self.assertEqual(self.titles(ports.PERSON), ["PERSON-A"])

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
        self.assertNotIn("MIND-A", self.titles(ports.PERSON))

    def test_the_person_is_never_given_a_browser_they_did_not_open(self):
        with self.assertRaises(drv.Refused) as closed:
            self.as_(ports.PERSON).read()
        self.assertEqual(self.starts, [])
        self.assertIn("not open", str(closed.exception))

    def test_the_persons_browser_handed_over_as_mind_views_is_never_used_by_a_mind(self):
        """The old order that put a mind in the person's browser, through the hand-over this time:
        what is held for Mind View must own Mind View's profile, whatever the header said."""
        self.fx.start("person", "default", as_side=ports.MIND)
        for act in (lambda d: d.go(self.fx.site + "/mind-a.html"), lambda d: d.read(), lambda d: d.tabs(), lambda d: d.text()):
            with self.assertRaises(drv.PageRefused) as refused:
                act(self.as_(ports.MIND))
            self.assertIn("Nothing was done", str(refused.exception))
        self.assertTrue(all(not b.alive for b in self.driver.sides.browsers.values()))

    def test_a_mind_views_browser_handed_over_as_the_persons_is_not_the_persons(self):
        self.fx.start("mind", "mind", as_side=ports.PERSON)
        with self.assertRaises(drv.PageRefused) as refused:
            self.as_(ports.PERSON).read()
        self.assertIn("Mind View's own", str(refused.exception))

    def test_a_browser_that_is_handed_over_as_mind_views_after_a_connection_is_not_followed(self):
        self.fx.start("mind", "mind")
        self.go(ports.MIND, "mind-a")
        self.fx.stop("mind")
        time.sleep(0.5)
        self.fx.start("person", "default", as_side=ports.MIND)           # the person's browser is offered as Mind View's
        with self.assertRaises(drv.PageRefused):
            self.as_(ports.MIND).go(self.fx.site + "/mind-b.html")
        self.assertNotIn("MIND-B", self.titles(ports.PERSON))

    def test_the_update_step_stops_mind_views_browser_and_never_the_persons(self):
        """What yantrik-update runs on both sides of a restart (an update, and a rollback), against a
        real Chromium's own SingletonLock and a real process start time."""
        from yantrik_browser import stop_mind_view
        person = self.fx.start("person", "default")
        other = self.fx.start("person", "other", give=False)
        legacy = self.fx.start("mind", "mind")
        profile = ports.mind_view_profile()
        os.chmod(profile, 0o775)             # 520: the desktop account's umask is 002
        chosen = stop_mind_view.mind_view_main(self.fx.home, os.getuid())
        self.assertIsNotNone(chosen, "a real Mind View browser on a 775 profile was not found")
        self.assertNotIn(chosen, (person.pid, other.pid))
        # A planted link naming the person's real Browser, or an unrelated process, is not believed:
        # neither holds a file of Mind View's profile (the real one is stopped first, its link stays).
        lock = os.path.join(profile, "SingletonLock")
        real_link = os.readlink(lock)
        sleeper = subprocess.Popen(["sleep", "60"])
        self.addCleanup(sleeper.kill)
        for victim in (person, sleeper):
            os.unlink(lock)
            os.symlink("%s-%d" % (real_link.rpartition("-")[0], victim.pid), lock)
            self.assertIsNone(stop_mind_view.mind_view_main(self.fx.home, os.getuid()), "a planted link was believed")
            self.assertFalse(stop_mind_view.stop(victim.pid, os.getuid(), profile))
        os.unlink(lock)
        os.symlink(real_link, lock)
        self.assertEqual(stop_mind_view.mind_view_main(self.fx.home, os.getuid()), chosen)
        self.assertTrue(stop_mind_view.stop(chosen, os.getuid(), profile))
        legacy.wait(timeout=30)
        self.assertIsNotNone(legacy.poll(), "Mind View's browser should have stopped")
        self.assertIsNone(person.poll(), "the person's Browser was stopped")
        self.assertIsNone(other.poll(), "a browser on the person's other profile was stopped")
        self.assertIsNone(sleeper.poll(), "an unrelated process was stopped")

    def test_the_first_call_after_a_browser_appears_does_not_time_out(self):
        """The first call after each browser starts is timed here, and none comes near the 20 s limit."""
        for who, side in (("mind", ports.MIND), ("person", ports.PERSON)):
            self.fx.start(who, "mind" if who == "mind" else "default")
            began = time.monotonic()
            self.go(side, "%s-a" % who)
            self.assertLess(time.monotonic() - began, 10, who)


if __name__ == "__main__":
    if not SUPPORTED:
        print("skipped: needs Linux, google-chrome or chromium, and python3-websocket")
    unittest.main()
