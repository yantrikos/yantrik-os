#!/usr/bin/env python3
"""Who is the person (#1085): a caller on the person's uid is the person only if its ancestry proves it.

    python3 -m unittest discover -s tests/browser-core -p 'test_who.py' -v

Linux only. The ancestry is a fake /proc (and, for the cases that need real processes, the real
one); the shell's `caller_kind` answer is a stand-in, because the rule it applies is the shell's
and is tested in Rust (control_caller_kind.rs).
"""

import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]

from yantrik_browser import who  # noqa: E402

NOT_A_MIND = {"known": True, "mind": False}
A_MIND = {"known": True, "mind": True, "who": "Hermes Agent"}


class Tree:
    """A fake /proc with processes in it: add(pid, ppid, exe, cmd, start=, environ=)."""

    def __init__(self):
        self.tmp = tempfile.mkdtemp()
        self.root = self.tmp

    def add(self, pid, ppid, exe, cmd=None, start=None, environ=()):
        p = os.path.join(self.root, str(pid))
        os.makedirs(p)
        start = pid * 10 if start is None else start
        with open(os.path.join(p, "stat"), "w") as f:
            f.write("%d (x) S %d %s\n" % (pid, ppid, " ".join(["0"] * 17 + [str(start)])))
        os.symlink(exe, os.path.join(p, "exe"))
        with open(os.path.join(p, "cmdline"), "wb") as f:
            f.write(b"\0".join(a.encode() for a in (cmd or [exe])) + b"\0")
        with open(os.path.join(p, "environ"), "wb") as f:
            f.write(b"\0".join(e.encode() for e in environ) + b"\0")

    def chain(self, *members):
        """Add members from the caller down to init (exe, cmd): pids 200, 199, ... and 1. Returns 200."""
        n = len(members)
        pids = [200 - i for i in range(n - 1)] + [1]
        for i, (exe, cmd) in enumerate(members):
            self.add(pids[i], pids[i + 1] if i < n - 1 else 0, exe, cmd, start=1000 - i * 10)
        return 200


class Case(unittest.TestCase):
    def setUp(self):
        self.tree = Tree()
        self.addCleanup(shutil.rmtree, self.tree.tmp, ignore_errors=True)
        who._proven.clear()
        self.asked = []

    def shell(self, answer):
        def ask(pid):
            self.asked.append(pid)
            if isinstance(answer, Exception):
                raise answer
            return answer
        return ask

    def proves(self, pid, answer=NOT_A_MIND):
        return who.proves_person(pid, proc=self.tree.root, ask=self.shell(answer))

    def persons_terminal(self, *, shell_exe="/opt/yantrik/bin/yantrik-ui"):
        return self.tree.chain(("/usr/bin/python3", ["python3", "yos", "act", "browser", "tabs"]),
                               ("/bin/bash", ["-bash"]), ("/usr/bin/foot", ["foot"]),
                               (shell_exe, ["yantrik-ui"]), ("/usr/lib/systemd/systemd", ["systemd"]))


