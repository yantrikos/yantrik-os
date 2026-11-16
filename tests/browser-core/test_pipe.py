#!/usr/bin/env python3
"""The person's Browser over its DevTools pipe (#1137), with no browser and no TCP.

    python3 tests/browser-core/test_pipe.py

Linux only (SCM_RIGHTS, /proc). The framing is real bytes on a real socket pair. The browser at
the other end is a small CDP speaker in this process (the same part the shell leaves as fds 3
and 4); the browser service's own driver, guard, sides and pipes.py are the real ones.
test_ports_live.py and test_live.py cover real Chromiums where one is installed.
"""

import contextlib
import json
import os
import shutil
import socket
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]

from yantrik_browser import cdp, driver as drv, guard, handover, listener, pipes, ports, transport  # noqa: E402

ME = os.getuid()


def pair():
    a, b = socket.socketpair()
    return a, b


class Framing(unittest.TestCase):
    def test_a_message_is_json_ended_by_one_nul(self):
        self.assertEqual(transport.PipeTransport.frame('{"id":1}'), b'{"id":1}\0')
        with self.assertRaises(ValueError):
            transport.PipeTransport.frame('{"a":"b\0c"}')

    def test_messages_split_and_joined_by_the_stream_come_out_whole(self):
        a, b = pair()
        t = transport.PipeTransport(b.fileno())
        a.sendall(b'{"id":1}\0{"id"')
        self.assertEqual(t.recv(), '{"id":1}')
        threading.Timer(0.2, lambda: a.sendall(b':2}\0')).start()
        self.assertEqual(t.recv(), '{"id":2}')
        a.sendall('{"t":"é"}'.encode() + b"\0")
        self.assertEqual(t.recv(), '{"t":"é"}', "UTF-8 split across reads stays whole")

    def test_what_is_sent_arrives_ended(self):
        a, b = pair()
        t = transport.PipeTransport(b.fileno())
        t.send('{"id":7,"method":"Target.getTargets"}')
        self.assertEqual(a.recv(100), b'{"id":7,"method":"Target.getTargets"}\0')

    def test_the_end_of_the_stream_and_a_close_end_a_wait(self):
        a, b = pair()
        t = transport.PipeTransport(os.dup(b.fileno()))
        a.close()
        with self.assertRaises(transport.Closed):
            t.recv()
        a, b = pair()
        t = transport.PipeTransport(os.dup(b.fileno()))
        threading.Timer(0.3, t.close).start()
        with self.assertRaises(transport.Closed):
            t.recv()
        t.close()                                   # twice is harmless: no other fd is closed

    def test_closing_one_connection_does_not_end_the_browsers_stream(self):
        a, b = pair()
        t = transport.PipeTransport(os.dup(b.fileno()))
        t.close()
        a.sendall(b"{}\0")                            # the browser's end is still open
        self.assertEqual(b.recv(10), b"{}\0")


class FakeChromium(threading.Thread):
    """The browser's end of the pipe: reads commands, answers a few, announces its tabs."""

    def __init__(self, sock):
        super().__init__(daemon=True)
        self.sock, self.buf, self.seen = sock, b"", []
        self.tabs = {"T1": {"targetId": "T1", "type": "page", "url": "https://home.example/", "title": "Home"}}

    def reply(self, msg):
        self.sock.sendall(json.dumps(msg).encode() + b"\0")

    def run(self):
        while True:
            try:
                chunk = self.sock.recv(65536)
            except OSError:
                return
            if not chunk:
                return
            self.buf += chunk
            while b"\0" in self.buf:
                raw, self.buf = self.buf.split(b"\0", 1)
                self.answer(json.loads(raw))

    def answer(self, msg):
        method, params, mid = msg["method"], msg.get("params", {}), msg["id"]
        self.seen.append(method)
        result = {}
        if method == "Target.getTargets":
            result = {"targetInfos": list(self.tabs.values())}
        elif method == "Target.createTarget":
            tid = "T%d" % (len(self.tabs) + 1)
            self.tabs[tid] = {"targetId": tid, "type": "page", "url": params["url"], "title": "New"}
            self.reply({"id": mid, "result": {"targetId": tid}})
            self.reply({"method": "Target.targetCreated", "params": {"targetInfo": self.tabs[tid]}})
            return
        elif method == "Target.attachToTarget":
            result = {"sessionId": "S-" + params["targetId"]}
        elif method == "Page.getFrameTree":
            result = {"frameTree": {"frame": {"id": "F1"}}}
        elif method == "Page.createIsolatedWorld":
            result = {"executionContextId": 7}
        elif method == "Runtime.evaluate":
            expression = params["expression"]
            if "__yb.snapshot" in expression:
                value = {"elements": [{"ref": "e1", "role": "link", "name": "Read more"}], "url": "https://home.example/",
                         "title": "Home", "headings": ["Welcome"]}
            elif "__yb.changes" in expression:
                value = {"changed": False}
            elif "__yb.where" in expression:
                value = {"url": "https://home.example/", "title": "Home"}
            else:
                value = True                      # the reader being installed
            result = {"result": {"value": value}}
        self.reply({"id": mid, "result": result})


