"""The launcher an agent's LibreOffice starts through (fence.py), and the places it must keep out
(places.py; #942, #953).

The proof tests run anywhere Linux is: they call the proof functions on this machine, where the
person's places and the canaries are reachable, so each must refuse. The Landlock tests need Landlock
among the kernel's security modules, and skip with the reason where it is not. The sandbox itself is
tests/test_sandbox.py's.
"""

import errno
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import unittest

import support
from yantrik_libreoffice import fence, places, sandbox

SENTINEL = "SENTINEL-fence-7c41"

# Run under the launcher: try each path, and say what happened, as JSON on stdout.
PROBE = r"""
import json, sys
out = {}
for path in sys.argv[1:]:
    try:
        with open(path) as f:
            out[path] = f.read()
    except OSError as e:
        out[path] = "refused: %s" % e.strerror
print(json.dumps(out))
"""


def landlock_missing():
    try:
        fence.abi()
    except fence.FenceError as e:
        return str(e)
    return None


def put(path, text=SENTINEL):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    return path


class Home(unittest.TestCase):
    """A home with protected places, each holding the sentinel, and ordinary folders beside them."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="yantrik-fence-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.home = os.path.join(self.tmp, "home")
        h = self.home
        self.protected = [put(os.path.join(h, rel)) for rel in (
            ".ssh/id_ed25519", ".local/share/yantrik/snippets/snippets.json",
            ".local/state/yantrik/agent.log", ".config/yantrik-mind.env",
            "Pictures/Screenshots/shot.txt", "memory.db", ".git-credentials")]
        self.plain = put(os.path.join(h, "Documents", "plan.txt"), "plan")
        self.env = {"HOME": h}


class TestThePlaces(Home):
    def test_the_protected_list_is_home_paths_own(self):
        """places.PROTECTED is read off the Rust list; a place added there must be added here."""
        source = os.path.join(support.REPO, "crates", "yantrik-ipc-contracts", "src", "home_paths",
                              "mod.rs")
        with open(source, encoding="utf-8") as f:
            text = f.read()
        body = text[text.index("pub const PROTECTED: &[&str] = &["):]
        body = body[:body.index("];")]
        body = re.sub(r"//[^\n]*", "", body)
        self.assertEqual(tuple(re.findall(r'"([^"]+)"', body)), places.PROTECTED)

    def test_a_moved_data_home_is_protected_where_it_moved(self):
        data = os.path.join(self.tmp, "elsewhere")
        found = places.protected_places(self.home, dict(self.env, XDG_DATA_HOME=data))
        self.assertIn(os.path.join(data, "yantrik"), found)
        self.assertIn(os.path.join(data, "keyrings"), found)

    def test_the_probes_are_the_home_and_every_protected_place_that_is_there(self):
        probes = places.probes(self.home, self.env)
        self.assertEqual(probes[0], self.home)
        for path in (os.path.join(self.home, ".ssh"), os.path.join(self.home, "memory.db")):
            self.assertIn(path, probes)
        self.assertNotIn(os.path.join(self.home, ".aws"), probes, "not there, nothing to prove")

    def test_hidden_holds_the_home_the_protected_places_and_the_runtime_folders(self):
        env = dict(self.env, XDG_RUNTIME_DIR="/run/user/4242")
        hidden = places.hidden(self.home, env, uid=4242)
        for path in (self.home, os.path.join(self.home, ".ssh"), "/run/user/4242",
                     "/tmp/.X11-unix"):
            self.assertIn(path, hidden)

    def test_the_persons_sockets_are_found_in_the_runtime_folder_and_tmp(self):
        runtime = os.path.join(self.tmp, "run")
        os.makedirs(os.path.join(runtime, "yantrik"))
        made = []
        for path in (os.path.join(runtime, "bus"), os.path.join(runtime, "yantrik", "app-x.sock")):
            sock = socket.socket(socket.AF_UNIX)
            sock.bind(path)
            made.append(sock)
            self.addCleanup(sock.close)
        found = places.persons_sockets({"XDG_RUNTIME_DIR": runtime}, uid=os.getuid())
        self.assertIn(os.path.join(runtime, "bus"), found)
        self.assertIn(os.path.join(runtime, "yantrik", "app-x.sock"), found)

    def test_libreoffice_installed_in_the_home_is_unsuitable(self):
        program = put(os.path.join(self.home, "lo", "program", "soffice"), "#!/bin/sh\n")
        with self.assertRaises(places.Unsuitable):
            places.install_root(program, self.home)
        self.assertEqual(places.install_root("/usr/lib/libreoffice/program/soffice", self.home),
                         "/usr/lib/libreoffice")


@unittest.skipUnless(sys.platform.startswith("linux"), "the proof reads Linux's errors")
class TestTheProof(Home):
    """Each proof must refuse on this machine, where the places and canaries are reachable, and
    accept only the kernel's words for absent or refused (#942 review L3)."""

    def test_a_protected_place_that_opens_fails_the_proof(self):
        with self.assertRaises(fence.FenceError) as caught:
            fence.prove([self.protected[0]])
        self.assertIn("did not hold", str(caught.exception))

    def test_absent_and_refused_probes_pass(self):
        fence.prove([os.path.join(self.tmp, "not-there"), os.path.join(self.plain, "below-a-file")])

    def test_an_error_that_is_neither_absent_nor_refused_fails_the_proof(self):
        for code in (errno.EMFILE, errno.ENXIO, errno.ELOOP, errno.EIO):
            with support.mock.patch("os.open", side_effect=OSError(code, os.strerror(code))):
                with self.assertRaises(fence.FenceError) as caught:
                    fence.prove(["/anything"])
            self.assertIn("neither absent nor refused", str(caught.exception), code)

    def test_a_fifo_probe_does_not_hang_the_proof(self):
        fifo = os.path.join(self.tmp, "fifo")
        os.mkfifo(fifo)
        with self.assertRaises(fence.FenceError):
            fence.prove([fifo])   # opened at once (O_NONBLOCK), and it is not a cover

    def test_an_empty_cover_passes_and_a_folder_with_files_does_not(self):
        cover = os.path.join(self.tmp, "cover")
        os.mkdir(cover)
        fence.prove([cover, "/dev/null"])
        with self.assertRaises(fence.FenceError):
            fence.prove([os.path.join(self.home, ".ssh")])

    def test_a_socket_that_answers_fails_the_proof(self):
        path = os.path.join(self.tmp, "person.sock")
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(path)
        listener.listen(1)
        self.addCleanup(listener.close)
        with self.assertRaises(fence.FenceError) as caught:
            fence.prove_sockets([path], [])
        self.assertIn("could still be reached", str(caught.exception))
        fence.prove_sockets([os.path.join(self.tmp, "gone.sock")], [])

    def test_an_abstract_socket_that_answers_fails_the_proof(self):
        name = "yantrik-test-abstract-%d" % os.getpid()
        listener = socket.socket(socket.AF_UNIX)
        listener.bind("\0" + name)
        listener.listen(1)
        self.addCleanup(listener.close)
        with self.assertRaises(fence.FenceError):
            fence.prove_sockets([], [name])

    def test_a_network_with_a_listening_port_or_a_route_out_fails_the_proof(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        self.addCleanup(listener.close)
        with self.assertRaises(fence.FenceError):
            fence.prove_network([listener.getsockname()[1]])
        with support.mock.patch("socket.if_nameindex", return_value=[(1, "lo")]), \
                support.mock.patch.object(fence, "_connect", side_effect=[
                    OSError(errno.ECONNREFUSED, "refused"), None]):
            with self.assertRaises(fence.FenceError) as caught:
                fence.prove_network([1])
        self.assertIn("route out", str(caught.exception))

    def test_only_loopback_and_nothing_reached_passes(self):
        unreachable = OSError(errno.ENETUNREACH, "unreachable")
        with support.mock.patch("socket.if_nameindex", return_value=[(1, "lo")]), \
                support.mock.patch.object(fence, "_connect", side_effect=[
                    OSError(errno.ECONNREFUSED, "refused"), unreachable]):
            fence.prove_network([1])
        with support.mock.patch("socket.if_nameindex", return_value=[(1, "lo"), (2, "eth0")]):
            with self.assertRaises(fence.FenceError):
                fence.prove_network([])

    def test_the_launcher_runs_nothing_when_the_proof_fails(self):
        marker = os.path.join(self.tmp, "ran")
        argv = fence.command(sys.executable, fence.__file__, [], [self.protected[0]],
                             ["/bin/sh", "-c", "echo ran > %s" % marker])
        done = subprocess.run(argv, capture_output=True, text=True, timeout=60)
        self.assertEqual(done.returncode, fence.REFUSED, done.stderr)
        self.assertIn("refused: the sandbox did not hold", done.stderr)
        self.assertFalse(os.path.exists(marker), "the command never ran")

    def test_the_launchers_arguments_go_round_trip(self):
        argv = fence.command("/usr/bin/python3", "/yantrik/bin/fence.py", [("system", "/usr")],
                             ["/h"], ["soffice", "--headless"], sockets=["/s"], abstract=["a"],
                             ports=[7])
        told, run = fence._parse(argv[3:])  # noqa: SLF001 - the parser the launcher runs
        self.assertEqual((told["rules"], told["probes"], told["sockets"], told["abstract"],
                          told["ports"], run),
                         ([("system", "/usr")], ["/h"], ["/s"], ["a"], [7],
                          ["soffice", "--headless"]))
        with self.assertRaises(fence.FenceError):
            fence._parse(["--tcp", "x", "--", "a"])  # noqa: SLF001


def bwrap_missing():
    """Why bwrap cannot make a sandbox here, or None."""
    try:
        sandbox.check(sandbox.find_bwrap())
    except sandbox.SandboxError as e:
        return str(e)
    return None


@unittest.skipUnless(sys.platform.startswith("linux"), "Landlock is Linux's")
class TestLandlockInside(Home):
    """Landlock, the second wall: a process under the launcher's rules opens only what they allow.
    It runs with the whole filesystem shown and only the network cut off (the proof needs that),
    so Landlock is the one deciding what opens."""

    def setUp(self):
        missing = landlock_missing() or bwrap_missing()
        if missing:
            self.skipTest("Landlock or bwrap is not available here (%s)" % missing)
        super().setUp()

    def netless(self, argv):
        return [sandbox.find_bwrap(), "--unshare-user", "--unshare-net", "--dev-bind", "/", "/",
                "--"] + argv

    def test_a_process_under_the_rules_opens_only_what_they_allow(self):
        work = os.path.join(self.tmp, "work")
        os.makedirs(work)
        put(os.path.join(work, "copy.txt"), "copy")
        rules = [("system", p) for p in ("/usr", "/etc", "/proc")] + [("private", work)]
        python = os.path.realpath(sys.executable)
        if not places.within(python, "/usr"):
            rules.append(("system", os.path.dirname(os.path.dirname(python))))
        argv = fence.command(sys.executable, fence.__file__, rules, places.probes(self.home),
                             [sys.executable, "-I", "-c", PROBE, os.path.join(work, "copy.txt"),
                              self.plain] + self.protected)
        done = subprocess.run(self.netless(argv), capture_output=True, text=True, timeout=60)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("proved, with Landlock ABI", done.stderr)
        seen = json.loads(done.stdout)
        self.assertNotIn(SENTINEL, done.stdout)
        self.assertEqual(seen[os.path.join(work, "copy.txt")], "copy")
        for path in [self.plain] + self.protected:
            self.assertTrue(seen[path].startswith("refused"), (path, seen[path]))


if __name__ == "__main__":
    unittest.main()
