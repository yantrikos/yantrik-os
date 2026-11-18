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
    """The update's step runs as root: it signals only the desktop account's own Mind View browser,
    and nothing a planted file can name."""

    def setUp(self):
        self.me = os.getuid()
        self.home = tempfile.mkdtemp(prefix="yb-stop-home-")
        self.addCleanup(shutil.rmtree, self.home, True)
        self.profile = os.path.join(self.home, ".local", "share", *ports.MIND_VIEW_PROFILE.split("/"))
        os.makedirs(self.profile, mode=0o755)
        os.chmod(self.profile, 0o755)
        self.procs = []
        self.addCleanup(lambda: [p.kill() for p in self.procs])

    def browser(self, holding=True):
        """A process standing in for the browser, started BEFORE its lock is written, as Chromium's is,
        holding a file of the profile open as a real one does (or not: the person's Browser holds none)."""
        import subprocess
        import time
        script = 'exec 3>>"$0/Local State"; exec sleep 60' if holding else "exec sleep 60"
        p = subprocess.Popen(["bash", "-c", script, self.profile])
        self.procs.append(p)
        time.sleep(0.2)
        return p

    def lock(self, pid, host=None, profile=None, age=0):
        import socket
        link = os.path.join(profile or self.profile, "SingletonLock")
        if os.path.islink(link):
            os.unlink(link)
        os.symlink("%s-%d" % (host or socket.gethostname(), pid), link)
        if age:
            import time
            then = time.time() - age
            os.utime(link, (then, then), follow_symlinks=False)

    def found(self, **kw):
        return stop_mind_view.mind_view_main(self.home, kw.pop("uid", self.me), **kw)

    def test_the_account_own_running_browser_is_found_and_stopped_through_a_pidfd(self):
        p = self.browser()
        self.lock(p.pid)
        self.assertEqual(self.found(), p.pid)
        self.assertTrue(stop_mind_view.stop(p.pid, self.me, self.profile))
        self.assertEqual(p.wait(timeout=10), -15, "SIGTERM")

    def test_a_link_planted_in_another_accounts_home_is_never_looked_at(self):
        """The PoC: an account that can write a link names a root-owned victim. Only the desktop
        account's own profile is read, and only when it and its link are that account's."""
        victim = self.browser()
        self.lock(victim.pid)
        self.assertIsNone(self.found(uid=self.me + 1), "files owned by another account than the desktop's")
        self.assertIsNone(stop_mind_view.mind_view_main(os.path.join(self.home, "elsewhere"), self.me))
        os.chmod(self.profile, 0o757)
        self.assertIsNone(self.found(), "a directory others can write")
        os.chmod(self.profile, 0o775)
        self.assertIsNone(self.found(group_ok=lambda uid, gid: False), "group-writable, and the group is shared")
        os.chmod(self.profile, 0o755)
        self.assertEqual(self.found(), victim.pid, "and the same files are fine when they are the account's")

    def test_a_directory_the_accounts_own_private_group_can_write_is_accepted(self):
        """520: the desktop account's umask is 002, so the shell makes the profile 775."""
        p = self.browser()
        self.lock(p.pid)
        os.chmod(self.profile, 0o775)
        seen = []
        self.assertEqual(self.found(group_ok=lambda uid, gid: seen.append((uid, gid)) or True), p.pid)
        self.assertEqual(seen, [(self.me, os.stat(self.profile).st_gid)])

    def test_what_a_private_group_is(self):
        import grp
        import pwd
        from unittest import mock
        me = pwd.struct_passwd(("alice", "x", 1000, 1000, "", "/home/alice", "/bin/sh"))
        other = pwd.struct_passwd(("bob", "x", 1001, 1000, "", "/home/bob", "/bin/sh"))
        mine = grp.struct_group(("alice", "x", 1000, []))
        shared = grp.struct_group(("alice", "x", 1000, ["alice", "carol"]))
        with mock.patch.object(pwd, "getpwuid", return_value=me), mock.patch.object(pwd, "getpwall", return_value=[me]):
            with mock.patch.object(grp, "getgrgid", return_value=mine):
                self.assertTrue(stop_mind_view.private_group(1000, 1000))
                self.assertFalse(stop_mind_view.private_group(1000, 50), "not its primary group")
            with mock.patch.object(grp, "getgrgid", return_value=shared):
                self.assertFalse(stop_mind_view.private_group(1000, 1000), "another member")
        with mock.patch.object(pwd, "getpwuid", return_value=me), mock.patch.object(pwd, "getpwall", return_value=[me, other]):
            with mock.patch.object(grp, "getgrgid", return_value=mine):
                self.assertFalse(stop_mind_view.private_group(1000, 1000), "another account's primary group too")

    def test_a_process_of_another_account_is_never_chosen(self):
        self.lock(1)                        # init: root's, and nothing a planted link may name
        self.assertIsNone(self.found())
        self.assertFalse(stop_mind_view.stop(1, self.me, self.profile), "and it is refused again at the signal")

    def test_a_stale_link_naming_a_pid_reused_since_is_not_believed(self):
        p = self.browser()
        self.lock(p.pid, age=3600)          # the lock was written an hour before this process began
        self.assertIsNone(self.found())

    def test_a_planted_link_naming_the_accounts_own_process_that_holds_nothing_of_the_profile(self):
        """520: the desktop account itself can plant the link. Its Browser, or any process of its own,
        holds no file of Mind View's profile, so it is not chosen and not signalled."""
        person = self.browser(holding=False)
        self.lock(person.pid)
        self.assertIsNone(self.found())
        self.assertFalse(stop_mind_view.stop(person.pid, self.me, self.profile), "refused again at the signal")
        self.assertIsNone(person.poll(), "the process was signalled")
        # A process holding a file somewhere else is no more the profile's.
        elsewhere = tempfile.mkdtemp(prefix="yb-else-")
        self.addCleanup(shutil.rmtree, elsewhere, True)
        import subprocess
        import time
        other = subprocess.Popen(["bash", "-c", 'exec 3>>"$0/f"; exec sleep 60', elsewhere])
        self.procs.append(other)
        time.sleep(0.2)
        self.lock(other.pid)
        self.assertIsNone(self.found())
        self.assertIsNone(other.poll())

    def test_another_machines_lock_and_this_process_itself_are_not_believed(self):
        p = self.browser()
        self.lock(p.pid, host="some-other-host")
        self.assertIsNone(self.found())
        self.lock(os.getpid())
        self.assertIsNone(self.found())
        self.lock(p.pid)
        self.assertEqual(self.found(), p.pid)

    def test_nothing_else_is_scanned_a_planted_profile_elsewhere_is_ignored(self):
        other = tempfile.mkdtemp(prefix="yb-other-home-")
        self.addCleanup(shutil.rmtree, other, True)
        planted = os.path.join(other, ".local", "share", *ports.MIND_VIEW_PROFILE.split("/"))
        os.makedirs(planted)
        victim = self.browser()
        self.lock(victim.pid, profile=planted)
        self.assertIsNone(self.found(), "this home has no profile of its own")

    def test_a_link_to_the_profile_through_a_link_is_not_followed(self):
        real = tempfile.mkdtemp(prefix="yb-real-")
        self.addCleanup(shutil.rmtree, real, True)
        shutil.rmtree(self.profile)
        os.symlink(real, self.profile)
        victim = self.browser()
        self.lock(victim.pid, profile=real)
        self.assertIsNone(self.found())

    def test_main_needs_the_account_and_names_what_it_would_stop(self):
        import contextlib
        import io
        import pwd
        self.assertEqual(stop_mind_view.main([]), 2)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(stop_mind_view.main(["--user", pwd.getpwuid(self.me).pw_name, "--list"]), 0)
        if out.getvalue():
            # On a developer's own Yantrik machine this account really has one; that is the
            # tool working, not a fault, and the lines above have already checked its answer.
            self.skipTest("a real Mind View browser is running for this account: %s" % out.getvalue().strip())


if __name__ == "__main__":
    unittest.main()