def start_browser_for_the_person():
    """A PipeHolder holding one end of a socket pair with the fake browser on the other, and a driver
    for the person over it. Nothing is ever asked of TCP."""
    mine, theirs = socket.socketpair()
    browser = FakeChromium(theirs)
    browser.start()
    hub = pipes.Pipes()
    hub.give(ports.PERSON, mine, os.getpid())
    g = guard.Guard(identify=lambda port: (_ for _ in ()).throw(listener.Unidentified("nothing", nobody=True)),
                    uid=ME, side_of=lambda: ports.PERSON, pipes=hub)
    return browser, hub.holder(ports.PERSON), drv.Driver(judge=False, guard_=g)


class TheServiceDrivesThePersonsBrowserOverThePipe(unittest.TestCase):
    def setUp(self):
        patch = mock.patch.object(listener, "place_of_pid", lambda proc, pid: listener.PERSON)
        patch.start()
        self.addCleanup(patch.stop)
        # Any attempt at TCP is a failure.
        for target in ("urllib.request.urlopen", "socket.create_connection"):
            p = mock.patch(target, side_effect=AssertionError("TCP was used: " + target))
            p.start()
            self.addCleanup(p.stop)
        self.chromium, self.holder, self.driver = start_browser_for_the_person()

    def test_the_tabs_are_read_over_the_pipe(self):
        tabs = self.driver.tabs()
        self.assertEqual([t["url"] for t in tabs["tabs"]], ["https://home.example/"])

    def test_a_page_is_read_over_the_pipe(self):
        page = self.driver.read()
        self.assertEqual(page["url"], "https://home.example/")
        self.assertEqual(page["elements"][0]["name"], "Read more")
        self.assertIn("Page.enable", self.chromium.seen)

    def test_a_tab_is_opened_over_the_pipe(self):
        out = self.driver.go("https://news.example/", new_tab=True)
        self.assertIn("opened https://news.example/ in a new tab", str(out))
        self.assertEqual(self.chromium.tabs["T2"]["url"], "https://news.example/")

    def test_a_second_connection_reuses_the_same_stream_without_ending_it(self):
        self.driver.tabs()
        browser = self.driver.sides.browsers[ports.PERSON]
        browser.close()                                  # e.g. the guard dropping a connection
        self.assertTrue(self.chromium.is_alive(), "the browser's end of the stream stayed open")
        self.assertEqual(len(self.driver.tabs()["tabs"]), 1)

    def test_when_the_browser_exits_the_person_is_told_it_is_not_open(self):
        self.driver.tabs()
        self.chromium.sock.shutdown(socket.SHUT_RDWR)       # the browser exits: the stream ends
        time.sleep(0.5)
        self.assertFalse(self.driver.status()["open"])

    def test_no_browser_handed_over_is_not_open_and_not_tried_elsewhere(self):
        g = guard.Guard(identify=lambda port: (_ for _ in ()).throw(listener.Unidentified("nothing", nobody=True)),
                        uid=ME, side_of=lambda: ports.PERSON, pipes=pipes.Pipes())
        d = drv.Driver(judge=False, guard_=g)
        self.assertFalse(d.status()["open"])


def the_identity_checks_pass():
    """The three identity checks of handover.py stand aside, for tests of the rest of `take`."""
    stack = contextlib.ExitStack()
    for name in ("shell_is_installed", "browser_is_the_callers", "stream_is_the_browsers"):
        stack.enter_context(mock.patch.object(pipes.handover, name, lambda *a, **k: None))
    return stack


