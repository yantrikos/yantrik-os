"""Hermes streaming enable, driven through the real shell function.

Hermes's config ships with a top-level `streaming:` block whose `enabled:` is `false`, so the
gateway hands the desktop only the finished answer. `enable_hermes_streaming` flips that one key
to `true` and leaves every other line byte-for-byte. Everything here is offline and stdlib-only.
"""

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import support

STREAMING = support.HARNESSES / "lib" / "install" / "hermes-streaming.sh"
HERMES_SH = support.HARNESSES / "lib" / "install" / "hermes.sh"


def enable(home):
    return subprocess.run(
        ["sh", "-c", '. "$1"; enable_hermes_streaming "$2"', "sh", str(STREAMING), str(home)],
        capture_output=True, text=True,
    )


@unittest.skipUnless(shutil.which("sh"), "the enable is a POSIX sh function")
class HermesStreamingTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.home = Path(self.tmp.name)
        self.cfg = self.home / ".hermes" / "config.yaml"
        self.cfg.parent.mkdir(parents=True)

    def test_false_becomes_true_otherwise_identical(self):
        before = (
            "model: default\n"
            "streaming:\n"
            "  enabled: false\n"
            "  transport: edit\n"
            "display:\n"
            "  streaming: false\n"
        )
        self.cfg.write_text(before)
        result = enable(self.home)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("streams", result.stdout)
        after = self.cfg.read_text()
        self.assertEqual(
            after,
            before.replace("  enabled: false\n", "  enabled: true\n", 1),
        )

    def test_display_streaming_left_alone(self):
        before = (
            "display:\n"
            "  streaming: false\n"
            "streaming:\n"
            "  enabled: false\n"
        )
        self.cfg.write_text(before)
        result = enable(self.home)
        self.assertEqual(result.returncode, 0, result.stderr)
        after = self.cfg.read_text()
        self.assertIn("  streaming: false\n", after)
        self.assertIn("  enabled: true\n", after)

    def test_missing_block_appended(self):
        before = "model: default\n"
        self.cfg.write_text(before)
        result = enable(self.home)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("streams", result.stdout)
        after = self.cfg.read_text()
        self.assertEqual(after, before + "streaming:\n  enabled: true\n")

    def test_already_true_changes_nothing(self):
        before = "streaming:\n  enabled: true\n"
        self.cfg.write_text(before)
        result = enable(self.home)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(self.cfg.read_text(), before)

    def test_missing_file_returns_zero(self):
        result = enable(self.home)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_backup_written_once(self):
        self.cfg.write_text("streaming:\n  enabled: false\n")
        enable(self.home)
        backup = Path(str(self.cfg) + ".before-streaming")
        self.assertTrue(backup.exists())
        self.assertIn("enabled: false", backup.read_text())
        backup.write_text("do not touch\n")
        enable(self.home)
        self.assertEqual(backup.read_text(), "do not touch\n")

    def test_hermes_sh_calls_before_gateway_restart(self):
        text = HERMES_SH.read_text()
        self.assertIn("hermes-streaming.sh", text)
        self.assertIn('enable_hermes_streaming "$HOME"', text)
        enable_line = text.index('enable_hermes_streaming "$HOME"')
        restart_line = text.index("systemctl --user restart hermes-gateway")
        self.assertLess(enable_line, restart_line)


if __name__ == "__main__":
    unittest.main()
