"""The adapter as the desktop meets it: the `.desktop` entry the shell reads, the launcher its
`Exec` runs, the program `X-Yantrik-Adapter` names — started, found, stopped — and the surface
held to the protocol by `yos check` over a real socket.

No LibreOffice here: the program runs without `uno` (so it serves and says LibreOffice cannot be
reached), and the socket test serves the surface over the fake UNO in-process. test_live.py is the
run against a real LibreOffice.
"""

import json
import os
import signal
import stat
import subprocess
import sys
import time
import unittest

import support
from yantrik_libreoffice import APP_ID


def desktop_keys():
    keys = {}
    with open(support.DESKTOP, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if "=" in line and not line.startswith("#"):
                key, value = line.split("=", 1)
                keys[key.strip()] = value.strip()
    return keys


class TestTheDesktopEntry(unittest.TestCase):
    def test_it_declares_the_surface_the_adapter_serves(self):
        keys = desktop_keys()
        self.assertEqual(keys["X-Yantrik-Surface"], APP_ID)
        self.assertEqual(keys["X-Yantrik-Aliases"], "writer;calc")
        self.assertTrue(keys["X-Yantrik-Purpose"])

    def test_it_names_the_program_it_wraps(self):
        # Exec runs the wrapper this package ships, which is installed whether or not
        # LibreOffice is, so only TryExec — naming the wrapped app — hides the entry from a
        # machine that has nothing for it to open (#214).
        self.assertEqual(desktop_keys()["TryExec"], "soffice")

    def test_the_commands_it_names_are_the_ones_this_package_ships(self):
        keys = desktop_keys()
        launcher = keys["Exec"].split()[0]
        adapter = keys["X-Yantrik-Adapter"].split()[0]
        for program in (launcher, adapter):
            path = os.path.join(support.BIN, program)
            self.assertTrue(os.path.isfile(path), "%s names %s, which bin/ does not have"
                            % (support.DESKTOP, program))
            with open(path, encoding="utf-8") as f:
                self.assertTrue(f.readline().startswith("#!"), "%s is run by path" % program)
        self.assertEqual(keys["Exec"], "yantrik-libreoffice %U")


class TestTheLauncher(unittest.TestCase):
    """bin/yantrik-libreoffice, run against a stand-in `soffice` that writes down its arguments."""

    def run_launcher(self, *args, **env):
        machine = support.Machine()
        self.addCleanup(machine.cleanup)
        record = os.path.join(machine.tmp, "argv.json")
        soffice = os.path.join(machine.tmp, "soffice")
        with open(soffice, "w", encoding="utf-8") as f:
            f.write("#!%s\nimport json, sys\njson.dump(sys.argv[1:], open(%r, 'w'))\n"
                    % (sys.executable, record))
        os.chmod(soffice, os.stat(soffice).st_mode | stat.S_IXUSR)
        done = subprocess.run(["sh", os.path.join(support.BIN, "yantrik-libreoffice"), *args],
                              env=machine.env(YANTRIK_SOFFICE=soffice, **env), timeout=30)
        self.assertEqual(done.returncode, 0)
        with open(record, encoding="utf-8") as f:
            return json.load(f)

    def test_it_starts_libreoffice_listening_and_passes_everything_else_through(self):
        self.assertEqual(self.run_launcher("/home/me/a b.odt", "--calc"),
                         ["--accept=pipe,name=yantrik-libreoffice;urp;", "/home/me/a b.odt",
                          "--calc"])

    def test_the_pipe_is_the_one_the_adapter_reads(self):
        self.assertEqual(self.run_launcher(YANTRIK_LIBREOFFICE_PIPE="elsewhere")[0],
                         "--accept=pipe,name=elsewhere;urp;")


class TestTheProgram(unittest.TestCase):
    """bin/yantrik-libreoffice-adapter as the shell starts it, on a private machine, with no
    LibreOffice to reach."""

    def setUp(self):
        self.machine = support.Machine()
        self.addCleanup(self.machine.cleanup)

    def no_uno(self):
        """A directory whose `uno` refuses to import, put first on the program's path.

        So "no LibreOffice to reach" is the same on every machine: a runner that has
        python3-uno installed would otherwise take the other branch, and try a real pipe."""
        hidden = os.path.join(self.machine.tmp, "no-uno")
        os.makedirs(hidden, exist_ok=True)
        with open(os.path.join(hidden, "uno.py"), "w", encoding="utf-8") as f:
            f.write("raise ImportError('hidden by the test: no python3-uno here')\n")
        return hidden

    def start(self, **env):
        # PYTHONFAULTHANDLER: a SIGABRT makes the adapter print every thread's stack, so a stop
        # that hangs (seen once in CI, 5 Oct 2026, never reproduced locally) says where it hung.
        base = self.machine.env(YANTRIK_LIBREOFFICE_GRACE="0.5", PYTHONFAULTHANDLER="1", **env)
        base["PYTHONPATH"] = os.pathsep.join((self.no_uno(), base["PYTHONPATH"]))
        program = subprocess.Popen(
            [sys.executable, os.path.join(support.BIN, "yantrik-libreoffice-adapter")],
            env=base, stderr=subprocess.PIPE, text=True)
        self.addCleanup(self.reap, program)
        socket = self.machine.socket(env.get("YANTRIK_SURFACE", APP_ID))
        deadline = time.monotonic() + 20
        while not os.path.exists(socket):
            if program.poll() is not None:
                self.fail("the adapter exited before it served: %s" % program.stderr.read())
            self.assertLess(time.monotonic(), deadline, "the adapter never bound %s" % socket)
            time.sleep(0.05)
        return program, socket

    @staticmethod
    def reap(program):
        if program.poll() is None:
            program.kill()
        program.communicate()

    def describe(self, socket):
        from yantrik_surface import call_once
        return call_once(socket, "app.describe", {})["result"]

    def stops_cleanly_on_sigterm(self, program, socket):
        # A return code of -15 here is the adapter killed by Python's default SIGTERM action —
        # its handler not yet installed — not a timeout: a timeout raises TimeoutExpired.
        program.send_signal(signal.SIGTERM)
        try:
            _, err = program.communicate(timeout=30)
        except subprocess.TimeoutExpired:
            program.send_signal(signal.SIGABRT)
            _, err = program.communicate(timeout=10)
            self.fail("the adapter did not stop within 30 s of SIGTERM; its threads were:\n%s"
                      % err[-6000:])
        self.assertEqual(program.returncode, 0, err)
        self.assertIn("libreoffice stopped: asked to stop", err)
        self.assertFalse(os.path.exists(socket), "the socket outlived the adapter")

    def test_it_serves_and_says_libreoffice_cannot_be_reached(self):
        program, socket = self.start(YANTRIK_LIBREOFFICE_PIPE="yantrik-test-no-such-pipe")
        described = self.describe(socket)
        self.assertEqual(described["app"], APP_ID)
        self.assertFalse(described["state"]["connected"])
        self.assertEqual(described["summary"], "LibreOffice — no python3-uno to reach it with")
        self.stops_cleanly_on_sigterm(program, socket)

    def test_a_stop_sent_the_moment_the_socket_appears_is_a_clean_stop(self):
        # The shell may stop an adapter as soon as it sees it serve. The socket is that sign, so
        # the stop handler is in place before the socket exists — this is the race the test
        # above used to lose on a loaded runner, with nothing in between to hide it.
        for _ in range(3):
            program, socket = self.start(YANTRIK_LIBREOFFICE_PIPE="yantrik-test-no-such-pipe")
            self.stops_cleanly_on_sigterm(program, socket)

    def test_it_binds_the_id_the_shell_gives_it(self):
        _, socket = self.start(YANTRIK_SURFACE="office-test")
        self.assertEqual(self.describe(socket)["app"], "office-test")

    def test_it_stops_on_its_own_when_the_app_it_serves_is_gone(self):
        app = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
        self.addCleanup(self.reap, app)
        program, socket = self.start(YANTRIK_APP_PID=str(app.pid),
                                     YANTRIK_LIBREOFFICE_PIPE="yantrik-test-no-such-pipe")
        time.sleep(1.5)
        self.assertIsNone(program.poll(), "it waits while the app is there")
        app.kill()
        app.wait()
        _, err = program.communicate(timeout=20)
        self.assertEqual(program.returncode, 0, err)
        self.assertIn("serving pid %d" % app.pid, err)
        self.assertIn("libreoffice stopped: LibreOffice has gone", err)
        self.assertFalse(os.path.exists(socket))

    def test_a_second_adapter_does_not_take_the_first_ones_name(self):
        self.start()
        second = subprocess.run(
            [sys.executable, os.path.join(support.BIN, "yantrik-libreoffice-adapter")],
            env=self.machine.env(), capture_output=True, text=True, timeout=30)
        self.assertEqual(second.returncode, 1)
        self.assertIn("another instance owns", second.stderr)

    def test_a_name_that_is_not_a_surface_name_is_refused(self):
        done = subprocess.run(
            [sys.executable, os.path.join(support.BIN, "yantrik-libreoffice-adapter")],
            env=self.machine.env(YANTRIK_SURFACE="Libre Office"), capture_output=True, text=True,
            timeout=30)
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("is not a surface name", done.stderr)

    def test_headless_without_libreoffice_says_so(self):
        done = subprocess.run(
            [sys.executable, os.path.join(support.BIN, "yantrik-libreoffice-adapter"),
             "--headless", "--soffice", "/nonexistent/soffice"],
            env=self.machine.env(), capture_output=True, text=True, timeout=30)
        self.assertEqual(done.returncode, 1)
        self.assertIn("--headless needs LibreOffice", done.stderr)


@unittest.skipUnless(os.path.isfile(support.YOS), "deploy/yantrik-os/yos is not in this tree")
class TestOverTheSocket(support.Case):
    """The surface over the fake LibreOffice, served on a real socket and driven by the OS's own
    client: `yos check` finds nothing wrong, and `yos act` reads and writes as a mind would."""

    def setUp(self):
        super().setUp()
        self.machine.set_mode("ask")
        for key, value in (("HOME", self.machine.home), ("XDG_RUNTIME_DIR", self.machine.runtime)):
            saved = os.environ.get(key)
            os.environ[key] = value
            self.addCleanup(lambda k=key, v=saved: os.environ.pop(k) if v is None else
                            os.environ.__setitem__(k, v))
        self.budget = self.calc()
        self.act("open", path=self.writer())
        self.act("open", path=self.budget)
        self.surface.serve_in_thread()
        self.addCleanup(self.surface.stop)

    def yos(self, *args):
        return subprocess.run([sys.executable, support.YOS, *args], env=self.machine.env(),
                              capture_output=True, text=True, timeout=60)

    def test_yos_check_finds_nothing_wrong(self):
        done = self.yos("check", APP_ID, "--json")
        report = json.loads(done.stdout)
        rows = report["surfaces"][0]["checks"]
        self.assertEqual((done.returncode, [r for r in rows if r["status"] == "fail"]), (0, []))
        passed = {r["check"] for r in rows if r["status"] == "pass"}
        for check in ("schema", "grades", "params", "secrets", "revision", "steady", "unknown",
                      "missing", "undeclared", "types", "stale"):
            self.assertIn(check, passed, rows)

    def test_yos_act_reads_and_writes_cells_as_a_mind_would(self):
        out = self.yos("act", APP_ID, "read_cells", "document=budget.ods", "range=A2:B3").stdout
        self.assertIn('"Rent"', out)
        done = self.yos("act", APP_ID, "write_cells", "document=budget.ods", 'cells={"B2": 1300}')
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(self.result("read_cells", document="budget.ods", range="B2")["rows"],
                         [[1300]])
        # `save` is sensitive and this machine is in ask mode: without the person, nothing is saved.
        done = self.yos("act", APP_ID, "save", "document=budget.ods", "--no-ask")
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("GRANT: libreoffice.save is graded `sensitive`", done.stderr)
        self.assertEqual(self.on_disk(self.budget)["sheets"][0]["cells"]["B2"], 1200)


if __name__ == "__main__":
    unittest.main()