class TheHandOver(unittest.TestCase):
    """The shell gives the stream over browser-pipe.sock; only the installed shell may."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="pipe-")
        self.addCleanup(lambda: shutil.rmtree(self.dir, True))
        patch = mock.patch.object(listener, "place_of_pid", lambda proc, pid: listener.PERSON)
        patch.start()
        self.addCleanup(patch.stop)
        self.hub = pipes.Pipes()
        self.holder = self.hub.holder(ports.PERSON)
        self.path = self.hub.serve(os.path.join(self.dir, "browser-pipe.sock"))
        self.addCleanup(self.hub.stop)

    def offer(self, pid=None, sock=None, side=None):
        """What the shell sends: a line naming the pid (and the side, when given) and the stream."""
        mine, theirs = socket.socketpair()
        conn = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        conn.connect(self.path)
        header = {"pid": pid or os.getpid(), **({"side": side} if side else {})}
        socket.send_fds(conn, [json.dumps(header).encode() + b"\n"], [(sock or theirs).fileno()])
        conn.settimeout(5)
        self.addCleanup(conn.close)
        return conn.recv(200).decode().strip(), mine

    def test_the_socket_is_the_persons_alone(self):
        self.assertEqual(os.stat(self.path).st_mode & 0o777, 0o600)

    def test_the_shell_hands_over_and_the_service_holds_it(self):
        with the_identity_checks_pass():
            reply, mine = self.offer()
        self.assertEqual(reply, "ok")
        self.assertIsNotNone(self.holder.current())

    def test_anything_but_the_shell_is_refused(self):
        reply, _ = self.offer()
        self.assertIn("only the desktop's shell", reply)
        self.assertIsNone(self.holder.current())

    def test_a_minds_browser_is_not_taken_as_the_persons(self):
        with the_identity_checks_pass(), \
                mock.patch.object(listener, "place_of_pid", lambda proc, pid: listener.MIND_VIEW):
            reply, _ = self.offer()
        self.assertIn("mind's browser", reply)
        self.assertIsNone(self.holder.current())

    def test_mind_views_browser_is_handed_over_by_the_same_path_and_held_apart(self):
        """#1200: one socket, one set of checks; the side the shell names, and the profile the
        process owns, put the pipe in Mind View's slot and not the person's."""
        with the_identity_checks_pass(), \
                mock.patch.object(listener, "place_of_pid", lambda proc, pid: listener.MIND_VIEW):
            reply, _ = self.offer(side=ports.MIND)
        self.assertEqual(reply, "ok")
        self.assertIsNotNone(self.hub.holder(ports.MIND).current())
        self.assertIsNone(self.holder.current(), "the person's slot is not Mind View's")

    def test_each_side_takes_only_the_browser_of_its_own_profile(self):
        """A person's browser offered as Mind View's, and Mind View's as the person's, are refused:
        the header is the shell's claim and the profile the process owns is the fact."""
        with the_identity_checks_pass():
            reply, _ = self.offer(side=ports.MIND)           # place_of_pid says PERSON (setUp)
            self.assertIn("not Mind View's", reply)
        with the_identity_checks_pass(), \
                mock.patch.object(listener, "place_of_pid", lambda proc, pid: listener.MIND_VIEW):
            reply, _ = self.offer(side=ports.PERSON)
            self.assertIn("mind's browser", reply)
        with the_identity_checks_pass():
            self.assertIn("no such side", self.offer(side="admin")[0])
        for side in (ports.PERSON, ports.MIND):
            self.assertIsNone(self.hub.holder(side).current())

    def test_a_mind_view_browser_not_yet_on_its_profile_is_told_to_retry_and_then_taken(self):
        """#1200 review, MEDIUM-2: a cold start has not written its SingletonLock or opened a LOCK
        when the first offer is checked. That is a "retry:", not a final refusal, and the same
        browser is taken once its profile shows."""
        places = iter([listener.PERSON, listener.MIND_VIEW])
        with the_identity_checks_pass(), mock.patch.object(listener, "place_of_pid", lambda proc, pid: next(places)):
            first, _ = self.offer(side=ports.MIND)
            self.assertIsNone(self.hub.holder(ports.MIND).current())
            second, _ = self.offer(side=ports.MIND)
        self.assertTrue(first.startswith("retry:"), first)
        self.assertEqual(second, "ok")
        self.assertIsNotNone(self.hub.holder(ports.MIND).current())

    def test_a_profile_that_cannot_be_read_is_answered_with_a_retry_and_never_silence(self):
        """`listener.Unidentified` from `place_of_pid` once killed the receiving thread with no
        reply, which the shell reads as a final answer. It is answered, by `take` and by the
        thread that serves it."""
        def unreadable(proc, pid):
            raise listener.Unidentified("the files process %d holds could not be read" % pid, transient=True)
        with the_identity_checks_pass(), mock.patch.object(listener, "place_of_pid", unreadable):
            reply, _ = self.offer(side=ports.MIND)
        self.assertTrue(reply.startswith("retry:") and "could not be read" in reply, reply)
        self.assertIsNone(self.hub.holder(ports.MIND).current())
        # And when it is raised past `take` (a check added later that reads /proc): still a reply.
        with mock.patch.object(pipes.Pipes, "_take", side_effect=listener.Unidentified("/proc went away")):
            reply, _ = self.offer(side=ports.MIND)
        self.assertEqual(reply, "retry: /proc went away")

    def test_mind_views_browser_goes_through_the_same_identity_checks(self):
        """Not a copy of them: the shell, the child, the pipe flag and the stream are asked for a
        Mind View offer exactly as for the person's, so the refusals are the same sentences."""
        with mock.patch.object(listener, "place_of_pid", lambda proc, pid: listener.MIND_VIEW):
            reply, _ = self.offer(side=ports.MIND)            # this test process is not the installed shell
        self.assertIn("only the desktop's shell", reply)
        self.assertIsNone(self.hub.holder(ports.MIND).current())
        seen = []
        real = pipes.handover.browser_is_the_callers
        with the_identity_checks_pass(), \
                mock.patch.object(pipes.handover, "browser_is_the_callers", lambda *a, **k: seen.append(a) or real(*a, **k)), \
                mock.patch.object(listener, "place_of_pid", lambda proc, pid: listener.MIND_VIEW):
            reply, _ = self.offer(side=ports.MIND)
        self.assertEqual(len(seen), 1)
        self.assertNotEqual(reply, "ok", "this process is not a child of the shell, started with the pipe")

    def test_a_live_browsers_pipe_is_not_taken_away_by_a_later_offer(self):
        with the_identity_checks_pass():
            self.assertEqual(self.offer()[0], "ok")
            self.assertIn("already held", self.offer()[0])

    def test_a_pid_that_is_gone_is_not_a_browser(self):
        holder = pipes.PipeHolder(start_of=lambda pid: None)
        holder.give(socket.socketpair()[0], 99999999)
        self.assertIsNone(holder.current())

    def test_the_hand_over_connection_is_a_lease_held_while_the_browser_is(self):
        """M3: the shell reads the connection closing as 'the service lost the pipe'."""
        alive = {"yes": True}
        holder = pipes.Pipes(start_of=lambda pid: 1 if alive["yes"] else 2)
        path = holder.serve(os.path.join(self.dir, "lease.sock"))
        self.addCleanup(holder.stop)
        mine, theirs = socket.socketpair()
        conn = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        conn.connect(path)
        self.addCleanup(conn.close)
        with the_identity_checks_pass(), mock.patch.object(pipes, "LEASE_POLL", 0.05):
            socket.send_fds(conn, [json.dumps({"pid": os.getpid()}).encode() + b"\n"], [theirs.fileno()])
            conn.settimeout(5)
            self.assertEqual(conn.recv(10), b"ok\n")
            conn.settimeout(0.5)
            with self.assertRaises(socket.timeout):
                conn.recv(1)            # still open: the service holds the stream
            alive["yes"] = False        # the browser is gone: the service lets go
            conn.settimeout(5)
            self.assertEqual(conn.recv(1), b"")


