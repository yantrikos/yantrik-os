"""The Hermes installer (harnesses/lib/install/hermes.sh) against a fake Hermes in a fake home.

Hermes's own installer needs the network, so it is not run here. What is run is everything after
it, on the home it leaves behind (#712): a launcher naming the Python in the installer's temporary
directory, and the same Python kept in ~/.hermes/tools. The installer has to leave a launcher that
still works once that temporary directory is gone, which is what /tmp cleanup did to the gateway.
"""

import os
import re
import shutil
import stat
import subprocess
import tempfile
import unittest

import support

INSTALL = support.HARNESSES / "lib" / "install"
PLUGIN = support.HARNESSES / "hermes"
PYTHON = "python-3.14.7+20260901-linux-x64"
# A path that starts at /tmp; /var/tmp/... is not one (the same test the repair itself makes).
UNDER_TMP = re.compile(r"(?:^|[^\w./-])/tmp/")

FAKE_PYTHON = """#!/bin/sh
# Stands in for Hermes's Python: `-I -c <hermes>` and then the hermes arguments.
shift 3
echo "$*" >> "$HOME/hermes-calls"
case "$1" in --version) echo "Hermes Agent v0.0.0 (fake)" ;; esac
"""


def _executable(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)
    os.chmod(path, 0o755)


@unittest.skipUnless(shutil.which("sh") and os.path.isdir("/tmp"), "needs a POSIX shell and /tmp")
class HermesLauncherTests(unittest.TestCase):
    def setUp(self):
        # The home is not under /tmp: one there would name /tmp after the repair as well.
        try:
            self.home = tempfile.mkdtemp(prefix="yantrik-hermes-home-", dir="/var/tmp")
        except OSError:
            self.skipTest("/var/tmp is not writable here")
        self.addCleanup(shutil.rmtree, self.home, True)
        # Where Hermes's installer put its Python while it worked: mktemp -d, so /tmp/tmp.XXXX.
        self.installer_tmp = tempfile.mkdtemp(prefix="tmp.", dir="/tmp")
        self.addCleanup(shutil.rmtree, self.installer_tmp, True)

        self.bin = os.path.join(self.home, ".hermes", "hermes-agent", ".hermes", "bin")
        self.launcher = os.path.join(self.bin, "hermes")
        temp_python = os.path.join(self.installer_tmp, "tools", PYTHON, "bin", "python3")
        _executable(temp_python, FAKE_PYTHON)
        self.kept = os.path.join(self.home, ".hermes", "tools", PYTHON, "bin", "python3.14")
        _executable(self.kept, FAKE_PYTHON)
        # The launcher as Hermes's installer writes it, and the wrapper it puts in ~/.local/bin.
        _executable(self.launcher, "#!/bin/sh\nexec %s -I -c 'from hermes_cli.main import main; main()' \"$@\"\n" % temp_python)
        self.wrapper = os.path.join(self.home, ".local", "bin", "hermes")
        _executable(self.wrapper, '#!/bin/sh\nexec "%s" "$@"\n' % self.launcher)

        # systemctl, which the installer restarts the gateway with.
        self.fakes = os.path.join(self.home, "fakes")
        _executable(os.path.join(self.fakes, "systemctl"), '#!/bin/sh\necho "systemctl $*" >> "$HOME/hermes-calls"\n')
        self.env = {"HOME": self.home, "PATH": "%s:/usr/bin:/bin" % self.fakes, "LC_ALL": "C"}

    def install(self):
        return subprocess.run(["sh", str(INSTALL / "hermes.sh"), str(PLUGIN)], env=self.env,
                              capture_output=True, text=True, timeout=60)

    def under_tmp(self):
        named = []
        for name in os.listdir(self.bin):
            with open(os.path.join(self.bin, name)) as f:
                if UNDER_TMP.search(f.read()):
                    named.append(name)
        return named

    def test_the_installer_leaves_a_launcher_that_outlives_its_temporary_directory(self):
        run = self.install()
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn("Hermes Agent v0.0.0 (fake) is here", run.stdout)

        shutil.rmtree(self.installer_tmp)
        self.assertEqual(self.under_tmp(), [], "a file in the launcher directory still names /tmp")
        with open(self.launcher) as f:
            self.assertIn(self.kept, f.read())
        self.assertTrue(os.stat(self.launcher).st_mode & stat.S_IXUSR)
        version = subprocess.run([self.wrapper, "--version"], env=self.env,
                                 capture_output=True, text=True, timeout=30)
        self.assertEqual((version.returncode, version.stdout.strip()), (0, "Hermes Agent v0.0.0 (fake)"))

    def test_with_no_python_to_point_at_it_fails_loudly_and_leaves_the_launcher_alone(self):
        os.remove(self.kept)
        with open(self.launcher) as f:
            before = f.read()
        run = self.install()
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("install failed:", run.stderr)
        self.assertIn("there is no", run.stderr)
        with open(self.launcher) as f:
            self.assertEqual(f.read(), before)
        calls = os.path.join(self.home, "hermes-calls")
        if os.path.exists(calls):
            with open(calls) as f:
                self.assertNotIn("systemctl", f.read(), "the gateway was restarted on a launcher that cannot last")

    def test_installing_again_changes_nothing(self):
        self.assertEqual(self.install().returncode, 0)
        with open(self.launcher) as f:
            first = f.read()
        again = self.install()
        self.assertEqual(again.returncode, 0, again.stdout + again.stderr)
        self.assertNotIn("now runs the Python", again.stdout)
        with open(self.launcher) as f:
            self.assertEqual(f.read(), first)


if __name__ == "__main__":
    unittest.main()
