"""The sandbox an agent's LibreOffice runs in (sandbox.py, agent_office.py; #953).

The spec tests read the bwrap command and run anywhere. The kernel tests need bwrap able to make
user namespaces here, and skip with the reason where it cannot; each runs a stand-in for soffice
through the very command `AgentOffice` starts soffice with, and the stand-in tries every way out
the reviews of #942 found. Each has an unsandboxed control that must get out the same ways, so a
probe that proves nothing fails instead of passing.
"""

import json
import os
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import unittest

import support
from test_fence import SENTINEL, bwrap_missing, put
from yantrik_libreoffice import agent_office, places, sandbox
from yantrik_libreoffice.canaries import Canaries


def binds(argv):
    """(kind, source, inside) for every bind in a bwrap argv."""
    out, i = [], 0
    while i < len(argv):
        if argv[i] in ("--ro-bind", "--bind", "--dev-bind", "--ro-bind-try", "--bind-try"):
            out.append((argv[i], argv[i + 1], argv[i + 2]))
            i += 3
        else:
            i += 1
    return out


class Machine(unittest.TestCase):
    """A home with a key at the top and one nested in an ordinary folder, a runtime folder, and
    the adapter's private folder."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="yantrik-sandbox-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.home = os.path.join(self.tmp, "home")
        put(os.path.join(self.home, ".ssh", "id_ed25519"))
        put(os.path.join(self.home, "Documents", "old", ".ssh", "k.csv"))
        self.private = sandbox.Private(os.path.join(self.tmp, "private"))
        os.makedirs(self.private.root)
        self.private.make()
        self.runtime = os.path.join(self.tmp, "run")
        os.makedirs(self.runtime)
        self.env = {"HOME": self.home, "XDG_RUNTIME_DIR": self.runtime}


class Spec(Machine):
    def command(self, root=None, hidden=None):
        return sandbox.command("/usr/bin/bwrap", self.private, root, "/opt/a/fence.py",
                               hidden if hidden is not None else places.hidden(self.home, self.env),
                               ["/usr/bin/python3", "-I", sandbox.LAUNCHER, "--", "soffice"])

    def test_every_namespace_is_its_own_and_nothing_outlives_the_adapter(self):
        argv = self.command()
        for flag in sandbox.NAMESPACES + ("--die-with-parent", "--new-session"):
            self.assertIn(flag, argv)
        self.assertEqual(argv[argv.index("--cap-drop") + 1], "ALL")
        self.assertIn("--unshare-net", argv, "no network (#942 M1, L2)")
        tail = argv[argv.index("--remount-ro"):]
        self.assertEqual(tail[:2], ["--remount-ro", "/"], "the root is read-only once built")
        self.assertEqual(argv[argv.index("--") + 1:], ["/usr/bin/python3", "-I", sandbox.LAUNCHER,
                                                       "--", "soffice"])

    def test_nothing_of_the_home_the_runtime_folder_or_the_real_tmp_is_shown(self):
        argv = self.command()
        shown = binds(argv)
        for _, source, inside in shown:
            for place in (self.home, self.runtime, "/run", "/home", "/root"):
                self.assertFalse(places.within(source, place), (source, place))
            if places.within(source, "/tmp"):
                self.assertTrue(places.within(source, self.private.root), source)
        inside = {i: s for _, s, i in shown}
        self.assertEqual(inside["/tmp"], self.private.pipes, "/tmp holds only our pipes")
        self.assertEqual(inside[sandbox.WORK], self.private.work)
        self.assertEqual(inside[sandbox.PROFILE], self.private.profile)
        self.assertEqual(inside[sandbox.TMP], self.private.tmp)
        self.assertEqual(inside[sandbox.LAUNCHER], "/opt/a/fence.py")
        self.assertEqual([k for k, _, i in shown if i == sandbox.LAUNCHER], ["--ro-bind"])
        self.assertIn(sandbox.HOME, argv[argv.index("--tmpfs") + 1:], "an empty home of its own")

    def test_system_folders_are_shown_read_only(self):
        for kind, source, inside in binds(self.command()):
            if inside in (sandbox.WORK, sandbox.PROFILE, sandbox.TMP, "/tmp"):
                self.assertEqual(kind, "--bind")
            else:
                self.assertIn(kind, ("--ro-bind",), (kind, source))
        etc = [i for _, _, i in binds(self.command()) if i.startswith("/etc")]
        self.assertTrue(all(i[len("/etc/"):].split("/")[0] in sandbox.ETC for i in etc), etc)

    def test_a_link_out_of_the_install_is_shown_and_one_into_the_home_is_not(self):
        """#942 M2: Debian's share/uno_packages/cache is a link to /var/spool/libreoffice."""
        root = os.path.join(self.tmp, "opt", "libreoffice")
        os.makedirs(os.path.join(root, "program"))
        os.symlink("/etc/passwd", os.path.join(root, "program", "passwd-link"))
        os.symlink(os.path.join(self.home, ".ssh"), os.path.join(root, "keys"))
        os.symlink(os.path.join(self.home, "Documents"), os.path.join(root, "docs"))
        os.symlink("/usr/bin", os.path.join(root, "bin"))
        found = sandbox.linked_targets(root, places.hidden(self.home, self.env))
        self.assertEqual(found, ["/etc/passwd"])

    def test_a_link_to_a_whole_top_folder_or_one_just_below_it_is_not_shown(self):
        """#962 L3: never all of /var or /opt, nor /var/lib, for one link in the install."""
        root = os.path.join(self.tmp, "opt", "libreoffice")
        os.makedirs(root)
        for name, target in (("var", "/var"), ("lib", "/var/lib"), ("opt", "/opt"),
                             ("etc", "/etc"), ("fonts", "/var/cache/fontconfig")):
            os.symlink(target, os.path.join(root, name))
        found = sandbox.linked_targets(root, [])
        self.assertNotIn("/var", found)
        self.assertNotIn("/var/lib", found)
        self.assertNotIn("/opt", found)
        self.assertNotIn("/etc", found)
        if os.path.isdir("/var/cache/fontconfig"):
            self.assertEqual(found, ["/var/cache/fontconfig"])

    def test_the_sandbox_makes_no_user_namespaces_and_no_kind_of_place_makes_links(self):
        """#962 L2, and the second wall of H1 and H2."""
        self.assertIn("--disable-userns", self.command())
        for kind, rights in agent_office.fence.KINDS.items():
            self.assertFalse(rights & agent_office.fence.MAKE_SYM, kind)

    def test_an_old_bwrap_is_refused_with_its_reason(self):
        done = subprocess.CompletedProcess([], 1, b"",
                                           b"bwrap: Unknown option --disable-userns\n")
        with self.assertRaises(sandbox.SandboxError) as caught:
            sandbox.check("/usr/bin/bwrap", run=lambda *a, **k: done)
        self.assertIn("older than 0.8", str(caught.exception))

    def test_too_large_an_install_is_refused_not_skimmed(self):
        root = os.path.join(self.tmp, "big")
        os.makedirs(root)
        for i in range(5):
            put(os.path.join(root, "f%d" % i), "x")
        with self.assertRaises(sandbox.SandboxError):
            sandbox.linked_targets(root, [], walked=3)

    def test_a_hidden_place_inside_a_shown_folder_is_covered_again(self):
        """#942 M3: a protected name nested in a folder that is shown."""
        shown = os.path.join(self.home, "Documents")
        covers = sandbox.masks([(shown, "/inside")],
                               [os.path.join(shown, "old", ".ssh"),
                                os.path.join(shown, "old", ".ssh", "k.csv"),
                                os.path.join(self.tmp, "elsewhere")])
        self.assertEqual(covers, ["--tmpfs", "/inside/old/.ssh",
                                  "--ro-bind", "/dev/null", "/inside/old/.ssh/k.csv"])

    def test_the_environment_carries_no_bus_display_runtime_or_preload(self):
        env = sandbox.environment({
            "DBUS_SESSION_BUS_ADDRESS": "unix:path=/run/user/1000/bus", "DISPLAY": ":0",
            "WAYLAND_DISPLAY": "wayland-0", "XDG_RUNTIME_DIR": "/run/user/1000",
            "LD_PRELOAD": "/x.so", "HOME": "/home/p", "LANG": "de_DE.UTF-8",
            "LC_ALL": "$(touch /tmp/x)", "YANTRIK_AGENT_TOKEN": "tok"})
        for name in ("DBUS_SESSION_BUS_ADDRESS", "DISPLAY", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR",
                     "LD_PRELOAD", "YANTRIK_AGENT_TOKEN", "LC_ALL"):
            self.assertNotIn(name, env)
        self.assertEqual((env["HOME"], env["TMPDIR"], env["LANG"]),
                         (sandbox.HOME, sandbox.TMP, "de_DE.UTF-8"))

    def test_landlock_inside_allows_the_system_read_only_and_only_the_private_folders(self):
        rules = dict((p, k) for k, p in sandbox.inner_rules(None))
        self.assertEqual(rules["/usr"], "system")
        self.assertEqual(rules["/tmp"], "sockets")
        for path in (sandbox.WORK, sandbox.PROFILE, sandbox.TMP, sandbox.HOME):
            self.assertEqual(rules[path], "private")
        self.assertNotIn("/home", rules)
        self.assertEqual(dict((p, k) for k, p in sandbox.inner_rules("/opt/lo"))["/opt/lo"],
                         "system")

    @unittest.skipIf(os.getuid() == 0, "as root, the stand-in bwrap is root's own")
    def test_only_roots_bwrap_is_used(self):
        mine = put(os.path.join(self.tmp, "bwrap"), "#!/bin/sh\n")
        os.chmod(mine, 0o755)
        with self.assertRaises(sandbox.SandboxError) as caught:
            sandbox.find_bwrap((mine, os.path.join(self.tmp, "none")))
        self.assertIn("not installed", str(caught.exception))

    def test_the_check_refuses_when_user_namespaces_are_off(self):
        done = subprocess.CompletedProcess(
            [], 1, b"", b"bwrap: setting up uid map: Permission denied\n")
        with self.assertRaises(sandbox.SandboxError) as caught:
            sandbox.check("/usr/bin/bwrap", run=lambda *a, **k: done)
        self.assertIn("uid map: Permission denied", str(caught.exception))
        self.assertIn("user namespaces", str(caught.exception))