class TheHandOverChecksWhoIsGivingWhat(unittest.TestCase):
    """H1/M2 on the service side, against a /proc-shaped tree and real sockets."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="ident-")
        self.addCleanup(lambda: shutil.rmtree(self.tmp, True))

    def proc_of(self, pid, exe=None, cmdline=(), parent=1, fd3=None, mnt="mnt:[4026531841]"):
        at = os.path.join(self.tmp, str(pid))
        os.makedirs(os.path.join(at, "fd"), exist_ok=True)
        if exe:
            os.symlink(exe, os.path.join(at, "exe"))
        # The namespaces: the caller's, and this service's own under `self`, alike unless said.
        for who, target in ((str(pid), mnt), ("self", "mnt:[4026531841]")):
            ns = os.path.join(self.tmp, who, "ns")
            os.makedirs(ns, exist_ok=True)
            for name, link in (("mnt", target), ("user", "user:[4026531837]")):
                if not os.path.lexists(os.path.join(ns, name)):
                    os.symlink(link, os.path.join(ns, name))
        with open(os.path.join(at, "cmdline"), "wb") as f:
            f.write(b"".join(a.encode() + b"\0" for a in cmdline))
        with open(os.path.join(at, "stat"), "w") as f:
            f.write("%d (chromium x) S %d 1 1 0\n" % (pid, parent))
        if fd3:
            os.symlink(fd3, os.path.join(at, "fd", "3"))
        return self.tmp

    ROOT_OWNED = "/bin/sh"

    def test_the_installed_shell_is_the_shell(self):
        proc = self.proc_of(10, exe=self.ROOT_OWNED)
        self.assertIsNone(handover.shell_is_installed(10, installed=self.ROOT_OWNED, proc=proc))

    def replaced_shell(self, pid):
        """A tree where the installed shell's file was unlinked under the running one: `exe`
        reads `<installed> (deleted)`, and the file behind it is this account's (a test cannot
        make root's)."""
        installed = os.path.join(self.tmp, "opt", "yantrik", "bin", "yantrik-ui")
        os.makedirs(os.path.dirname(installed))
        gone = installed + " (deleted)"
        shutil.copy(os.path.realpath(sys.executable), gone)
        os.chmod(gone, 0o755)
        return installed, self.proc_of(pid, exe=gone)

    def test_a_shell_whose_file_was_replaced_by_an_update_is_still_the_shell(self):
        installed, proc = self.replaced_shell(11)
        self.assertIsNone(handover.shell_is_installed(11, installed=installed, proc=proc, owner=os.getuid()))

    def test_a_deleted_copy_the_caller_owns_is_not_the_shell(self):
        """The review's demonstration (N2): a copy of python3 at the installed path inside a private
        mount namespace, unlinked, reads `(deleted)` too. Its file is the caller's, not root's."""
        installed, proc = self.replaced_shell(12)
        self.assertIn("not the installer's alone", handover.shell_is_installed(12, installed=installed, proc=proc))

    def test_a_caller_in_another_mount_namespace_is_not_the_shell(self):
        proc = self.proc_of(13, exe=self.ROOT_OWNED, mnt="mnt:[4026532999]")
        self.assertIn("another mnt namespace", handover.shell_is_installed(13, installed=self.ROOT_OWNED, proc=proc))
        os.makedirs(os.path.join(self.tmp, "14"))
        self.assertIn("could not be identified", handover.shell_is_installed(14, installed=self.ROOT_OWNED, proc=self.tmp))

    def test_a_copy_of_python_named_yantrik_ui_is_not_the_shell(self):
        """The review's demonstration: the old rule compared the file name only."""
        from yantrik_surface import gate
        copy = os.path.join(self.tmp, "bin", "yantrik-ui")
        os.makedirs(os.path.dirname(copy))
        shutil.copy(os.path.realpath(sys.executable), copy)
        proc = self.proc_of(12, exe=copy)
        self.assertTrue(gate.is_shell_binary(copy), "the basename rule passes it")
        self.assertIn("not the installed shell", handover.shell_is_installed(12, proc=proc))

    def test_a_pid_is_the_callers_browser_only_with_the_pipe_no_port_and_the_caller_as_parent(self):
        flag = "--remote-debugging-pipe"
        proc = self.proc_of(20, cmdline=["chromium", flag], parent=500)
        self.assertIsNone(handover.browser_is_the_callers(20, 500, proc))
        self.assertIn("not a browser the caller started", handover.browser_is_the_callers(20, 501, proc))
        proc = self.proc_of(21, cmdline=["chromium"], parent=500)
        self.assertIn("DevTools pipe", handover.browser_is_the_callers(21, 500, proc))
        proc = self.proc_of(22, cmdline=["chromium", flag, "--remote-debugging-port=9222"], parent=500)
        self.assertIn("DevTools pipe", handover.browser_is_the_callers(22, 500, proc))
        self.assertIn("could not be read", handover.browser_is_the_callers(23, 500, proc))
        # LOW-4: a single dash is a switch to Chromium too, and so is any other debugging switch.
        for bad in ("-remote-debugging-port=9222", "--remote-debugging-address=0.0.0.0", "-remote-debugging-pipe"):
            proc = self.proc_of(24, cmdline=["chromium", flag, bad], parent=500)
            self.assertIn("DevTools pipe", handover.browser_is_the_callers(24, 500, proc), bad)
        proc = self.proc_of(25, cmdline=["chromium", flag, "--no-first-run", "about:blank"], parent=500)
        self.assertIsNone(handover.browser_is_the_callers(25, 500, proc))

    def retitled(self, pid, text, parent=500):
        """A /proc entry whose cmdline is one string, as Chromium 150 leaves it at 0.85 s
        (proofs/1151/handover-probe.txt: nargs 2, the flag only inside argv[0])."""
        proc = self.proc_of(pid, cmdline=["x"], parent=parent)
        with open(os.path.join(proc, str(pid), "cmdline"), "wb") as f:
            f.write(text.encode() + bytes([0]))
        return proc

    def test_a_chromium_that_retitled_itself_is_still_the_callers_browser(self):
        """The 520 failure: all 8 real offers were refused, because the flag was no longer an
        element of the cmdline. Fails on the exact-match check this replaces."""
        line = "/usr/lib/chromium/chromium --show-component-extension-options --no-first-run --remote-debugging-pipe"
        proc = self.retitled(30, line)
        self.assertIsNone(handover.browser_is_the_callers(30, 500, proc))
        self.assertIn("not a browser the caller started", handover.browser_is_the_callers(30, 501, proc))

    def test_a_retitled_process_without_the_pipe_or_with_a_port_is_still_refused(self):
        proc = self.retitled(31, "/usr/lib/chromium/chromium --no-first-run")
        self.assertIn("DevTools pipe", handover.browser_is_the_callers(31, 500, proc))
        proc = self.retitled(32, "/usr/lib/chromium/chromium --remote-debugging-pipe --remote-debugging-port=9222")
        self.assertIn("DevTools pipe", handover.browser_is_the_callers(32, 500, proc))
        proc = self.retitled(33, "/usr/lib/chromium/chromium --remote-debugging-pipeline")
        self.assertIn("DevTools pipe", handover.browser_is_the_callers(33, 500, proc), "a word, not a substring")

    def test_an_impostor_that_retitles_itself_still_fails_on_parent_and_stream(self):
        """Saying the flag proves nothing, and nothing relies on it alone: a process that is not the
        shell's child, or does not hold the stream the shell sent, is refused whatever it says."""
        proc = self.retitled(34, "chromium --remote-debugging-pipe", parent=777)
        self.assertIn("not a browser the caller started", handover.browser_is_the_callers(34, 500, proc))
        sent, _browsers_end = socket.socketpair()
        stranger, _also = socket.socketpair()
        proc = self.proc_of(35, cmdline=["x"], parent=500, fd3="socket:[%d]" % os.fstat(stranger.fileno()).st_ino)
        self.assertIn("not the one", handover.stream_is_the_browsers(sent, 35, proc))

    def test_the_stream_sent_is_the_browsers_only_if_the_kernel_says_its_other_end_is_fd_3(self):
        sent, browsers_end = socket.socketpair()
        stranger, _also = socket.socketpair()
        self.assertEqual(handover.peer_inode(sent), os.fstat(browsers_end.fileno()).st_ino)
        proc = self.proc_of(30, fd3="socket:[%d]" % os.fstat(browsers_end.fileno()).st_ino)
        self.assertIsNone(handover.stream_is_the_browsers(sent, 30, proc))
        self.assertIn("not the one pid 30", handover.stream_is_the_browsers(stranger, 30, proc))

    def test_what_the_browser_wrote_while_nobody_held_the_stream_is_dropped_on_give(self):
        holder = pipes.PipeHolder(start_of=lambda pid: 7)
        mine, browsers_end = socket.socketpair()
        browsers_end.sendall(b'{"id":1,"result":{"stale":true}}\0')
        self.assertTrue(holder.give(mine, 4242))
        mine.setblocking(False)
        with self.assertRaises(BlockingIOError):
            mine.recv(1)
        browsers_end.close()

    def test_take_refuses_a_copied_python_end_to_end(self):
        copy = os.path.join(self.tmp, "yantrik-ui")
        shutil.copy(os.path.realpath(sys.executable), copy)
        proc = self.proc_of(os.getpid(), exe=copy)
        hub = pipes.Pipes(proc=proc)
        a, _b = socket.socketpair()
        peer = mock.Mock(pid=os.getpid(), uid=os.getuid())
        self.assertIn("only the desktop's shell", hub.take(a, peer=peer))
        for side in (ports.PERSON, ports.MIND):
            self.assertIsNone(hub.holder(side).current())


