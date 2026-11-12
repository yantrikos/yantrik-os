#!/usr/bin/env python3
"""The person's Browser in the sealed workshop's wall hands its DevTools pipe over (#1124, gate 5).

    python3 tests/browser-core/test_pipe_wall.py

The wall starts the Browser as `bwrap ... --unshare-pid -- chromium`. Seen on a real bubblewrap,
the tree is: outer bwrap (the shell's child; its fd 3 is its own signalfd) -> bwrap reaper (pid 1
of the new pid namespace) -> the browser, which alone holds fds 3 and 4. The proof is the same
three facts as before, applied to the process that really holds the stream: the caller is the
installed shell, that process is in the shell's own tree through nothing but the system's bwrap,
and the kernel says the stream sent is its fd 3. /proc is a made-up tree and the sockets are real.
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

from yantrik_browser import handover  # noqa: E402

FLAG = "--remote-debugging-pipe"
SHELL = 500


class TheWallsTree(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="wall-")
        self.addCleanup(lambda: shutil.rmtree(self.tmp, True))
        # A stand-in for /usr/bin/bwrap that the checks accept: ours, and nobody else's to write.
        self.bwrap = os.path.join(self.tmp, "bwrap-real")
        self.copy = os.path.join(self.tmp, "bwrap-copy")
        for path in (self.bwrap, self.copy):
            open(path, "w").close()
            os.chmod(path, 0o755)
        os.chmod(self.copy, 0o775)
        for patch in (mock.patch.object(handover, "SYSTEM_BWRAP", (self.bwrap,)),
                      mock.patch.object(handover, "BWRAP_OWNER", os.getuid())):
            patch.start()
            self.addCleanup(patch.stop)
        self.proc = os.path.join(self.tmp, "proc")

    def at(self, pid, parent, exe=None, cmdline=("x",), fd3=None):
        d = os.path.join(self.proc, str(pid))
        shutil.rmtree(d, True)
        os.makedirs(os.path.join(d, "fd"), exist_ok=True)
        with open(os.path.join(d, "stat"), "w") as f:
            f.write("%d (bwrap x) S %d 1 1 0\n" % (pid, parent))
        with open(os.path.join(d, "cmdline"), "wb") as f:
            f.write(b"".join(a.encode() + b"\0" for a in cmdline))
        if exe:
            os.symlink(exe, os.path.join(d, "exe"))
        if fd3:
            os.symlink(fd3, os.path.join(d, "fd", "3"))

    def wall(self, program=600, **program_kw):
        """The real tree: shell 500 -> outer bwrap 598 -> reaper 599 -> browser 600."""
        self.at(598, SHELL, exe=self.bwrap, fd3="anon_inode:[signalfd]")
        self.at(599, 598, exe=self.bwrap)
        kw = dict(cmdline=["chromium", FLAG], fd3=None)
        kw.update(program_kw)
        self.at(program, 599, **kw)

    def check(self, pid, caller=SHELL):
        return handover.browser_is_the_callers(pid, caller, self.proc)

    def test_a_browser_in_the_wall_is_the_callers(self):
        self.wall()
        self.assertIsNone(self.check(600))

    def test_the_stream_proof_applies_to_the_process_that_holds_fd_3(self):
        sent, browsers_end = socket.socketpair()
        ino = os.fstat(browsers_end.fileno()).st_ino
        self.wall(fd3="socket:[%d]" % ino)
        self.assertIsNone(handover.stream_is_the_browsers(sent, 600, self.proc))
        # The outer bwrap holds a signalfd on fd 3, not the stream: the 520 failure.
        self.assertIn("not the one pid 598", handover.stream_is_the_browsers(sent, 598, self.proc))

    def test_the_plain_launch_is_still_the_shells_direct_child(self):
        self.at(700, SHELL, cmdline=["chromium", FLAG])
        self.assertIsNone(self.check(700))
        self.assertIn("not a browser the caller started", self.check(700, caller=501))

    def test_the_outer_bwrap_alone_is_not_the_browser(self):
        """The pid the shell spawns has no pipe flag: it is not offered as the browser."""
        self.wall()
        self.assertIn("DevTools pipe", self.check(598))

    def test_a_chain_through_a_program_that_is_not_the_system_bwrap_is_refused(self):
        self.at(598, SHELL, exe="/usr/bin/python3")
        self.at(599, 598, exe=self.bwrap)
        self.at(600, 599, cmdline=["chromium", FLAG])
        self.assertIn("not a browser the caller started", self.check(600))
        self.assertIn("not the system's bwrap", self.check(600))

    def test_a_copy_of_bwrap_the_account_could_write_is_refused(self):
        self.at(598, SHELL, exe=self.copy)
        self.at(599, 598, exe=self.bwrap)
        self.at(600, 599, cmdline=["chromium", FLAG])
        self.assertIn("not the system's bwrap", self.check(600))  # not on the allowed list
        self.at(599, 598, exe=self.copy)
        with mock.patch.object(handover, "SYSTEM_BWRAP", (self.copy,)):
            self.assertIn("not the installer's alone", self.check(600))

    def test_another_program_mounted_over_the_path_is_not_the_system_bwrap(self):
        """Inside its own mount namespace an account can bind any root-owned program over
        /usr/bin/bwrap: `exe` then reads the right name, but the file is another one."""
        self.wall()
        self.assertIsNone(self.check(600))
        real_stat = os.stat
        other = os.path.join(self.tmp, "python3")
        open(other, "w").close()
        os.chmod(other, 0o755)

        def stat(path, *a, **kw):
            # /proc/<pid>/exe of the middle bwrap follows to a different file than the installed one
            if str(path).endswith(os.path.join("599", "exe")):
                return real_stat(other)
            return real_stat(path, *a, **kw)

        with mock.patch.object(handover.os, "stat", stat):
            self.assertIn("not the file the system installed", self.check(600))

    def test_a_non_child_of_the_shell_is_refused_even_through_bwrap(self):
        self.at(598, 777, exe=self.bwrap)
        self.at(599, 598, exe=self.bwrap)
        self.at(600, 599, cmdline=["chromium", FLAG])
        self.assertIn("not a browser the caller started", self.check(600))

    def test_any_descendant_is_not_enough(self):
        """A browser the shell's wall started, then a helper below it: only bwrap hops count."""
        self.wall()
        self.at(601, 600, cmdline=["chromium", FLAG])      # a child of the browser
        self.assertIn("not a browser the caller started", self.check(601))
        self.at(602, 599, exe=self.bwrap)                  # a third bwrap level
        self.at(603, 602, exe=self.bwrap)
        self.at(604, 603, cmdline=["chromium", FLAG])
        self.assertIn("not a browser the caller started", self.check(604))

    def test_the_pipe_flag_on_the_wrong_process_is_refused(self):
        """The flag is on bwrap's own command line, as it is in real life; the program has none."""
        self.at(598, SHELL, exe=self.bwrap, cmdline=["bwrap", "--", "chromium", FLAG])
        self.at(599, 598, exe=self.bwrap)
        self.at(600, 599, cmdline=["chromium", "--no-first-run"])
        self.assertIn("DevTools pipe", self.check(600))
        sent, _theirs = socket.socketpair()
        self.assertIn("fd 3 could not be read", handover.stream_is_the_browsers(sent, 598, self.proc))

    def test_a_process_that_is_gone_is_not_provable(self):
        self.assertIn("could not be read", self.check(999))
        self.at(600, 4242, cmdline=["chromium", FLAG])     # a parent that left no /proc entry
        self.assertIn("not a browser the caller started", self.check(600))


if __name__ == "__main__":
    unittest.main()
