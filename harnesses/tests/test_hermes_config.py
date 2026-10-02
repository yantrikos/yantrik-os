"""hermes_config.py: the edit hermes.sh makes to Hermes's config.yaml, so the desktop platform
gets the desktop's tools and never Hermes's own terminal. Needs PyYAML, as Hermes's Python has.

The resolver checks run twice: against a stand-in that adds what Hermes adds (an unseen plugin
toolset, every MCP server when the platform names none it has), always; and against Hermes's own
`_get_platform_tools` when a Hermes checkout can be imported (HERMES_AGENT_DIR, or
~/.hermes/hermes-agent), which CI has not.
"""

import importlib.util
import io
import os
import stat
import sys
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
# setting of the person's, an MCP server of their own, and the desktop platform still on
# Hermes's own toolsets.
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
mcp_servers:
  github:
    command: gh-mcp
"""


def stand_in(plugin_toolsets=("spotify_like",)):
    """Hermes's resolver as far as these tests need it: what it adds beyond the list."""

    def resolve(config, platform):
        listed = (config.get("platform_toolsets") or {}).get(platform)
        servers = {name for name, cfg in (config.get("mcp_servers") or {}).items()
                   if not (isinstance(cfg, dict) and cfg.get("enabled") is False)}
        if not isinstance(listed, list):
            return {"hermes-" + platform} | servers
        got = {str(t) for t in listed} - {"no_mcp"}
        seen = set((config.get("known_plugin_toolsets") or {}).get(platform) or [])
        got |= {p for p in plugin_toolsets if p not in seen and p not in got}
        named = got & servers
        if "no_mcp" in listed:
            got -= servers
        elif not named:
            got |= servers
        return got

    return resolve