class NothingListensOnThePersonsOldPort(unittest.TestCase):
    def test_the_service_opens_no_tcp_listener(self):
        """The service's holder and driver for the person make no TCP socket at all."""
        made = []
        real = socket.socket

        def spy(family=-1, type=-1, *a, **kw):
            s = real(family, type, *a, **kw)
            made.append(s.family)
            return s
        with mock.patch.object(socket, "socket", spy):
            hub = pipes.Pipes()
            d = drv.Driver(judge=False, guard_=guard.Guard(identify=lambda p: (_ for _ in ()).throw(
                listener.Unidentified("nothing", nobody=True)), uid=ME, side_of=lambda: ports.PERSON, pipes=hub))
            d.status()
        self.assertNotIn(socket.AF_INET, made)
        self.assertNotIn(socket.AF_INET6, made)

    def test_the_persons_launch_gives_no_port_and_nothing_else_spells_9222_for_listening(self):
        flags = open(os.path.join(ROOT, "crates", "yantrik-browser-ports", "src", "lib.rs"), encoding="utf-8").read()
        self.assertIn('Side::Person => vec![PIPE_FLAG.into()]', flags)

    def test_nothing_in_the_deploy_tools_reaches_the_persons_browser_by_port(self):
        """M4: release-check, yos and its web helpers drive the Browser through its service; no port is spelled."""
        for tool in ("release-check", "yos", "yos_web.py", "yos_mcp_web.py"):
            path = os.path.join(ROOT, "deploy", "yantrik-os", tool)
            with open(path, encoding="utf-8") as f:
                source = f.read()
            compile(source, path, "exec")
            for spelled in ("9222", "yos.CDP", "/json/version", "/json/new", "/json/list", "person_port"):
                self.assertNotIn(spelled, source, "%s still reaches for %s" % (tool, spelled))

    def test_a_mind_is_sent_to_mind_views_pipe_whoever_is_open(self):
        """#1200: Mind View's browser is held over a pipe like the person's: no address, no TCP."""
        for persons_open in (True, False):
            hub = pipes.Pipes()
            if persons_open:
                hub.give(ports.PERSON, socket.socketpair()[0], os.getpid())
            g = guard.Guard(identify=lambda p: None, uid=ME, side_of=lambda: ports.MIND, pipes=hub)
            d = drv.Driver(judge=False, guard_=g)
            for side in (ports.MIND, ports.PERSON):
                self.assertIsNone(d.sides.browsers[side].address)
                self.assertIsNotNone(d.sides.browsers[side].opener)
            self.assertTrue(d.sides.browsers[ports.MIND].held_at_start, "every page is held until fenced")
            self.assertFalse(d.sides.browsers[ports.PERSON].held_at_start)

    def test_a_browser_cannot_be_given_an_address_by_the_service_or_by_mistake(self):
        from yantrik_browser import cdp
        with self.assertRaises(ValueError):
            cdp.Browser()                                    # no default port to fall back to
        with self.assertRaises(ValueError):
            cdp.Browser("http://127.0.0.1:1", opener=lambda: None)

    def test_the_service_with_mind_view_up_opens_no_tcp_socket_either(self):
        made = []
        real = socket.socket

        def spy(family=-1, type=-1, *a, **kw):
            s = real(family, type, *a, **kw)
            made.append(s.family)
            return s
        hub = pipes.Pipes()
        hub.give(ports.MIND, socket.socketpair()[0], os.getpid())
        with mock.patch.object(socket, "socket", spy), \
                mock.patch.object(listener, "place_of_pid", lambda proc, pid: listener.MIND_VIEW):
            d = drv.Driver(judge=False, guard_=guard.Guard(identify=lambda p: (_ for _ in ()).throw(
                listener.Unidentified("nothing", nobody=True)), uid=ME, side_of=lambda: ports.MIND, pipes=hub))
            d.status()
        self.assertNotIn(socket.AF_INET, made)
        self.assertNotIn(socket.AF_INET6, made)


if __name__ == "__main__":
    unittest.main()
