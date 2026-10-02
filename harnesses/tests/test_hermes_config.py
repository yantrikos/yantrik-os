"""hermes_config.py: the edit hermes.sh makes to Hermes's config.yaml, so the desktop platform
gets the desktop's tools and never Hermes's own terminal. Needs PyYAML, as Hermes's Python has."""

import importlib.util
import io
import os
import stat
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover - CI and Hermes's own Python both have it
    yaml = None

SCRIPT = Path(__file__).resolve().parents[1] / "lib" / "install" / "hermes_config.py"
_spec = importlib.util.spec_from_file_location("hermes_config", SCRIPT)
hermes_config = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(hermes_config)

WANTED = ["skills", "todo", "memory", "session_search", "clarify", "delegation", "yantrik_os"]

# What a Hermes that has been used looks like: a model, other platforms' toolsets, a delegation
# setting of the person's, and the desktop platform still on Hermes's own toolsets.
LIVED_IN = """\
model:
  default: nvidia/nemotron
  provider: nvidia
platform_toolsets:
  telegram: [web, terminal]
  yantrik: [terminal, file, web, memory]
delegation:
  model: small-one
memory:
  provider: yantrikdb
"""


@unittest.skipIf(yaml is None, "PyYAML is not installed")
class HermesConfigTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "config.yaml")

    def tearDown(self):
        self.dir.cleanup()

    def write(self, text, mode=0o600):
        with open(self.path, "w", encoding="utf-8") as f:
            f.write(text)
        os.chmod(self.path, mode)

    def read(self):
        with open(self.path, encoding="utf-8") as f:
            return yaml.safe_load(f)

    def run_main(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = hermes_config.main(["hermes_config.py", *args])
        return code, out.getvalue(), err.getvalue()

    def test_the_desktop_platform_loses_hermess_own_terminal_and_nothing_else_is_touched(self):
        # The live machine: `yantrik` had Hermes's own terminal, run as the person, ungraded.
        self.write(LIVED_IN)
        hermes_config.apply(self.path)
        got = self.read()
        self.assertEqual(got["platform_toolsets"]["yantrik"], WANTED)
        self.assertEqual(got["platform_toolsets"]["telegram"], ["web", "terminal"], "another platform's is its own")
        self.assertEqual(got["model"], {"default": "nvidia/nemotron", "provider": "nvidia"})
        self.assertEqual(got["memory"], {"provider": "yantrikdb"})
        self.assertEqual(got["delegation"], {"model": "small-one", "max_iterations": 25})
        self.assertEqual(hermes_config.problems(self.path), [])

    def test_a_config_with_no_platform_toolsets_gets_them(self):
        # 561 had no `yantrik` entry at all, which hands the desktop platform every toolset.
        self.write("model:\n  default: x\n")
        self.assertEqual(len(hermes_config.problems(self.path)), 1)
        hermes_config.apply(self.path)
        self.assertEqual(self.read()["platform_toolsets"], {"yantrik": WANTED})

    def test_no_config_yet_is_made_private(self):
        hermes_config.apply(self.path)
        self.assertEqual(self.read()["platform_toolsets"]["yantrik"], WANTED)
        self.assertEqual(stat.S_IMODE(os.stat(self.path).st_mode), 0o600)
        self.assertFalse(os.path.exists(self.path + hermes_config.BACKUP_SUFFIX), "nothing to back up")

    def test_the_persons_file_is_backed_up_once_and_its_mode_kept(self):
        self.write(LIVED_IN, mode=0o640)
        hermes_config.apply(self.path)
        hermes_config.apply(self.path)
        with open(self.path + hermes_config.BACKUP_SUFFIX, encoding="utf-8") as f:
            self.assertEqual(f.read(), LIVED_IN, "the second install did not replace the original")
        self.assertEqual(stat.S_IMODE(os.stat(self.path).st_mode), 0o640)
        leftovers = [n for n in os.listdir(self.dir.name) if n.endswith(".tmp")]
        self.assertEqual(leftovers, [])

    def test_a_file_it_cannot_safely_change_is_left_exactly_as_it_was(self):
        for text in ("platform_toolsets: [terminal]\n", "delegation: 3\n", "- a list\n", "model: [unclosed\n"):
            with self.subTest(text=text):
                self.write(text)
                code, _, err = self.run_main("apply", self.path)
                self.assertEqual(code, 1)
                self.assertTrue(err.strip())
                with open(self.path, encoding="utf-8") as f:
                    self.assertEqual(f.read(), text)

    def test_the_check_fails_loudly_on_any_of_hermess_own_toolsets(self):
        for own in hermes_config.FORBIDDEN:
            with self.subTest(own=own):
                self.write("platform_toolsets:\n  yantrik: [skills, %s]\n" % own)
                code, _, err = self.run_main("check", self.path)
                self.assertEqual(code, 1)
                self.assertIn(own, err)
        self.write("model:\n  default: x\n")
        code, _, err = self.run_main("check", self.path)
        self.assertEqual(code, 1)
        self.assertIn("not set", err)

    def test_apply_then_check_passes_and_says_what_the_platform_has(self):
        self.write(LIVED_IN)
        code, out, _ = self.run_main("apply", self.path)
        self.assertEqual(code, 0)
        self.assertIn("yantrik_os", out)
        self.assertNotIn("terminal", out)
        self.assertEqual(self.run_main("check", self.path)[0], 0)

    def test_hermes_sh_applies_it_after_enabling_the_plugin_and_checks_it_last(self):
        # `hermes plugins enable` edits the same lists, and the check has to see the final file.
        script = (SCRIPT.parent / "hermes.sh").read_text(encoding="utf-8")
        enable = script.index("hermes plugins enable yantrik-desktop")
        apply_at = script.index('hermes_config.py" apply')
        pin = script.index('pip install --python "$hermes_python" "$YANTRIKDB_PLUGIN"')
        check = script.index('hermes_config.py" check')
        restart = script.index("systemctl --user restart hermes-gateway")
        self.assertLess(enable, apply_at)
        self.assertLess(apply_at, pin, "the restriction must not wait on the memory provider's pin")
        self.assertLess(restart, check)
        self.assertEqual(script.count("hermes config set memory.provider"), 1)

    def test_the_memory_provider_is_installed_from_a_commit_and_nothing_is_left_to_pin(self):
        script = (SCRIPT.parent / "hermes.sh").read_text(encoding="utf-8")
        self.assertNotIn("TODO(pin)", script)
        self.assertRegex(
            script,
            r'YANTRIKDB_PLUGIN="yantrikdb-hermes-plugin @ git\+https://github\.com/yantrikos/'
            r'yantrikdb-hermes-plugin@[0-9a-f]{40}"',
        )
        self.assertIn("MEMORY_PROVIDER=yantrikdb\n", script)
        self.assertIn("memory.memory_enabled false", script)
        self.assertIn("memory.user_profile_enabled false", script)

    def test_an_existing_desktop_entry_is_replaced_never_doubled(self):
        # 561 had its `yantrik:` list set by hand before the installer learned to; a re-run must
        # replace it in place, and leave the file one valid mapping with the key once.
        self.write("platform_toolsets:\n  yantrik: [skills, todo, memory, session_search, clarify, delegation, yantrik_os]\n"
                   "  cli: [terminal]\ndelegation:\n  max_iterations: 40\n")
        hermes_config.apply(self.path)
        hermes_config.apply(self.path)
        with open(self.path, encoding="utf-8") as f:
            text = f.read()
        self.assertEqual(text.count("yantrik:"), 1, text)
        self.assertEqual(text.count("platform_toolsets:"), 1, text)
        got = self.read()
        self.assertEqual(got["platform_toolsets"], {"yantrik": WANTED, "cli": ["terminal"]})
        self.assertEqual(got["delegation"], {"max_iterations": 25})


if __name__ == "__main__":
    unittest.main()