class TestNoSandboxNoAgentWork(unittest.TestCase):
    """Fail closed: where the sandbox cannot be built, nothing is started."""

    def test_without_bwrap_an_agent_is_refused_and_nothing_starts(self):
        office = agent_office.AgentOffice(home=tempfile.gettempdir(), bwrap=("/nonexistent/bwrap",))
        with support.mock.patch("subprocess.Popen") as popen:
            for start in (False, True):
                with self.assertRaises(support.OfficeError) as caught:
                    office.workspace(start=start)
                said = str(caught.exception)
                self.assertTrue(said.startswith(agent_office.CLOSED_TO_AGENTS), said)
                self.assertIn("bwrap", said)
        popen.assert_not_called()

    def test_when_the_namespaces_cannot_be_made_an_agent_is_refused(self):
        office = agent_office.AgentOffice(home=tempfile.gettempdir())
        with support.mock.patch.object(sandbox, "find_bwrap", return_value="/usr/bin/bwrap"), \
                support.mock.patch.object(sandbox, "check",
                                          side_effect=sandbox.SandboxError("userns off")), \
                support.mock.patch("subprocess.Popen") as popen:
            with self.assertRaises(support.OfficeError) as caught:
                office.open(os.path.join(tempfile.gettempdir(), "x.odt"))
        self.assertTrue(str(caught.exception).startswith(agent_office.CLOSED_TO_AGENTS))
        popen.assert_not_called()


