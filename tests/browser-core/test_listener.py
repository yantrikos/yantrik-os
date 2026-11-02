#!/usr/bin/env python3
"""Which browser holds a port, read from a /proc that is a fake (#1085).

    python3 tests/browser-core/test_listener.py

Linux only. The fake /proc here is what the real one looks like, including the thing a NUL-split
parser missed on 520: Chromium's main process reads back as ONE space-joined string. The same
reading is held against a real Chromium in test_ports_live.py.
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

ENV = {"HOME": "/home/p"}
MIND_VIEW = ports.mind_view_profile(ENV)
PERSON = "/home/p/.config/chromium"


def nul(*args):
    return ("\0".join(args) + "\0").encode()


def joined(*args):
    return " ".join(args).encode()


class FakeProc:
    def __init__(self):
        self.root = tempfile.mkdtemp(prefix="yb-fakeproc-")
        os.makedirs(os.path.join(self.root, "net"))
        self.rows = []
        self.write_table()

    def write_table(self):
        lines = ["  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode"]
        for i, (local, uid, inode) in enumerate(self.rows):
            lines.append("   %d: %s 00000000:0000 0A 00000000:00000000 00:00000000 00000000  %d        0 %d 1 0"
                         % (i, local, uid, inode))
        with open(os.path.join(self.root, "net", "tcp"), "w") as f:
            f.write("\n".join(lines) + "\n")

    def listen(self, port, uid, inode, local="0100007F"):
        self.rows.append(("%s:%04X" % (local, port), uid, inode))
        self.write_table()

    def process(self, pid, cmdline, inode=None, environ=None):
        d = os.path.join(self.root, str(pid))
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

    def close(self):
        shutil.rmtree(self.root, ignore_errors=True)


class Identify(unittest.TestCase):
    def setUp(self):
        self.proc = FakeProc()
        self.addCleanup(self.proc.close)

    def who(self, port):
        return listener.identify(port, self.proc.root, ENV)

    def test_mind_views_browser_is_known_whichever_way_the_command_line_is_written(self):
        for form in (nul, joined):
            proc = FakeProc()
            self.addCleanup(proc.close)
            proc.listen(9223, 1000, 11)
            proc.process(41, form("/usr/lib/chromium/chromium", "--remote-debugging-port=9223",
                                  "--user-data-dir=" + MIND_VIEW, "about:blank"), inode=11)
            held = listener.identify(9223, proc.root, ENV)
            self.assertEqual((held.place, held.uid, held.pids), (listener.MIND_VIEW, 1000, (41,)), form.__name__)

    def test_no_flag_is_the_default_profile_which_is_the_persons(self):
        self.proc.listen(9222, 1000, 5)
        self.proc.process(9, joined("chromium", "--remote-debugging-port=9222"), inode=5)
        self.assertEqual(self.who(9222).place, listener.DEFAULT)

    def test_the_persons_profile_spelled_out_is_not_mind_views(self):
        self.proc.listen(9222, 1000, 5)
        self.proc.process(9, joined("chromium", "--user-data-dir=" + PERSON), inode=5)
        held = self.who(9222)
        self.assertEqual(held.place, listener.OTHER)
        self.assertFalse(listener.is_mind_side(held.place))

    def test_a_prefix_of_mind_views_path_is_not_mind_views(self):
        self.proc.listen(9223, 1000, 5)
        self.proc.process(9, joined("chromium", "--user-data-dir=" + MIND_VIEW + "-copy"), inode=5)
        self.assertEqual(self.who(9223).place, listener.OTHER)

    def test_a_flag_given_twice_or_empty_is_refused_not_guessed(self):
        self.proc.listen(9223, 1000, 5)
        for line in (joined("chromium", "--user-data-dir=/a", "--user-data-dir=" + MIND_VIEW),
                     nul("chromium", "--user-data-dir=")):
            self.proc.process(9, line, inode=5)
            with self.assertRaises(listener.Unidentified):
                self.who(9223)

    def test_nobody_two_sockets_unseen_and_disagreeing_holders_are_all_refusals(self):
        with self.assertRaises(listener.Unidentified) as nobody:
            self.who(9999)
        self.assertTrue(nobody.exception.nobody)
        self.proc.listen(9223, 1000, 1)
        with self.assertRaises(listener.Unidentified) as unseen:      # listening, holder not visible
            self.who(9223)
        self.assertFalse(unseen.exception.nobody)
        self.proc.listen(9223, 1000, 2)                                # a second socket
        with self.assertRaises(listener.Unidentified):
            self.who(9223)
        proc = FakeProc()
        self.addCleanup(proc.close)
        proc.listen(9223, 1000, 1)
        proc.process(1, joined("chromium", "--user-data-dir=" + MIND_VIEW), inode=1)
        proc.process(2, joined("chromium"), inode=1)
        with self.assertRaises(listener.Unidentified) as split:
            listener.identify(9223, proc.root, ENV)
        self.assertIn("disagree", str(split.exception))

    def test_the_exact_address_beats_a_wildcard_one_as_the_kernel_does(self):
        self.proc.listen(9223, 1000, 1, local="00000000")
        self.proc.listen(9223, 1000, 2)
        self.proc.process(1, joined("chromium"), inode=1)
        self.proc.process(2, joined("chromium", "--user-data-dir=" + MIND_VIEW), inode=2)
        self.assertEqual(self.who(9223).place, listener.MIND_VIEW)

    def test_a_companion_identity_is_a_minds_too(self):
        self.proc.listen(9230, 1000, 3)
        self.proc.process(3, joined("chromium", "--user-data-dir=" + ports.identities_root(ENV) + "/work"), inode=3)
        self.assertEqual(self.who(9230).place, listener.IDENTITY)


class StopMindView(unittest.TestCase):
    """The update's step: Mind View's browsers stop, the person's never do."""

    def test_only_mind_views_main_processes_are_chosen(self):
        proc = FakeProc()
        self.addCleanup(proc.close)
        env = {"HOME": "/home/p"}
        proc.process(10, joined("chromium", "--remote-debugging-port=9222"), environ=env)                        # the person's, old flags
        proc.process(11, joined("chromium", "--remote-debugging-port=9222", "--user-data-dir=" + PERSON), environ=env)  # the person's, new flags
        proc.process(12, joined("chromium", "--type=renderer", "--user-data-dir=" + MIND_VIEW), environ=env)    # a helper
        proc.process(20, joined("chromium", "--remote-debugging-port=9222", "--user-data-dir=" + MIND_VIEW), environ=env)  # legacy Mind View
        proc.process(21, joined("chromium", "--remote-debugging-port=9223", "--user-data-dir=" + MIND_VIEW), environ=env)  # current Mind View
        proc.process(30, nul("vim", "x"), environ=env)
        chosen = sorted(pid for pid, _ in stop_mind_view.mind_view_mains(proc.root))
        self.assertEqual(chosen, [20, 21])

    def test_each_process_is_read_in_its_own_environment(self):
        proc = FakeProc()
        self.addCleanup(proc.close)
        other = ports.mind_view_profile({"HOME": "/home/q", "XDG_DATA_HOME": "/data/q"})
        proc.process(40, joined("chromium", "--user-data-dir=" + other), environ={"HOME": "/home/q", "XDG_DATA_HOME": "/data/q"})
        # The same path under an environment that would not give it is not Mind View's.
        proc.process(41, joined("chromium", "--user-data-dir=" + other), environ={"HOME": "/home/p"})
        self.assertEqual([pid for pid, _ in stop_mind_view.mind_view_mains(proc.root)], [40])


if __name__ == "__main__":
    unittest.main()
