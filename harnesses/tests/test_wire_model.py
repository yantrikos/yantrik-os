"""wire_model from harnesses/lib/install/common.sh: the model step of every install (#675 + #677).

Installers run under `set -eu`, so the one thing worth pinning down is that a model step which
did not run never aborts an install (yantrik-mind-72's review: a DeepSeek install must survive a
failing `yantrik-ui --wire-model`), and that what happened is recorded in the harness's
state/model for the "Can it answer?" proof: `picked`, `failed`, or no file for its own model.
A fake `yantrik-ui` on PATH stands in for the desktop. Needs a unix `sh`, so CI runs it.
"""

import os
import shutil
import stat
import subprocess
import tempfile
import unittest

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
COMMON_SH = "harnesses/lib/install/common.sh"


class WireModelTests(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="wire-model-home-")
        self.bin = tempfile.mkdtemp(prefix="wire-model-bin-")
        self.addCleanup(shutil.rmtree, self.home, ignore_errors=True)
        self.addCleanup(shutil.rmtree, self.bin, ignore_errors=True)

    def fake_ui(self, out, code):
        path = os.path.join(self.bin, "yantrik-ui")
        with open(path, "w") as f:
            f.write("#!/bin/sh\n# the desktop's --wire-model\necho '%s'\nexit %d\n" % (out, code))
        os.chmod(path, stat.S_IRWXU)

    def install(self, harness):
        env = dict(os.environ)
        env["HOME"] = self.home
        env["YANTRIK_UI"] = os.path.join(self.bin, "yantrik-ui")
        return subprocess.run(
            ["sh", "-c", 'set -eu; . %s; wire_model "$1"; echo "install went on"' % COMMON_SH, "sh", harness],
            cwd=REPO_ROOT, env=env, capture_output=True, text=True)

    def state(self, harness):
        path = os.path.join(self.home, ".local/state/yantrik/harnesses", harness, "model")
        if not os.path.exists(path):
            return None
        with open(path) as f:
            return f.read().strip()

    def test_a_failing_model_step_never_stops_a_deepseek_install(self):
        self.fake_ui("deepseek: the gateway token could not be saved", 1)
        r = self.install("deepseek")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("install went on", r.stdout)
        self.assertIn("Use Yantrik models does it", r.stdout)
        self.assertEqual(self.state("deepseek"), "failed")

    def test_a_wired_harness_is_recorded_as_picked(self):
        self.fake_ui("pi now uses your model (Revert in Settings → Minds puts its own file back)", 0)
        r = self.install("pi")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.state("pi"), "picked")
        self.fake_ui("pi already uses your model", 0)
        self.install("pi")
        self.assertEqual(self.state("pi"), "picked", "a second install changes nothing")

    def test_own_models_and_the_minds_card_leave_no_model_state(self):
        self.fake_ui("hermes uses its own models, as you chose", 0)
        self.install("hermes")
        self.assertIsNone(self.state("hermes"))
        self.fake_ui("mind keeps its own provider until you choose Use Yantrik models on its card", 0)
        r = self.install("mind")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIsNone(self.state("mind"), "an install never wires the Mind")

    def test_a_desktop_without_the_model_step_is_the_same_as_none(self):
        path = os.path.join(self.bin, "yantrik-ui")
        with open(path, "w") as f:
            f.write("#!/bin/sh\necho 'panicked: failed to load config' >&2\nexit 101\n")
        os.chmod(path, stat.S_IRWXU)
        r = self.install("deepseek")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("model: its own", r.stdout)
        self.assertIsNone(self.state("deepseek"))

    def test_only_the_last_line_is_said(self):
        self.fake_ui("INFO yantrik_ui: Loading config\npi now uses your model", 0)
        r = self.install("pi")
        self.assertIn("model: pi now uses your model", r.stdout)
        self.assertNotIn("Loading config", r.stdout)

    def test_no_desktop_is_said_and_the_install_goes_on(self):
        env = dict(os.environ)
        env["HOME"] = self.home
        env["YANTRIK_UI"] = os.path.join(self.bin, "no-such-desktop")
        r = subprocess.run(
            ["sh", "-c", 'set -eu; . %s; wire_model deepseek; echo "install went on"' % COMMON_SH],
            cwd=REPO_ROOT, env=env, capture_output=True, text=True)
        self.assertIn("model: its own", r.stdout)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("install went on", r.stdout)
        self.assertIsNone(self.state("deepseek"))


if __name__ == "__main__":
    unittest.main()
