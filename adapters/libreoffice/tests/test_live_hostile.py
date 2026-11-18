"""The review's probes of #962, run inside the real sandbox (#953 review H1, H2, L2).

Each test starts a stand-in for a LibreOffice taken over by a document, through the very command
`AgentOffice` starts soffice with, and the stand-in does what the review did from inside: swap a
file or a folder in the work folder for a link to the person's files, or its UNO socket for a link
to the person's LibreOffice pipe. They run twice where it matters: with Landlock as this kernel
offers it, which refuses to make the link at all (the second wall), and through a launcher with
Landlock turned off, as on a kernel without it, where only the adapter's own guards stand. Each
has a control that shows the attack reaching the person's files when the guard is not there.
They need bwrap able to make user namespaces, and skip with the reason where it cannot.
"""

import json
import os
import secrets
import socket
import subprocess
import sys
import time
import unittest
from unittest import mock

import support
from test_fence import SENTINEL, bwrap_missing
from test_sandbox import Listening, Machine
from yantrik_libreoffice import agent_office, copies, fence, sandbox, unopipe
from yantrik_libreoffice.canaries import Canaries
from yantrik_libreoffice.workdir import WorkFolder

SANDBOXED = "the-sandboxed-libreoffice"
# Added to a copy of the launcher: a kernel that offers no Landlock.
LANDLOCK_OFF = '\ndef abi():\n    raise FenceError("turned off by the test")\n\n'

# Inside: do one thing the review did, and say how it went as JSON.
INSIDE = r"""
import json, os, socket, sys, time
told = json.loads(sys.argv[1])
said = {}
def attempt(name, fn):
    try:
        fn()
        said[name] = "done"
    except OSError as e:
        said[name] = "refused: %s" % e.strerror
if told["do"] == "link file":
    os.remove(told["at"])
    attempt("link", lambda: os.symlink(told["to"], told["at"]))
elif told["do"] == "link folder":
    os.rename(told["at"], told["at"] + "-moved")
    attempt("link", lambda: os.symlink(told["to"], told["at"]))
elif told["do"] == "user namespace":
    pid = os.fork()
    if pid == 0:
        try:
            os.unshare(os.CLONE_NEWUSER)
            os._exit(0)
        except OSError:
            os._exit(1)
    said["user namespace"] = "made" if os.waitpid(pid, 0)[1] == 0 else "refused"
elif told["do"] == "serve":
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.bind(told["at"])
    s.listen(8)
    if told["when"] == "before":
        os.remove(told["at"])
        attempt("link", lambda: os.symlink(told["to"], told["at"]))
    s.settimeout(0.2)
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if told["when"] == "after" and os.path.exists(told["go"]) and "link" not in said:
            os.remove(told["at"])
            attempt("link", lambda: os.symlink(told["to"], told["at"]))
            open(told["go"] + ".done", "w").close()
            if told.get("then_exit"):
                break
        try:
            c, _ = s.accept()
        except OSError:
            continue
        try:
            c.sendall(told["say"].encode())
        except OSError:
            pass
        c.close()
print(json.dumps(said), flush=True)
"""


def hear(path):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(5)
    try:
        s.connect(path)
        return s.recv(100).decode()
    finally:
        s.close()


