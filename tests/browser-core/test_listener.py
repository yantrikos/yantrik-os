#!/usr/bin/env python3
"""Which browser holds a port, read from a /proc that is a fake (#1085).

    python3 tests/browser-core/test_listener.py

Linux only. A browser is a mind's by the profile it OWNS (the pid its SingletonLock names, or a LOCK
file it holds open), never by its command line. The fixtures here give the command line every shape
a real Chromium's has had, including the one that got Mind View's own browser refused on 520 (one
space-joined string with a trailing NUL), and a process that rewrites its argv to claim a profile it
does not own. The same reading is held against real Chromiums in test_ports_live.py.
"""

import os
import shutil
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]

from yantrik_browser import listener, ports, stop_mind_view  # noqa: E402


class FakeProc:
    """A /proc, and the data home whose profiles the processes in it own or do not."""

    def __init__(self):
        self.root = tempfile.mkdtemp(prefix="yb-fakeproc-")
        self.env = {"HOME": self.root}
        self.proc = os.path.join(self.root, "proc")
        os.makedirs(os.path.join(self.proc, "net"))
        self.mind_view = ports.mind_view_profile(self.env)
        self.identities = ports.identities_root(self.env)
        os.makedirs(self.mind_view)
        os.makedirs(self.identities)
        self.rows = []
        self.write_table()

    def write_table(self):
        lines = ["  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode"]
        for i, (local, uid, inode) in enumerate(self.rows):
            lines.append("   %d: %s 00000000:0000 0A 00000000:00000000 00:00000000 00000000  %d        0 %d 1 0"
                         % (i, local, uid, inode))
        with open(os.path.join(self.proc, "net", "tcp"), "w") as f:
            f.write("\n".join(lines) + "\n")

    def listen(self, port, uid, inode, local="0100007F"):
        self.rows.append(("%s:%04X" % (local, port), uid, inode))
        self.write_table()

    def process(self, pid, cmdline, inode=None, environ=None):
        d = os.path.join(self.proc, str(pid))
        os.makedirs(os.path.join(d, "fd"), exist_ok=True)
        if inode is not None:
            link = os.path.join(d, "fd", "7")
            if os.path.islink(link):
                os.unlink(link)
            os.symlink("socket:[%d]" % inode, link)
        with open(os.path.join(d, "cmdline"), "wb") as f:
            f.write(cmdline)
        if environ is not None:
            with open(os.path.join(d, "environ"), "wb") as f:
                f.write(("\0".join("%s=%s" % kv for kv in environ.items()) + "\0").encode())

    def takes(self, profile, pid):
        """`pid` takes `profile` as Chromium does: a SingletonLock link naming it."""
        lock = os.path.join(profile, "SingletonLock")
        if os.path.islink(lock):
            os.unlink(lock)
        os.symlink("host-%d" % pid, lock)

    def holds_lock(self, pid, profile):
        """`pid` holds a LOCK file under `profile` open."""
        f = os.path.join(profile, "Local Storage", "LOCK")
        os.makedirs(os.path.dirname(f), exist_ok=True)
        open(f, "w").close()
        os.makedirs(os.path.join(self.proc, str(pid), "fd"), exist_ok=True)
        os.symlink(f, os.path.join(self.proc, str(pid), "fd", "9"))

    def close(self):
        shutil.rmtree(self.root, ignore_errors=True)


def shapes(profile):
    """Every shape a real Chromium's /proc/<pid>/cmdline has been seen in. The third is what a
    packaged Chromium showed on 520: ONE space-joined string, and a trailing NUL."""
    words = ["/usr/lib/chromium/chromium", "--remote-debugging-port=9223", "--user-data-dir=" + profile, "about:blank"]
    return [("\0".join(words) + "\0").encode(), " ".join(words).encode(), (" ".join(words) + "\0").encode()]