class TestTheChain(Case):
    def test_the_persons_own_terminal_opened_from_the_desktop_proves(self):
        self.assertTrue(self.proves(self.persons_terminal()))
        self.assertEqual(len(self.asked), 1)

    def test_a_login_over_ssh_proves(self):
        pid = self.tree.chain(("/usr/bin/python3", ["python3", "yos"]), ("/bin/bash", ["-bash"]),
                              ("/usr/sbin/sshd", ["sshd: p@pts/0"]), ("/usr/lib/systemd/systemd", ["systemd"]))
        self.assertTrue(self.proves(pid))

    def test_a_chain_with_no_anchor_does_not(self):
        pid = self.tree.chain(("/usr/bin/python3", ["python3", "yos"]), ("/bin/bash", ["-bash"]),
                              ("/usr/lib/systemd/systemd", ["systemd", "--user"]), ("/sbin/init", ["init"]))
        self.assertFalse(self.proves(pid))
        self.assertEqual(self.asked, [], "the shell is not asked when the chain already fails")

    def test_a_yos_started_by_a_harnesss_terminal_tool_is_not_the_person(self):
        """yos <- bash <- the gateway <- systemd --user: the gateway is no person's session."""
        pid = self.tree.chain(("/usr/bin/python3", ["python3", "yos", "act", "browser", "go"]),
                              ("/bin/bash", ["bash", "-c", "yos act browser go"]),
                              ("/h/venv/bin/python", ["python", "-m", "hermes_cli.main", "gateway", "run"]),
                              ("/usr/lib/systemd/systemd", ["systemd", "--user"]), ("/sbin/init", ["init"]))
        self.assertFalse(self.proves(pid, NOT_A_MIND))

    def test_the_shell_naming_a_mind_in_the_ancestry_refuses_even_under_an_anchor(self):
        """A gateway someone started from an ssh session: anchored, and the desktop knows it."""
        pid = self.tree.chain(("/usr/bin/python3", ["python3", "yos"]),
                              ("/h/venv/bin/python", ["python", "-m", "hermes_cli.main"]),
                              ("/usr/sbin/sshd", ["sshd"]), ("/sbin/init", ["init"]))
        self.assertFalse(self.proves(pid, A_MIND))

    def test_anything_under_yos_mcp_is_not_the_person_whatever_the_shell_says(self):
        pid = self.tree.chain(("/usr/bin/python3", ["python3", "yos"]),
                              ("/usr/bin/python3", ["python3", "/opt/yantrik/bin/yos-mcp"]),
                              ("/opt/yantrik/bin/yantrik-ui", ["yantrik-ui"]), ("/sbin/init", ["init"]))
        self.assertFalse(self.proves(pid, NOT_A_MIND))

    def test_a_yos_that_declares_it_was_run_by_yos_mcp_is_not_the_person(self):
        pid = self.persons_terminal()
        self.assertTrue(self.proves(pid))
        who._proven.clear()
        with open(os.path.join(self.tree.root, str(pid), "environ"), "wb") as f:
            f.write(b"HOME=/h\0YOS_VIA=yos-mcp\0")
        self.assertFalse(self.proves(pid))

    def test_an_ancestor_that_has_exited_does_not_read_as_the_person(self):
        """The terminal's shell is gone: the caller is under init, with no anchor above it."""
        self.tree.add(1, 0, "/sbin/init", start=1)
        self.tree.add(300, 1, "/usr/bin/python3", ["python3", "yos"], start=3000)
        self.assertFalse(self.proves(300))

    def test_the_shell_not_answering_or_not_knowing_is_not_proof(self):
        pid = self.persons_terminal()
        for answer in (who.Unproven("no shell"), {"known": False, "mind": False}, {"mind": False}, {}, [],
                       {"known": True}):
            who._proven.clear()
            self.assertFalse(self.proves(pid, answer), answer)

    def test_a_reused_pid_a_loop_and_an_unreadable_link_are_not_proof(self):
        pid = self.persons_terminal()
        self.tree.add(5000, 199, "/x", start=1)           # a child that started before its parent
        self.assertFalse(self.proves(5000), "a child that started before its parent")
        self.tree.add(6000, 6001, "/x")
        self.tree.add(6001, 6000, "/x")
        self.assertFalse(self.proves(6000), "a loop")
        shutil.rmtree(os.path.join(self.tree.root, "199"))
        self.assertFalse(self.proves(pid), "a missing link")

    def test_the_desktops_own_call_is_not_the_person_and_the_shell_is_not_asked_about_itself(self):
        self.tree.add(300, 1, "/opt/yantrik/bin/yantrik-ui", start=3000)
        self.tree.add(1, 0, "/sbin/init", start=1)
        self.assertFalse(self.proves(300))
        self.assertEqual(self.asked, [])

    def test_what_is_a_mind_and_what_is_only_unproven(self):
        """Told apart so the person is not told they are a Mind."""
        def kind(pid, answer=NOT_A_MIND):
            who._proven.clear()
            return who.classify(pid, proc=self.tree.root, ask=self.shell(answer))
        pid = self.persons_terminal()
        self.assertEqual(kind(pid), "person")
        self.assertEqual(kind(pid, A_MIND), "mind")
        self.assertEqual(kind(pid, {"known": False}), "unproven")
        self.assertEqual(kind(pid, who.Unproven("down")), "unproven")
        self.tree.add(300, 1, "/usr/bin/python3", ["python3", "yos"], start=3000)
        self.tree.add(1, 0, "/sbin/init", start=1) if not os.path.exists(os.path.join(self.tree.root, "1")) else None
        self.assertEqual(kind(300), "unproven", "a Terminal whose shell restarted")
        with open(os.path.join(self.tree.root, str(pid), "environ"), "wb") as f:
            f.write(b"YOS_VIA=yos-mcp")
        self.assertEqual(kind(pid), "mind")

    def test_a_loop_with_equal_start_times_is_not_proof(self):
        self.tree.add(6000, 6001, "/x", start=5)
        self.tree.add(6001, 6000, "/x", start=5)
        self.assertFalse(self.proves(6000))

    def test_init_itself_unreadable_is_not_proof(self):
        pid = self.persons_terminal()
        shutil.rmtree(os.path.join(self.tree.root, "1"))
        self.assertFalse(self.proves(pid))

    def test_a_chain_that_ends_short_of_init_is_not_proof(self):
        self.tree.add(7000, 0, "/opt/yantrik/bin/yantrik-ui", start=1)
        self.tree.add(7001, 7000, "/usr/bin/python3", start=2)
        self.assertFalse(self.proves(7001))

    def test_asking_a_shell_that_does_not_answer_is_not_an_answer(self):
        from unittest import mock
        for failure in (OSError("no socket"), ConnectionError("reset"), ValueError("junk")):
            with mock.patch.object(who.wire, "call_once", side_effect=failure):
                with self.assertRaises(who.Unproven):
                    who.ask_shell(123)

    def test_a_proof_is_kept_for_the_same_process_only_briefly(self):
        pid = self.persons_terminal()
        clock = [0.0]
        for _ in range(3):
            self.assertTrue(who.proves_person(pid, proc=self.tree.root, ask=self.shell(NOT_A_MIND),
                                              now=lambda: clock[0]))
        self.assertEqual(len(self.asked), 1)
        clock[0] += who.REMEMBER + 1
        self.assertTrue(who.proves_person(pid, proc=self.tree.root, ask=self.shell(NOT_A_MIND), now=lambda: clock[0]))
        self.assertEqual(len(self.asked), 2)


