"""The Hermes launcher heal, driven through the real shell function.

The installer's own temp Python is gone after a /tmp cleanup, so a launcher that still names it
dies with `python3: not found`. `heal_hermes_launcher` repoints it to the persistent copy under
`~/.hermes/tools`. Everything here is offline and stdlib-only.
"""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import support

LAUNCHER = support.HARNESSES / "lib" / "install" / "hermes-launcher.sh"
HERMES_SH = support.HARNESSES / "lib" / "install" / "hermes.sh"


def heal(home):
    return subprocess.run(
        ["sh", "-c", '. "$1"; heal_hermes_launcher "$2"', "sh", str(LAUNCHER), str(home)],
        capture_output=True, text=True,
    )


@unittest.skipUnless(shutil.which("sh"), "the heal is a POSIX sh function")
class HermesLauncherTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.home = Path(self.tmp.name)
        self.bin = self.home / ".hermes" / "hermes-agent" / ".hermes" / "bin"
        self.tools = self.home / ".hermes" / "tools"
        self.bin.mkdir(parents=True)
        self.tools.mkdir(parents=True)

    def make_python(self):
        py = self.tools / "python-3.14.7+x" / "bin" / "python3"
        py.parent.mkdir(parents=True)
        py.write_text("#!/bin/sh\necho ok\n")
        py.chmod(0o755)
        return py

    def make_launcher(self, interpreter):
        launcher = self.bin / "hermes"
        launcher.write_text(
            "#!/bin/sh\nexec %s -I -c 'print(1)'\n" % interpreter
        )
        launcher.chmod(0o755)
        return launcher

    def test_repairs_a_tmp_interpreter(self):
        self.make_python()
        launcher = self.make_launcher("/tmp/tmp.AbC123/tools/python-3.14.7+x/bin/python3")
        result = heal(self.home)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("repointed", result.stdout)
        for path in self.bin.rglob("*"):
            if path.is_file() and not path.name.endswith(".before-tmp-fix"):
                self.assertNotIn("/tmp/tmp.AbC123/tools/", path.read_text())
        self.assertEqual(launcher.stat().st_mode & 0o777, 0o755)
        self.assertIn("-I -c 'print(1)'", launcher.read_text())
        self.assertEqual(
            subprocess.run([str(launcher)], capture_output=True, text=True).stdout,
            "ok\n",
        )
        backup = Path(str(launcher) + ".before-tmp-fix")
        self.assertIn("/tmp/tmp.AbC123/tools/", backup.read_text())

    def test_idempotent(self):
        self.make_python()
        launcher = self.make_launcher("/tmp/tmp.AbC123/tools/python-3.14.7+x/bin/python3")
        heal(self.home)
        before = launcher.read_text()
        result = heal(self.home)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(launcher.read_text(), before)

    def test_healthy_launcher_untouched(self):
        self.make_python()
        launcher = self.make_launcher(
            str(self.home / ".hermes" / "tools" / "python-3.14.7+x" / "bin" / "python3")
        )
        before = launcher.read_bytes()
        result = heal(self.home)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(launcher.read_bytes(), before)

    def test_missing_target_left_alone(self):
        launcher = self.make_launcher("/tmp/tmp.AbC123/tools/python-3.14.7+x/bin/python3")
        before = launcher.read_text()
        result = heal(self.home)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("hermes", result.stdout)
        self.assertEqual(launcher.read_text(), before)

    def test_dotdot_interpreter_is_not_followed_out_of_tools(self):
        outside = self.home / "elsewhere" / "python3"
        outside.parent.mkdir(parents=True)
        outside.write_text("#!/bin/sh\necho escaped\n")
        outside.chmod(0o755)
        launcher = self.make_launcher("/tmp/tmp.AbC123/tools/../../elsewhere/python3")
        before = launcher.read_text()
        result = heal(self.home)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(launcher.read_text(), before)

    def test_unrelated_tmp_mention_not_rewritten(self):
        self.make_python()
        launcher = self.bin / "hermes"
        launcher.write_text("#!/bin/sh\n# see /tmp/foo.log\n")
        launcher.chmod(0o755)
        before = launcher.read_text()
        result = heal(self.home)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(launcher.read_text(), before)

    def test_no_bin_dir_is_silent(self):
        shutil.rmtree(self.bin)
        result = heal(self.home)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_hermes_sh_sources_and_orders(self):
        text = HERMES_SH.read_text()
        self.assertIn('hermes-launcher.sh', text)
        self.assertIn('heal_hermes_launcher "$HOME"', text)
        heal_line = text.index('heal_hermes_launcher "$HOME"')
        version_line = text.index("hermes --version")
        self.assertLess(heal_line, version_line)


if __name__ == "__main__":
    unittest.main()