class Identify(unittest.TestCase):
    def setUp(self):
        self.p = FakeProc()
        self.addCleanup(self.p.close)

    def who(self, port):
        return listener.identify(port, self.p.proc, self.p.env)

    def test_mind_views_browser_is_known_by_what_it_owns_whatever_shape_its_command_line_has(self):
        for n, line in enumerate(shapes(self.p.mind_view)):
            p = FakeProc()
            self.addCleanup(p.close)
            p.listen(9223, 1000, 11)
            p.process(41, line, inode=11)
            p.takes(p.mind_view, 41)
            held = listener.identify(9223, p.proc, p.env)
            self.assertEqual((held.place, held.uid, held.pids), (listener.MIND_VIEW, 1000, (41,)), "shape %d" % n)

    def test_a_browser_whose_command_line_says_nothing_is_still_told_by_its_files(self):
        self.p.listen(9223, 1000, 5)
        self.p.process(9, b"chromium\0", inode=5)
        self.p.holds_lock(9, self.p.mind_view)
        self.assertEqual(self.who(9223).place, listener.MIND_VIEW)

    def test_arguments_prove_nothing_either_way(self):
        """A process that rewrites its argv to claim Mind View's profile owns nothing, so it is not
        a mind's; the browser that owns it is, whatever its argv says."""
        self.p.listen(9222, 1000, 77)
        for line in shapes(self.p.mind_view):
            self.p.process(9, line, inode=77)
            self.assertEqual(self.who(9222).place, listener.PERSON)
        owner = FakeProc()
        self.addCleanup(owner.close)
        owner.listen(9223, 1000, 5)
        owner.process(9, b"chromium --user-data-dir=/home/p/.config/chromium", inode=5)
        owner.takes(owner.mind_view, 9)
        self.assertEqual(listener.identify(9223, owner.proc, owner.env).place, listener.MIND_VIEW)

    def test_a_real_process_whose_argv_claims_mind_views_profile(self):
        """A live process (`exec -a`) whose real /proc/<pid>/cmdline names Mind View's profile:
        the person's while it owns nothing, Mind View's once the profile's lock names it."""
        import subprocess
        import time
        claim = "chromium --user-data-dir=" + self.p.mind_view
        proc = subprocess.Popen(["bash", "-c", 'exec -a "$0" sleep 30', claim])
        self.addCleanup(proc.kill)
        time.sleep(0.5)
        with open("/proc/%d/cmdline" % proc.pid, "rb") as f:
            self.assertIn(self.p.mind_view.encode(), f.read(), "the claim is really in its argv")
        self.assertEqual(listener.place_of_pid("/proc", proc.pid, self.p.env), listener.PERSON)
        self.p.takes(self.p.mind_view, proc.pid)
        self.assertEqual(listener.place_of_pid("/proc", proc.pid, self.p.env), listener.MIND_VIEW)

    def test_a_stale_lock_on_a_process_that_is_not_the_holder_changes_nothing(self):
        self.p.listen(9222, 1000, 5)
        self.p.process(9, b"chromium", inode=5)
        self.p.takes(self.p.mind_view, 4242)          # some other pid, not the holder
        self.assertEqual(self.who(9222).place, listener.PERSON)

    def test_a_companion_identity_is_a_minds_too(self):
        work = os.path.join(self.p.identities, "work")
        os.makedirs(work)
        self.p.listen(9230, 1000, 3)
        self.p.process(3, b"chromium", inode=3)
        self.p.takes(work, 3)
        self.assertEqual(self.who(9230).place, listener.IDENTITY)

    def test_nobody_two_sockets_and_unseen_holders_are_refusals(self):
        with self.assertRaises(listener.Unidentified) as nobody:
            self.who(9999)
        self.assertTrue(nobody.exception.nobody)
        self.p.listen(9223, 1000, 1)
        with self.assertRaises(listener.Unidentified) as unseen:      # listening, holder not visible
            self.who(9223)
        self.assertFalse(unseen.exception.nobody)

    def test_two_sockets_on_one_port_are_refused_even_when_both_own_mind_views_profile(self):
        self.p.listen(9223, 1000, 1)
        self.p.listen(9223, 1000, 2)
        for pid, inode in ((1, 1), (2, 2)):
            self.p.process(pid, b"chromium", inode=inode)
        self.p.takes(self.p.mind_view, 1)
        with self.assertRaises(listener.Unidentified) as two:
            self.who(9223)
        self.assertIn("two different sockets", str(two.exception))

    def test_a_browser_on_ipv6_loopback_alone_is_seen_as_that(self):
        table = os.path.join(self.p.proc, "net", "tcp6")
        with open(table, "w") as f:
            f.write("  sl  local_address rem_address st\n   0: %s:2406 00000000000000000000000000000000:0000 0A 0:0 0:0 0 1000 0 9 1 0\n"
                    % listener.LOOPBACK6)
        self.assertTrue(listener.on_old_address(9222, self.p.proc))
        self.assertFalse(listener.on_old_address(9223, self.p.proc))
        self.assertFalse(listener.on_old_address(9222, os.path.join(self.p.root, "nowhere")))

    def test_the_exact_address_beats_a_wildcard_one_as_the_kernel_does(self):
        self.p.listen(9223, 1000, 1, local="00000000")
        self.p.listen(9223, 1000, 2)
        self.p.process(1, b"chromium", inode=1)
        self.p.process(2, b"chromium", inode=2)
        self.p.takes(self.p.mind_view, 2)
        self.assertEqual(self.who(9223).place, listener.MIND_VIEW)


class StopMindView(unittest.TestCase):
    """The update's step: Mind View's browsers stop, the person's never do."""

    def test_only_the_process_the_profile_names_is_chosen(self):
        p = FakeProc()
        self.addCleanup(p.close)
        for pid in (10, 11, 12, 20, 21):
            p.process(pid, b"chromium --user-data-dir=" + p.mind_view.encode())     # all CLAIM it
        p.takes(p.mind_view, 20)                       # only 20 owns it
        self.assertEqual(stop_mind_view.mind_view_mains(p.proc, homes=[p.root]), [20])

    def test_a_browser_whose_environment_was_overwritten_is_still_found(self):
        """520: a real Chromium rewrites its environ area, and one started with a small environment
        had no HOME left in it. Nothing here reads it."""
        p = FakeProc()
        self.addCleanup(p.close)
        p.process(30, b"chromium")
        with open(os.path.join(p.proc, "30", "environ"), "wb") as f:
            f.write(b"s --ozone-platform=wayland --no-first-run\0")
        p.takes(p.mind_view, 30)
        self.assertEqual(stop_mind_view.mind_view_mains(p.proc, homes=[p.root]), [30])

    def test_found_by_the_files_it_holds_when_the_home_is_not_one_it_knows(self):
        p = FakeProc()
        self.addCleanup(p.close)
        p.process(40, b"chromium")
        p.holds_lock(40, p.mind_view)
        p.takes(p.mind_view, 40)
        self.assertEqual(stop_mind_view.mind_view_mains(p.proc, homes=[]), [40])

    def test_the_persons_browser_is_never_found(self):
        p = FakeProc()
        self.addCleanup(p.close)
        person = os.path.join(p.root, ".config", "chromium")
        os.makedirs(person)
        p.process(50, b"chromium --user-data-dir=" + p.mind_view.encode())
        p.holds_lock(50, person)
        p.takes(person, 50)
        self.assertEqual(stop_mind_view.mind_view_mains(p.proc, homes=[p.root]), [])


if __name__ == "__main__":
    unittest.main()