def real_resolver():
    """Hermes's own `_get_platform_tools`, or None where no Hermes can be imported."""
    home = Path(os.environ.get("HERMES_AGENT_DIR") or Path.home() / ".hermes" / "hermes-agent")
    if home.is_dir() and str(home) not in sys.path:
        sys.path.insert(0, str(home))
    os.environ.setdefault("HERMES_HOME", tempfile.mkdtemp())
    try:
        from hermes_cli.tools_config import _get_platform_tools
    except Exception:
        return None
    return _get_platform_tools


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

    def run_main(self, *args, resolve=None, plugins=("spotify_like",)):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = hermes_config.main(
                ["hermes_config.py", *args],
                plugin_toolsets=lambda: list(plugins),
                resolver=lambda: resolve or stand_in(plugins),
            )
        return code, out.getvalue(), err.getvalue()

    # ── apply ─────────────────────────────────────────────────────────────────────────────────

    def test_the_desktop_platform_loses_hermess_own_terminal_and_nothing_else_is_touched(self):
        # The live machine: `yantrik` had Hermes's own terminal, run as the person, ungraded.
        self.write(LIVED_IN)
        hermes_config.apply(self.path, ["spotify_like"])
        got = self.read()
        self.assertEqual(got["platform_toolsets"]["yantrik"], WANTED)
        self.assertEqual(got["platform_toolsets"]["telegram"], ["web", "terminal"], "another platform's is its own")
        self.assertEqual(got["model"], {"default": "nvidia/nemotron", "provider": "nvidia"})
        self.assertEqual(got["memory"], {"provider": "yantrikdb"})
        self.assertEqual(got["delegation"], {"model": "small-one", "max_iterations": 25})
        self.assertEqual(got["mcp_servers"]["github"], {"command": "gh-mcp"}, "their server is kept")
        self.assertEqual(hermes_config.problems(self.path, stand_in()), [])

    def test_the_desktops_mcp_server_is_added_when_missing_and_theirs_is_kept(self):
        # Without it the platform names no server Hermes has, and Hermes hands it every one.
        self.write("model:\n  default: x\n")
        hermes_config.apply(self.path)
        self.assertEqual(self.read()["mcp_servers"], {"yantrik_os": {"command": "/opt/yantrik/bin/yos-mcp", "timeout": 300}})
        self.write("mcp_servers:\n  yantrik_os:\n    command: /usr/local/bin/yos-mcp\n")
        hermes_config.apply(self.path)
        self.assertEqual(self.read()["mcp_servers"]["yantrik_os"], {"command": "/usr/local/bin/yos-mcp"})

    def test_every_plugin_toolset_hermes_knows_is_marked_seen_so_none_is_on_by_default(self):
        self.write("known_plugin_toolsets:\n  yantrik: [older]\n  cli: [x]\n")
        hermes_config.apply(self.path, ["spotify_like", "kanban"])
        got = self.read()["known_plugin_toolsets"]
        self.assertEqual(got["yantrik"], ["kanban", "older", "spotify_like"])
        self.assertEqual(got["cli"], ["x"])

    def test_skills_inline_shell_is_turned_off(self):
        self.write("skills:\n  inline_shell: true\n  inline_shell_timeout: 10\n")
        hermes_config.apply(self.path)
        self.assertEqual(self.read()["skills"], {"inline_shell": False, "inline_shell_timeout": 10})

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

    def test_an_existing_desktop_entry_is_replaced_never_doubled(self):
        # 561 had its `yantrik:` list set by hand before the installer learned to; a re-run must
        # replace it in place, and leave the file one valid mapping with the key once.
        self.write("platform_toolsets:\n  yantrik: [skills, todo, memory, session_search, clarify, delegation, yantrik_os]\n"
                   "  cli: [terminal]\ndelegation:\n  max_iterations: 40\n")
        hermes_config.apply(self.path)
        hermes_config.apply(self.path)
        with open(self.path, encoding="utf-8") as f:
            text = f.read()
        self.assertEqual(text.count("platform_toolsets:"), 1, text)
        got = self.read()
        self.assertEqual(got["platform_toolsets"], {"yantrik": WANTED, "cli": ["terminal"]})
        self.assertEqual(got["delegation"], {"max_iterations": 25})

    def test_a_file_it_cannot_safely_change_is_left_exactly_as_it_was(self):
        for text in (
            "platform_toolsets: [terminal]\n",
            "delegation: 3\n",
            "mcp_servers: [x]\n",
            "skills: on\n",
            "known_plugin_toolsets:\n  yantrik: everything\n",
            "- a list\n",
            "model: [unclosed\n",
        ):
            with self.subTest(text=text):
                self.write(text)
                code, _, err = self.run_main("apply", self.path)
                self.assertEqual(code, 1)
                self.assertTrue(err.strip())
                with open(self.path, encoding="utf-8") as f:
                    self.assertEqual(f.read(), text)

    # ── check: the allowlist ──────────────────────────────────────────────────────────────────

    def check(self, text, **kw):
        self.write(text)
        return self.run_main("check", self.path, **kw)

    def test_any_of_hermess_own_toolsets_fails_the_check(self):
        for own in ("terminal", "file", "code_execution", "browser", "web"):
            with self.subTest(own=own):
                code, _, err = self.check("platform_toolsets:\n  yantrik: [skills, %s, yantrik_os]\n"
                                          "mcp_servers:\n  yantrik_os: {command: x}\n" % own)
                self.assertEqual(code, 1)
                self.assertIn(own, err)

    def test_anything_outside_the_allowlist_fails_not_only_the_five_known_ones(self):
        # A deny-list let through whatever Hermes adds next: `hermes-cli`, a composite, is all of
        # them at once under another name.
        for other in ("hermes-cli", "image_gen", "homeassistant"):
            with self.subTest(other=other):
                code, _, err = self.check("platform_toolsets:\n  yantrik: [skills, %s, yantrik_os]\n"
                                          "mcp_servers:\n  yantrik_os: {command: x}\n" % other)
                self.assertEqual(code, 1)
                self.assertIn(other, err)

    def test_a_platform_with_no_list_fails_the_check(self):
        code, _, err = self.check("model:\n  default: x\n")
        self.assertEqual(code, 1)
        self.assertIn("not set", err)

    def test_a_subset_of_the_allowlist_passes(self):
        code, _, _ = self.check("platform_toolsets:\n  yantrik: [skills, yantrik_os]\n"
                                "mcp_servers:\n  yantrik_os: {command: x}\n", plugins=())
        self.assertEqual(code, 0)

    def test_inline_shell_on_fails_the_check(self):
        code, _, err = self.check("platform_toolsets:\n  yantrik: [yantrik_os]\nskills:\n  inline_shell: true\n"
                                  "mcp_servers:\n  yantrik_os: {command: x}\n", plugins=())
        self.assertEqual(code, 1)
        self.assertIn("inline_shell", err)

    def test_an_unseen_plugin_toolset_hermes_turns_on_by_itself_fails_the_check(self):
        code, _, err = self.check("platform_toolsets:\n  yantrik: [yantrik_os]\n"
                                  "mcp_servers:\n  yantrik_os: {command: x}\n", plugins=("kanban",))
        self.assertEqual(code, 1)
        self.assertIn("resolves the desktop platform to kanban", err)

    def test_another_mcp_server_reaching_the_platform_fails_the_check(self):
        # The platform names yantrik_os, but Hermes has no such server, so it hands over all.
        code, _, err = self.check("platform_toolsets:\n  yantrik: [yantrik_os]\nmcp_servers:\n  github: {command: gh}\n",
                                  plugins=())
        self.assertEqual(code, 1)
        self.assertIn("github", err)

    def test_a_resolver_that_cannot_answer_fails_the_install_rather_than_passing_it(self):
        def broken(config, platform):
            raise RuntimeError("no plugins dir")

        code, _, err = self.check("platform_toolsets:\n  yantrik: [yantrik_os]\n", resolve=broken)
        self.assertEqual(code, 1)
        self.assertIn("could not resolve", err)

    def test_apply_then_check_passes_and_says_what_the_platform_has(self):
        self.write(LIVED_IN)
        code, out, err = self.run_main("apply", self.path)
        self.assertEqual(code, 0, err)
        self.assertIn("yantrik_os", out)
        self.assertNotIn("terminal", out)
        self.assertEqual(self.run_main("check", self.path)[0], 0)

    # ── Hermes's own resolver, where there is one ─────────────────────────────────────────────

    def test_hermess_own_resolver_gives_the_applied_platform_exactly_the_allowlist(self):
        resolve = real_resolver()
        if resolve is None:
            self.skipTest("no Hermes to import")
        self.write(LIVED_IN)
        hermes_config.apply(self.path)
        self.assertEqual(sorted(resolve(self.read(), "yantrik")), sorted(WANTED))
        self.assertEqual(hermes_config.problems(self.path, resolve), [])
        # And it is Hermes, not this file, that says what no_mcp would do: drop the desktop too.
        data = self.read()
        data["platform_toolsets"]["yantrik"] = WANTED + ["no_mcp"]
        self.assertNotIn("yantrik_os", resolve(data, "yantrik"))

    # ── hermes.sh ─────────────────────────────────────────────────────────────────────────────

    def test_hermes_sh_applies_it_after_enabling_the_plugin_and_checks_it_last(self):
        # `hermes plugins enable` edits the same lists, and the check has to see the final file.
        script = (SCRIPT.parent / "hermes.sh").read_text(encoding="utf-8")
        enable = script.index("hermes plugins enable yantrik-desktop")
        apply_at = script.index('hermes_config.py" apply')
        provider = script.index('"$YANTRIKDB_PLUGIN"')
        check = script.index('hermes_config.py" check')
        restart = script.index("systemctl --user restart hermes-gateway")
        self.assertLess(enable, apply_at)
        self.assertLess(apply_at, provider, "the restriction must not wait on the memory provider")
        self.assertLess(restart, check)
        self.assertEqual(script.count("hermes config set memory.provider"), 1)

    def test_the_memory_provider_comes_from_a_commit_and_its_dependencies_from_hashes(self):
        script = (SCRIPT.parent / "hermes.sh").read_text(encoding="utf-8")
        self.assertNotIn("TODO(pin)", script)
        self.assertRegex(
            script,
            r'YANTRIKDB_PLUGIN="yantrikdb-hermes-plugin @ git\+https://github\.com/yantrikos/'
            r'yantrikdb-hermes-plugin@[0-9a-f]{40}"',
        )
        self.assertIn("--require-hashes", script)
        self.assertIn('--no-deps "$YANTRIKDB_PLUGIN"', script, "nothing resolved from PyPI beside it")
        lock = (SCRIPT.parent / "yantrikdb-hermes-plugin.lock").read_text(encoding="utf-8")
        pinned = [line.split()[0] for line in lock.splitlines() if line and line[0].isalpha()]
        self.assertIn("yantrikdb==0.23.1", pinned)
        for requirement in pinned:
            self.assertIn("==", requirement, "every dependency at an exact version")
        self.assertEqual(lock.count("--hash=sha256:") >= len(pinned), True)
        self.assertIn("memory.memory_enabled false", script)
        self.assertIn("memory.user_profile_enabled false", script)


if __name__ == "__main__":
    unittest.main()