# Run as the stand-in for soffice: every way out the reviews found, tried, as JSON on stdout.
ESCAPE = r"""
import json, os, socket, sys
cfg = json.loads(sys.argv[1])
out = {}
def attempt(name, fn):
    try:
        got = fn()
        out[name] = "reached: %s" % (got,)
    except OSError as e:
        out[name] = "refused: %s" % (e.strerror or e)
def read(path):
    with open(path) as f:
        return f.read()
def connect(family, address, kind=socket.SOCK_STREAM):
    s = socket.socket(family, kind)
    s.settimeout(2)
    try:
        s.connect(address)
        if kind == socket.SOCK_STREAM and family == socket.AF_UNIX:
            s.settimeout(2)
            return s.recv(100).decode()
        return "connected"
    finally:
        s.close()
for path in cfg["files"]:
    attempt("open " + path, lambda p=path: read(p))
attempt("list home", lambda: os.listdir(cfg["home"]))
for path in cfg["sockets"]:
    attempt("connect " + path, lambda p=path: connect(socket.AF_UNIX, p))
attempt("abstract", lambda: connect(socket.AF_UNIX, "\0" + cfg["abstract"]))
attempt("tcp", lambda: connect(socket.AF_INET, ("127.0.0.1", cfg["port"])))
attempt("udp out", lambda: connect(socket.AF_INET, ("192.0.2.1", 9), socket.SOCK_DGRAM))
attempt("write root", lambda: open("/escaped", "w").close())
try:
    out["proc/net/unix"] = read("/proc/net/unix")
except OSError as e:
    out["proc/net/unix"] = "refused: %s" % e.strerror
out["processes"] = len([p for p in os.listdir("/proc") if p.isdigit()])
out["env"] = sorted(k for k in os.environ if k in ("DBUS_SESSION_BUS_ADDRESS", "DISPLAY",
                    "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"))
print(json.dumps(out))
"""