def text_of(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


@unittest.skipUnless(sys.platform.startswith("linux"), "bwrap is Linux's")
class Inside(Machine):
    def setUp(self):
        missing = bwrap_missing()
        if missing:
            self.skipTest("bwrap cannot make a sandbox here (%s)" % missing)
        super().setUp()
        self.key = os.path.join(self.home, ".ssh", "id_ed25519")
        self.landlock_off = os.path.join(self.tmp, "fence-no-landlock.py")
        with open(fence.__file__, encoding="utf-8") as f:
            text = f.read()
        main = '\nif __name__ == "__main__":'
        with open(self.landlock_off, "w", encoding="utf-8") as f:
            f.write(text.replace(main, LANDLOCK_OFF + main))

    def argv(self, told, landlock, drop=()):
        canaries = Canaries(self.private.root).open()
        self.addCleanup(canaries.close)
        launcher = agent_office.LAUNCHER if landlock else self.landlock_off
        with mock.patch.object(agent_office, "LAUNCHER", launcher):
            argv = agent_office.sandboxed(sandbox.find_bwrap(), self.private, None, self.home,
                                          self.env, canaries,
                                          [sandbox.PYTHON, "-I", "-c", INSIDE, json.dumps(told)])
        return [a for a in argv if a not in drop]

    def inside(self, told, landlock, drop=()):
        done = subprocess.run(self.argv(told, landlock, drop), capture_output=True, text=True,
                              timeout=60, env=sandbox.environment(self.env))
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("proved, with %s" % ("Landlock" if landlock else "no Landlock"),
                      done.stderr)
        return json.loads(done.stdout)


class TestTheWorkFolder(Inside):
    """H1: a link planted in the work folder from inside is never followed outside."""

    def setUp(self):
        super().setUp()
        self.person = os.path.join(self.home, "Documents", "book.ods")
        with open(self.person, "w", encoding="utf-8") as f:
            f.write("the person's book")
        self.folder = WorkFolder(self.private.work)
        self.addCleanup(self.folder.close)
        slot = self.folder.slot()
        self.copy = copies.take(self.person, self.person, os.path.join(slot, "book.ods"),
                                self.folder)
        self.inside_path = os.path.join(sandbox.WORK, os.path.basename(slot), "book.ods")

    def plant(self, landlock):
        return self.inside({"do": "link file", "at": self.inside_path, "to": self.key}, landlock)

    def test_control_without_landlock_the_planted_link_reaches_the_key(self):
        self.assertEqual(self.plant(landlock=False), {"link": "done"})
        self.assertEqual(text_of(self.copy.work), SENTINEL, "the old put_back read this")

    def test_without_landlock_save_refuses_the_planted_link(self):
        self.plant(landlock=False)
        with self.assertRaises(support.OfficeError) as caught:
            copies.put_back(self.copy, self.folder.read(self.copy.work))
        self.assertIn("not a plain file of its own", str(caught.exception))
        self.assertEqual(text_of(self.person), "the person's book")

    def test_with_landlock_the_link_is_not_even_made(self):
        said = self.plant(landlock=True)
        self.assertTrue(said["link"].startswith("refused"), said)

    def test_without_landlock_a_slot_swapped_for_a_folder_link_gets_nothing_written_there(self):
        slot = os.path.dirname(self.inside_path)
        ssh = os.path.dirname(self.key)
        said = self.inside({"do": "link folder", "at": slot, "to": ssh}, landlock=False)
        self.assertEqual(said, {"link": "done"})
        new = os.path.join(os.path.dirname(self.copy.work), "new.ods")
        self.folder.write_new(new, b"the copy")
        self.assertEqual(os.listdir(ssh), ["id_ed25519"], "nothing landed in the person's .ssh")
        with open(new, "wb") as f:                     # the control: by path, as before the fix
            f.write(b"the copy")
        self.assertIn("new.ods", os.listdir(ssh))


class TestThePipe(Inside):
    """H2: the UNO name never leads to the person's LibreOffice."""

    def setUp(self):
        super().setUp()
        self.persons = "/tmp/OSL_PIPE_%d_yantrik-hostile-%s" % (os.getuid(), secrets.token_hex(6))
        Listening(self, socket.AF_UNIX, self.persons)
        self.addCleanup(lambda: os.path.lexists(self.persons) and os.remove(self.persons))
        name = "yantrik-libreoffice-agent-test-%s" % secrets.token_hex(6)
        self.go = os.path.join(self.private.work, "go")
        self.told = {"do": "serve", "at": "/tmp/OSL_PIPE_%d_%s" % (os.getuid(), name),
                     "to": self.persons, "say": SANDBOXED,
                     "go": os.path.join(sandbox.WORK, "go")}
        self.name = name

    def start(self, when, landlock=False):
        self.told["when"] = when
        process = subprocess.Popen(self.argv(self.told, landlock), stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True,
                                   env=sandbox.environment(self.env))
        self.addCleanup(lambda: process.communicate())
        self.addCleanup(process.kill)
        pipe = unopipe.Pipe(self.name, self.private.pipes, self.private.uno, process.pid)
        self.addCleanup(pipe.remove)
        deadline = time.monotonic() + 30
        while not pipe.made() or (when == "before" and not os.path.islink(
                os.path.join(self.private.pipes, pipe.file))):
            self.assertIsNone(process.poll(), process.stderr.read() if process.poll() else "")
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.05)
        return process, pipe

    def test_control_without_landlock_the_planted_link_reaches_the_persons_libreoffice(self):
        _, pipe = self.start("before")
        self.assertEqual(hear(os.path.join(self.private.pipes, pipe.file)), SENTINEL)

    def test_without_landlock_a_link_planted_before_the_pin_is_refused(self):
        _, pipe = self.start("before")
        with self.assertRaises(unopipe.PipeRefused) as caught:
            pipe.pin()
        self.assertIn("not a socket", str(caught.exception))
        self.assertFalse(os.path.lexists(pipe.link))

    def test_without_landlock_a_link_planted_after_the_pin_still_reaches_the_sandbox(self):
        _, pipe = self.start("after")
        pipe.pin()
        self.assertEqual(hear(pipe.link), SANDBOXED)
        open(self.go, "w").close()
        deadline = time.monotonic() + 30
        while not os.path.exists(self.go + ".done"):
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.05)
        self.assertTrue(os.path.islink(os.path.join(self.private.pipes, pipe.file)))
        pipe.check()
        self.assertEqual(hear(pipe.link), SANDBOXED)
        self.assertEqual(hear(os.path.join(self.private.pipes, pipe.file)), SENTINEL,
                         "the control: by the sandbox's own name, the person's is reached")

    def test_with_landlock_the_link_is_not_even_made(self):
        self.told["then_exit"] = True
        process, pipe = self.start("after", landlock=True)
        open(self.go, "w").close()
        out, err = process.communicate(timeout=60)
        self.assertIn("proved, with Landlock", err)
        said = json.loads(out)
        self.assertTrue(said["link"].startswith("refused"), said)
        self.assertFalse(os.path.lexists(os.path.join(self.private.pipes, pipe.file)))


class TestNoUserNamespacesInside(Inside):
    """L2: the sandbox cannot make user namespaces of its own."""

    def test_a_user_namespace_is_refused_inside(self):
        if not hasattr(os, "unshare"):
            self.skipTest("this Python has no os.unshare")
        self.assertEqual(self.inside({"do": "user namespace"}, landlock=True),
                         {"user namespace": "refused"})

    def test_control_without_disable_userns_one_is_made(self):
        if not hasattr(os, "unshare"):
            self.skipTest("this Python has no os.unshare")
        self.assertEqual(self.inside({"do": "user namespace"}, landlock=True,
                                     drop=("--disable-userns",)),
                         {"user namespace": "made"})


if __name__ == "__main__":
    unittest.main()
