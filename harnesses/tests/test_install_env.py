"""env_line from harnesses/lib/install/common.sh, run under a plain `sh` with HOME in a temp dir.

The installer writes YANTRIK_ALLOW_ALL_USERS into Hermes's own .env (hermes.sh), and that file
may hold secrets, so the two things worth pinning down are the file's mode and the value's
silence, next to the editing itself: replace in place, append when absent, never a neighbour
that merely shares a prefix. Needs a real unix filesystem, so CI runs it and Windows does not.
"""

import os
import shutil
import subprocess
import tempfile
import unittest

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
COMMON_SH = "harnesses/lib/install/common.sh"


class EnvLineTests(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="env-line-home-")
        self.addCleanup(shutil.rmtree, self.home, ignore_errors=True)

    def env_line(self, file, key, value):
        env = dict(os.environ)
        env["HOME"] = self.home
        return subprocess.run(
            ["sh", "-c", '. %s; env_line "$1" "$2" "$3"' % COMMON_SH,
             "sh", file, key, value],
            cwd=REPO_ROOT, env=env, capture_output=True, text=True)

    def read(self, file):
        with open(file, "r") as f:
            return f.read()

    def test_new_file_is_created_600_with_the_line(self):
        # In a directory that does not exist yet, as with ~/.hermes on a fresh install.
        target = os.path.join(self.home, ".hermes", ".env")
        result = self.env_line(target, "KEY", "value")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.read(target), "KEY=value\n")
        self.assertEqual(os.stat(target).st_mode & 0o777, 0o600)

    def test_existing_key_replaced_in_place_others_kept_in_order(self):
        target = os.path.join(self.home, ".env")
        with open(target, "w") as f:
            f.write("# a comment\nFIRST=1\nKEY=old\nLAST=3\n")
        result = self.env_line(target, "KEY", "new")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.read(target), "# a comment\nFIRST=1\nKEY=new\nLAST=3\n")

    def test_absent_key_appended_after_a_last_line_without_a_newline(self):
        target = os.path.join(self.home, ".env")
        with open(target, "w") as f:
            f.write("FIRST=1\nOTHER=2")
        result = self.env_line(target, "KEY", "value")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.read(target), "FIRST=1\nOTHER=2\nKEY=value\n")

    def test_a_key_sharing_only_a_prefix_is_untouched(self):
        target = os.path.join(self.home, ".env")
        with open(target, "w") as f:
            f.write("FOOBAR=x\n")
        result = self.env_line(target, "FOO", "1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.read(target), "FOOBAR=x\nFOO=1\n")

    def test_running_twice_leaves_one_line(self):
        target = os.path.join(self.home, ".env")
        self.assertEqual(self.env_line(target, "KEY", "first").returncode, 0)
        result = self.env_line(target, "KEY", "second")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.read(target), "KEY=second\n")

    def test_a_value_of_shell_characters_is_written_literally(self):
        target = os.path.join(self.home, ".env")
        value = "/usr/local/bin & a | b \\ c 'd' \"e\" $f `g` (h)"
        result = self.env_line(target, "KEY", value)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.read(target), "KEY=%s\n" % value)

    def test_the_value_never_appears_on_stdout_or_stderr(self):
        target = os.path.join(self.home, ".env")
        secret = "s3cr3t-value"
        result = self.env_line(target, "KEY", secret)
        self.assertEqual(result.returncode, 0)
        self.assertNotIn(secret, result.stdout)
        self.assertNotIn(secret, result.stderr)


if __name__ == "__main__":
    unittest.main()