class Listening:
    """A listener that hands every connection the sentinel: the person's LibreOffice pipe, the
    session bus, an abstract socket of the display's kind, or a TCP service on the host."""

    def __init__(self, test, family, address):
        import threading
        self.sock = socket.socket(family, socket.SOCK_STREAM)
        self.sock.bind(address)
        self.sock.listen(8)
        test.addCleanup(self.close)
        threading.Thread(target=self._serve, daemon=True).start()

    def close(self):
        try:
            self.sock.shutdown(socket.SHUT_RDWR)   # wakes the accept, which frees the address
        except OSError:
            pass
        self.sock.close()

    def _serve(self):
        while True:
            try:
                conn, _ = self.sock.accept()
            except OSError:
                return
            try:
                conn.sendall(SENTINEL.encode())
            except OSError:   # a client that only knocked and left
                pass
            finally:
                conn.close()


@unittest.skipUnless(sys.platform.startswith("linux"), "bwrap is Linux's")
class TestTheKernelSandbox(Machine):
    def setUp(self):
        missing = bwrap_missing()
        if missing:
            self.skipTest("bwrap cannot make a sandbox here (%s); agents are refused LibreOffice "
                          "on such a machine, which TestNoSandboxNoAgentWork covers" % missing)
        super().setUp()
        self.files = [os.path.join(self.home, ".ssh", "id_ed25519"),
                      os.path.join(self.home, "Documents", "old", ".ssh", "k.csv"),
                      put(os.path.join(self.home, "Documents", "plain.txt"))]
        self.person_pipe = "/tmp/OSL_PIPE_%d_yantrik-sandbox-test-%s" % (os.getuid(),
                                                                         secrets.token_hex(6))
        self.addCleanup(lambda: os.path.exists(self.person_pipe) and os.remove(self.person_pipe))
        Listening(self, socket.AF_UNIX, self.person_pipe)
        self.bus = os.path.join(self.runtime, "bus")
        Listening(self, socket.AF_UNIX, self.bus)
        self.abstract = "yantrik-sandbox-test-%s" % secrets.token_hex(6)
        Listening(self, socket.AF_UNIX, "\0" + self.abstract)
        tcp = Listening(self, socket.AF_INET, ("127.0.0.1", 0))
        self.port = tcp.sock.getsockname()[1]
        self.config = json.dumps({"files": self.files, "home": self.home,
                                  "sockets": [self.person_pipe, self.bus],
                                  "abstract": self.abstract, "port": self.port})

    def escape(self, wrap):
        argv = wrap([sandbox.PYTHON, "-I", "-c", ESCAPE, self.config])
        done = subprocess.run(argv, capture_output=True, text=True, timeout=60,
                              env=sandbox.environment(self.env))
        return done

    def sandboxed(self, argv, canaries=None):
        canaries = canaries or Canaries(self.private.root).open()
        self.addCleanup(canaries.close)
        return agent_office.sandboxed(sandbox.find_bwrap(), self.private, None, self.home,
                                      self.env, canaries, argv)

    def test_the_control_gets_out_every_way_so_the_probe_means_something(self):
        done = self.escape(lambda argv: argv)
        seen = json.loads(done.stdout)
        for path in self.files:
            self.assertEqual(seen["open " + path], "reached: " + SENTINEL, path)
        for name in ("connect " + self.person_pipe, "connect " + self.bus, "abstract"):
            self.assertEqual(seen[name], "reached: " + SENTINEL, name)
        self.assertTrue(seen["tcp"].startswith("reached"), seen["tcp"])

    def test_inside_the_sandbox_every_way_out_is_shut(self):
        done = self.escape(self.sandboxed)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("yantrik-libreoffice-sandbox: proved", done.stderr)
        self.assertNotIn(SENTINEL, done.stdout + done.stderr)
        seen = json.loads(done.stdout)
        for name, said in seen.items():
            if name not in ("proc/net/unix", "processes", "env"):
                self.assertTrue(said.startswith("refused"), (name, said))
        self.assertNotIn(os.path.basename(self.person_pipe), seen["proc/net/unix"],
                         "the person's pipe is not even named in the sandbox (#942 M1)")
        self.assertNotIn(self.abstract, seen["proc/net/unix"])
        self.assertLessEqual(seen["processes"], 4, "a PID namespace of its own")
        self.assertEqual(seen["env"], [])

    def test_with_the_namespaces_removed_the_proof_refuses_and_nothing_runs(self):
        """The L1 lesson of #942: take the sandbox away and the start must fail, not pass."""
        marker = os.path.join(self.tmp, "ran")
        argv = self.sandboxed(["/bin/sh", "-c", "echo ran > %s" % marker])
        inner = argv[argv.index("--") + 1:]
        inner = [a.replace(sandbox.LAUNCHER, agent_office.LAUNCHER) for a in inner]
        done = subprocess.run(inner, capture_output=True, text=True, timeout=60)
        self.assertEqual(done.returncode, agent_office.fence.REFUSED, done.stderr)
        self.assertIn("refused: the sandbox did not hold", done.stderr)
        self.assertFalse(os.path.exists(marker))

    def test_with_the_network_namespace_removed_the_proof_refuses(self):
        marker = os.path.join(self.tmp, "ran")
        argv = self.sandboxed(["/bin/sh", "-c", "echo ran > /yantrik/work/ran"])
        argv.remove("--unshare-net")
        done = subprocess.run(argv, capture_output=True, text=True, timeout=60,
                              env=sandbox.environment(self.env))
        self.assertEqual(done.returncode, agent_office.fence.REFUSED, done.stderr)
        self.assertRegex(done.stderr, r"did not hold: (it has network interfaces|TCP port|"
                                      r"the socket @)")
        self.assertFalse(os.path.exists(os.path.join(self.private.work, "ran")))
        self.assertFalse(os.path.exists(marker))

    def test_the_agent_office_launch_refuses_a_soffice_that_cannot_answer(self):
        """AgentOffice's own start, end to end, with a stand-in soffice that exits: refused with a
        sentence, nothing left behind, and nothing it tried got out."""
        program = put(os.path.join(self.tmp, "opt", "lo", "program", "soffice"),
                      "#!/bin/sh\ncat %s\nexit 3\n" % self.files[0])
        os.chmod(program, 0o755)
        office = agent_office.AgentOffice(soffice=program, home=self.home, env=self.env,
                                          patience=20)
        with self.assertRaises(support.OfficeError) as caught:
            office.workspace(start=True)
        said = str(caught.exception)
        self.assertIn("would not start inside its sandbox", said)
        self.assertNotIn(SENTINEL, said)
        self.assertIsNone(office._private)  # noqa: SLF001 - its folder is gone


if __name__ == "__main__":
    unittest.main()