@unittest.skipUnless(os.path.isdir("/proc/self"), "needs /proc")
class TestRealProcesses(unittest.TestCase):
    """The real /proc, with this test process standing in as the anchor the person's session has."""

    def setUp(self):
        who._proven.clear()
        me = os.path.basename(os.path.realpath("/proc/self/exe"))
        old = who.ANCHORS
        who.ANCHORS = old | {me}
        self.addCleanup(setattr, who, "ANCHORS", old)
        self.kids = []
        self.addCleanup(self.reap)

    def reap(self):
        for k in self.kids:
            k.kill()
            k.wait()

    def by_name(self, pid):
        """Stands in for the shell's rule: a mind when a process in the ancestry is named hermes."""
        p = pid
        while p > 1:
            with open("/proc/%d/cmdline" % p, "rb") as f:
                if b"hermes" in f.read():
                    return A_MIND
            with open("/proc/%d/stat" % p) as f:
                p = int(f.read().rpartition(")")[2].split()[1])
        return NOT_A_MIND

    def start(self, name):
        k = subprocess.Popen(["bash", "-c", "exec -a %s sleep 30" % name])
        self.kids.append(k)
        for _ in range(100):
            try:
                with open("/proc/%d/cmdline" % k.pid, "rb") as f:
                    if name.encode() in f.read():
                        break
            except OSError:
                pass
            time.sleep(0.02)
        return k.pid

    def test_a_process_under_this_one_with_no_harness_proves(self):
        self.assertTrue(who.proves_person(self.start("sleeper"), ask=self.by_name))

    def test_a_real_process_named_hermes_does_not(self):
        pid = self.start("hermes-gateway")
        self.assertFalse(who.proves_person(pid, ask=self.by_name))

    def test_a_real_orphan_does_not(self):
        """Its parent exits, it is reparented to init (or a subreaper), and this process is no longer above it."""
        out = subprocess.run(["bash", "-c", "sleep 30 >/dev/null 2>&1 & echo $!"], capture_output=True, text=True).stdout
        pid = int(out.strip())
        self.addCleanup(lambda: os.kill(pid, 9) if os.path.exists("/proc/%d" % pid) else None)
        time.sleep(0.2)
        self.assertFalse(who.proves_person(pid, ask=lambda p: NOT_A_MIND))


if __name__ == "__main__":
    unittest.main()
