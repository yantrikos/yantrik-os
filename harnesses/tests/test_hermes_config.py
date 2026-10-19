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
        # The desktop's own entry is the one thing that is not left as found: a `yantrik_os` that
        # runs something else is "the desktop's tools" only by name (review round 4, F8).
        self.write("mcp_servers:\n  github: {command: gh-mcp}\n  yantrik_os:\n    command: /usr/local/bin/evil\n    env: {A: b}\n")
        hermes_config.apply(self.path)
        self.assertEqual(self.read()["mcp_servers"]["yantrik_os"], {"command": "/opt/yantrik/bin/yos-mcp", "timeout": 300})
        self.assertEqual(self.read()["mcp_servers"]["github"], {"command": "gh-mcp"})

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

    def test_hermes_never_offers_to_build_a_profile_on_the_reply_to_a_first_request(self):
        # 14 Oct: the first reply ended "want me to build a short profile of you". The offer is
        # Hermes's onboarding.profile_build; what Hermes has latched as seen is kept.
        self.write("onboarding:\n  seen: {busy_input_prompt: true}\n  profile_build: ask\n")
        hermes_config.apply(self.path)
        self.assertEqual(self.read()["onboarding"], {"seen": {"busy_input_prompt": True}, "profile_build": "off"})
        # And on a file with none, which is where Hermes's default ("ask") would apply.
        self.write("model:\n  default: x\n")
        hermes_config.apply(self.path)
        self.assertEqual(self.read()["onboarding"], {"profile_build": "off"})

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
                                          "mcp_servers:\n  yantrik_os: {command: /opt/yantrik/bin/yos-mcp, timeout: 300}\n" % own)
                self.assertEqual(code, 1)
                self.assertIn(own, err)

    def test_anything_outside_the_allowlist_fails_not_only_the_five_known_ones(self):
        # A deny-list let through whatever Hermes adds next: `hermes-cli`, a composite, is all of
        # them at once under another name.
        for other in ("hermes-cli", "image_gen", "homeassistant"):
            with self.subTest(other=other):
                code, _, err = self.check("platform_toolsets:\n  yantrik: [skills, %s, yantrik_os]\n"
                                          "mcp_servers:\n  yantrik_os: {command: /opt/yantrik/bin/yos-mcp, timeout: 300}\n" % other)
                self.assertEqual(code, 1)
                self.assertIn(other, err)

    def test_a_platform_with_no_list_fails_the_check(self):
        code, _, err = self.check("model:\n  default: x\n")
        self.assertEqual(code, 1)
        self.assertIn("not set", err)

    def test_a_subset_of_the_allowlist_passes(self):
        code, _, _ = self.check("platform_toolsets:\n  yantrik: [skills, yantrik_os]\n"
                                "mcp_servers:\n  yantrik_os: {command: /opt/yantrik/bin/yos-mcp, timeout: 300}\n", plugins=())
        self.assertEqual(code, 0)

    def test_inline_shell_on_fails_the_check(self):
        code, _, err = self.check("platform_toolsets:\n  yantrik: [yantrik_os]\nskills:\n  inline_shell: true\n"
                                  "mcp_servers:\n  yantrik_os: {command: /opt/yantrik/bin/yos-mcp, timeout: 300}\n", plugins=())
        self.assertEqual(code, 1)
        self.assertIn("inline_shell", err)

    def test_an_unseen_plugin_toolset_hermes_turns_on_by_itself_fails_the_check(self):
        code, _, err = self.check("platform_toolsets:\n  yantrik: [yantrik_os]\n"
                                  "mcp_servers:\n  yantrik_os: {command: /opt/yantrik/bin/yos-mcp, timeout: 300}\n", plugins=("kanban",))
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

    def test_hermess_own_resolver_with_the_desktop_platform_registered_installs_with_the_allowlist(self):
        # Hermes builds the desktop's composite (`hermes-yantrik`) only once the plugin has registered the
        # platform, so without this the test above never sees what a real gateway resolves. On Hermes
        # 0.14.0 (VM 520) that composite carries the kanban tools and the check fails until `apply` turns
        # kanban off; on the pinned Hermes nothing extra appears and nothing global is written.
        resolve = real_resolver()
        if resolve is None:
            self.skipTest("no Hermes to import")
        from gateway.platform_registry import platform_registry, PlatformEntry
        if not platform_registry.is_registered("yantrik"):
            platform_registry.register(PlatformEntry(
                name="yantrik", label="Yantrik OS", adapter_factory=lambda cfg: None, check_fn=lambda: True))
        self.write(LIVED_IN)
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            code = hermes_config.main(["x", "apply", self.path], lambda: [], lambda: resolve)
        self.assertEqual(code, 0)
        self.assertEqual(sorted(resolve(self.read(), "yantrik")), sorted(WANTED))
        # Only kanban is ever switched off globally, and only where this Hermes needs it.
        self.assertLessEqual(set((self.read().get("agent") or {}).get("disabled_toolsets") or []), {"kanban"})


    # ── review round 4 ────────────────────────────────────────────────────────────────────────

    def test_the_desktops_mcp_server_must_be_the_desktops_own_or_the_check_fails(self):
        # F8: kept as found, a `yantrik_os` running something else passed for the desktop's tools.
        base = "platform_toolsets:\n  yantrik: [yantrik_os]\nmcp_servers:\n  yantrik_os: %s\n"
        for bad in ("{command: /tmp/other}", "{command: /opt/yantrik/bin/yos-mcp}",
                    "{command: /opt/yantrik/bin/yos-mcp, timeout: 5}",
                    "{command: /opt/yantrik/bin/yos-mcp, timeout: 300, env: {X: y}}",
                    "{command: /opt/yantrik/bin/yos-mcp, timeout: 300, enabled: false}"):
            with self.subTest(bad=bad):
                code, _, err = self.check(base % bad, plugins=())
                self.assertEqual(code, 1)
                self.assertIn("mcp_servers.yantrik_os", err)
        code, _, err = self.check("platform_toolsets:\n  yantrik: [yantrik_os]\n", plugins=())
        self.assertEqual(code, 1, "no server at all is no desktop tools")

    def test_a_gateway_start_puts_the_allowlist_and_the_mcp_server_back(self):
        # F1: widened after install (`hermes update`, a plugin, an edit).
        self.write(LIVED_IN)
        hermes_config.apply(self.path, ["spotify_like"])
        widened = self.read()
        widened["platform_toolsets"]["yantrik"] = WANTED + ["terminal"]
        widened["mcp_servers"]["yantrik_os"] = {"command": "/tmp/x"}
        with open(self.path, "w", encoding="utf-8") as f:
            yaml.safe_dump(widened, f)
        found, left = hermes_config.reassert(self.path, lambda: ["spotify_like"], lambda: stand_in())
        self.assertTrue(any("terminal" in line for line in found), found)
        self.assertTrue(any("mcp_servers.yantrik_os" in line for line in found), found)
        self.assertEqual(left, [])
        self.assertEqual(self.read()["platform_toolsets"]["yantrik"], WANTED)
        self.assertEqual(self.read()["mcp_servers"]["yantrik_os"], {"command": "/opt/yantrik/bin/yos-mcp", "timeout": 300})
        # A clean file finds nothing, and a new plugin toolset since is found and then seen.
        found, left = hermes_config.reassert(self.path, lambda: ["spotify_like"], lambda: stand_in())
        self.assertEqual((found, left), ([], []))
        found, left = hermes_config.reassert(self.path, lambda: ["spotify_like", "kanban"], lambda: stand_in(("spotify_like", "kanban")))
        self.assertTrue(any("kanban" in line for line in found), found)
        self.assertEqual(left, [])

    def test_a_gateway_start_on_a_file_it_cannot_read_is_a_problem_not_a_pass(self):
        self.write("[not: yaml")
        found, left = hermes_config.reassert(self.path, lambda: [], lambda: stand_in())
        self.assertTrue(found and left)

    def test_hermes_sh_checks_before_the_gateway_starts_and_stops_it_on_any_failure(self):
        # F3: `check` ran only after `restart`, with the gateway already up on a widened file.
        script = (SCRIPT.parent / "hermes.sh").read_text(encoding="utf-8")
        before = script.index("hermes_check ||")
        install = script.index("gateway_install ||")
        restart = script.index("systemctl --user restart hermes-gateway")
        after = script.rindex("hermes_check ||")
        self.assertLess(before, install)
        self.assertLess(restart, after)
        self.assertEqual(script.count("hermes_check ||"), 2)
        # Every step from the first check on is `|| { stop_gateway; fail ... }`.
        tail = script[before:]
        self.assertEqual(tail.count("stop_gateway;"), 4)
        self.assertIn("systemctl --user stop hermes-gateway", script)

    def test_an_older_hermes_without_if_missing_keeps_its_unit_and_is_enabled_for_login(self):
        # Hermes 0.14.0 (VM 520) refused `--if-missing --start-now --start-on-login`, so the install
        # stopped there and Hermes never got its YantrikDB file. gateway_install is run from the
        # script itself, against a stand-in hermes and systemctl that record what they were asked.
        import re
        import shutil
        import subprocess
        sh = shutil.which("sh")
        if not sh or os.name == "nt":
            self.skipTest("needs a POSIX sh")
        script = (SCRIPT.parent / "hermes.sh").read_text(encoding="utf-8")
        body = re.search(r"^gateway_install\(\) \{.*?^\}", script, re.M | re.S).group(0)
        help_ = "hermes gateway install --help"
        cat = "systemctl --user cat hermes-gateway"
        enable = "systemctl --user enable hermes-gateway"
        for knows, has_unit, want in (
                (True, True, [help_, "hermes gateway install --if-missing --start-now --start-on-login"]),
                (False, True, [help_, cat, enable]),
                (False, False, [help_, cat, "hermes gateway install", enable])):
            with tempfile.TemporaryDirectory() as d:
                log = Path(d) / "log"
                usage = ("usage: hermes gateway install [-h] [--if-missing] [--start-now]" if knows
                         else "usage: hermes gateway install [-h] [--force] [--system]")
                (Path(d) / "hermes").write_text(
                    '#!/bin/sh\necho "hermes $*" >> %s\n[ "$3" = --help ] && echo \'%s\'\nexit 0\n' % (log, usage))
                (Path(d) / "systemctl").write_text(
                    '#!/bin/sh\necho "systemctl $*" >> %s\n[ "$2" = cat ] && exit %d\nexit 0\n' % (log, 0 if has_unit else 1))
                for f in ("hermes", "systemctl"):
                    os.chmod(Path(d) / f, 0o755)
                done = subprocess.run([sh, "-c", body + "\ngateway_install"], capture_output=True, text=True,
                                      env=dict(os.environ, PATH=d + os.pathsep + os.environ.get("PATH", "")),
                                      timeout=30)
                self.assertEqual(done.returncode, 0, done.stderr)
                self.assertEqual(log.read_text().splitlines(), want, (knows, has_unit))

    # ── a Hermes before 3d7f773, which turns kanban on for every platform (VM 520) ──────────────

    def older_hermes(self, also=()):
        """The stand-in, plus what an older Hermes recovers onto every platform whatever its list says,
        less what `agent.disabled_toolsets` (applied last) takes out."""
        base = stand_in()

        def resolve(config, platform):
            disabled = set((config.get("agent") or {}).get("disabled_toolsets") or [])
            return (base(config, platform) | {"kanban", *also}) - disabled

        return resolve

    def test_an_older_hermes_that_adds_kanban_has_it_turned_off_and_installs(self):
        self.write("model: x\nagent:\n  disabled_toolsets: [spotify]\n")
        out = io.StringIO()
        with redirect_stdout(out), redirect_stderr(io.StringIO()):
            code = hermes_config.main(["x", "apply", self.path], lambda: ["spotify_like"], lambda: self.older_hermes())
        self.assertEqual(code, 0)
        self.assertIn("kanban", out.getvalue())
        self.assertIn("your other Hermes platforms lose kanban too", out.getvalue(), "the global side effect is said")
        data = self.read()
        self.assertEqual(data["agent"]["disabled_toolsets"], ["spotify", "kanban"], "the person's own stay")
        self.assertEqual(hermes_config.problems(self.path, self.older_hermes()), [])

    def test_a_current_hermes_that_honours_the_list_is_not_given_a_global_switch(self):
        self.write("model: x\n")
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            code = hermes_config.main(["x", "apply", self.path], lambda: ["spotify_like"], lambda: stand_in())
        self.assertEqual(code, 0)
        self.assertNotIn("agent", self.read())

    def test_only_kanban_is_ever_turned_off_globally_anything_else_still_fails(self):
        # A terminal that an older Hermes added is a broken allowlist, not something to switch off for
        # the person's every other Hermes surface behind their back.
        self.write("model: x\n")
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            code = hermes_config.main(["x", "apply", self.path], lambda: ["spotify_like"],
                                      lambda: self.older_hermes(also=("terminal",)))
        self.assertEqual(code, 1)
        self.assertEqual(self.read()["agent"]["disabled_toolsets"], ["kanban"])

    def test_a_disabled_toolsets_that_is_not_a_list_is_refused_not_rewritten(self):
        self.write("model: x\nagent:\n  disabled_toolsets: \"['memory']\"\n")
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            code = hermes_config.main(["x", "apply", self.path], lambda: ["spotify_like"], lambda: self.older_hermes())
        self.assertEqual(code, 1)
        self.assertEqual(self.read()["agent"]["disabled_toolsets"], "['memory']")

    def test_hermes_sh_never_allows_every_user_and_allowlists_only_the_owner_on_the_socket(self):
        # PR #696 folded in, narrowed: the owner, never "all users", never a wildcard (#677).
        script = (SCRIPT.parent / "hermes.sh").read_text(encoding="utf-8")
        self.assertNotIn("YANTRIK_ALLOW_ALL_USERS true", script)
        self.assertIn('env_unset "$env_file" YANTRIK_ALLOW_ALL_USERS', script)
        at = script.index('env_line "$env_file" YANTRIK_ALLOWED_USERS owner')
        self.assertIn('unix:*|/*)', script[:at][-200:])
        self.assertNotIn("YANTRIK_ALLOWED_USERS *", script)
        adapter = (SCRIPT.parents[2] / "hermes" / "adapter.py").read_text(encoding="utf-8")
        self.assertIn('os.environ.setdefault("YANTRIK_ALLOWED_USERS", OWNER_ID)', adapter)
        self.assertNotIn('setdefault("YANTRIK_ALLOW_ALL_USERS"', adapter)

    # ── hermes.sh ─────────────────────────────────────────────────────────────────────────────

    def test_hermes_sh_applies_it_after_enabling_the_plugin_and_checks_it_last(self):
        # `hermes plugins enable` edits the same lists, and the check has to see the final file.
        script = (SCRIPT.parent / "hermes.sh").read_text(encoding="utf-8")
        enable = script.index("hermes plugins enable yantrik-desktop")
        apply_at = script.index('hermes_config.py" apply')
        provider = script.index('hermes_yantrikdb_plugin_build "$new_plugin"')
        restart = script.index("systemctl --user restart hermes-gateway")
        check = script.rindex("hermes_check ||")
        proofs = script.index("prove hermes")
        self.assertLess(enable, apply_at)
        self.assertLess(apply_at, provider, "the restriction must not wait on the memory provider")
        self.assertLess(restart, check)
        self.assertLess(check, proofs, "installed only once the three proofs pass")
        self.assertEqual(script.count('hermes_config.py" memory "$config"'), 1)
        self.assertNotIn("hermes config set memory.", script, "a `config set` waits on Hermes's own update")

    def test_the_memory_provider_comes_from_the_image_and_nothing_is_fetched(self):
        # #677: no catalog, no git, no PyPI at install time, and no TTY.
        script = (SCRIPT.parent / "hermes.sh").read_text(encoding="utf-8")
        memory = script[script.index("# ── Memory"):script.index("# ── The desktop channel")]
        # The install itself is in hermes-plugins.sh, which the refresh after an update shares.
        plugins = (SCRIPT.parent / "hermes-plugins.sh").read_text(encoding="utf-8")
        self.assertIn('hermes_yantrikdb_plugin_build "$new_plugin"', memory)
        self.assertIn("--no-index", plugins)
        self.assertIn('--find-links "$MEMORY_WHEELS"', plugins)
        self.assertNotIn("git+", plugins)
        self.assertNotIn("\nhermes plugins install", plugins)
        self.assertNotIn("git+", memory)
        self.assertNotIn("\nhermes plugins install", memory)
        self.assertIn('hermes_config.py" memory "$config"', memory)
        self.assertEqual(hermes_config.MEMORY, {"provider": "yantrikdb", "memory_enabled": False, "user_profile_enabled": False})
        self.assertIn("YANTRIKDB_AGENT_NAME hermes", memory, "every write carries its harness")
        self.assertIn("TODO(#586)", memory, "the shared mode waits on the tamper-proof door, said plainly")


class HermesYaml11Tests(unittest.TestCase):
    """Hermes's own Python: ruamel.yaml and no PyYAML, which the class above needs."""

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "config.yaml")

    def tearDown(self):
        self.dir.cleanup()

    def test_off_stays_a_string_where_hermes_writes_with_ruamel(self):
        # Hermes 0.21's Python has no PyYAML, so apply() writes with ruamel.yaml. As YAML 1.2 that
        # wrote `profile_build: off` unquoted, Hermes read it back as YAML 1.1 (hermes_yaml.py), got
        # False and still made the offer (VM 542, 15 Oct 2026). Read here as Hermes reads it.
        try:
            from ruamel.yaml import YAML
        except ImportError:
            self.skipTest("ruamel.yaml is not installed here")
        with open(self.path, "w", encoding="utf-8") as f:
            f.write("model:\n  default: x\n")
        os.chmod(self.path, 0o600)
        saved = sys.modules.get("yaml")
        sys.modules["yaml"] = None  # `import yaml` raises ImportError: the ruamel branch
        try:
            hermes_config.apply(self.path)
        finally:
            if saved is None:
                del sys.modules["yaml"]
            else:
                sys.modules["yaml"] = saved
        as_hermes = YAML(typ="safe", pure=True)
        as_hermes.version = (1, 1)
        with open(self.path, encoding="utf-8") as f:
            text = f.read()
        self.assertEqual(as_hermes.load(text)["onboarding"]["profile_build"], "off", text)
        self.assertNotIn("%YAML", text, "no directive, as Hermes's own safe_dump writes none")

@unittest.skipIf(yaml is None, "PyYAML is not installed")
class HermesMemoryTests(unittest.TestCase):
    """`memory`: YantrikDB as Hermes's one memory, written without `hermes config set`."""

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "config.yaml")

    def tearDown(self):
        self.dir.cleanup()

    def test_the_installed_plugin_manifest_asks_hermes_to_install_nothing(self):
        # Left in, Hermes's own update fetches yantrikdb from PyPI into its venv, ahead of vendor/.
        src = os.path.join(self.dir.name, "src.yaml")
        dest = os.path.join(self.dir.name, "plugin.yaml")
        with open(src, "w", encoding="utf-8") as f:
            f.write("name: yantrikdb\nversion: 0.28.0\npip_dependencies:\n  - yantrikdb>=0.12.1\n  - requests>=2.31\n"
                    "python_dependencies:\n  - x\nhooks:\n  - on_session_end\n  - on_memory_write\n")
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            self.assertEqual(hermes_config.main(["hermes_config.py", "manifest", src, dest]), 0)
        with open(dest, encoding="utf-8") as f:
            data = yaml.safe_load(f)
        self.assertEqual(data, {"name": "yantrikdb", "version": "0.28.0", "hooks": ["on_session_end", "on_memory_write"]})

    def test_hermes_sh_checks_after_hermes_s_update_that_the_provider_runs_from_vendor(self):
        script = (SCRIPT.parent / "hermes.sh").read_text(encoding="utf-8")
        plugins = (SCRIPT.parent / "hermes-plugins.sh").read_text(encoding="utf-8")
        self.assertIn('hermes_config.py" manifest', plugins)
        self.assertNotIn("cp \"$new_plugin/vendor/yantrikdb_hermes_plugin/plugin.yaml\"", plugins)
        settle = script.index("timeout 300 hermes config path")
        check = script.index("is not the one this install put in")
        restart = script.index("systemctl --user restart hermes-gateway")
        self.assertLess(settle, check, "checked in the Python Hermes runs on after its update")
        self.assertLess(check, restart)
        self.assertIn('module.__file__.startswith(vendor + "/")', script)

    def test_memory_makes_yantrikdb_the_one_memory_and_keeps_the_rest(self):
        with open(self.path, "w", encoding="utf-8") as f:
            f.write("memory:\n  memory_char_limit: 2200\n  memory_enabled: true\nmodel:\n  default: x\n")
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            self.assertEqual(hermes_config.main(["hermes_config.py", "memory", self.path]), 0)
        with open(self.path, encoding="utf-8") as f:
            data = yaml.safe_load(f)
        self.assertEqual(data["memory"], {
            "memory_char_limit": 2200,
            "memory_enabled": False,
            "provider": "yantrikdb",
            "user_profile_enabled": False,
        })
        self.assertEqual(data["model"], {"default": "x"})
        self.assertEqual(hermes_config.memory_problems(self.path), [])

    def test_memory_problems_names_each_key_that_is_wrong(self):
        with open(self.path, "w", encoding="utf-8") as f:
            f.write("memory:\n  provider: holographic\n  user_profile_enabled: false\n")
        wrong = hermes_config.memory_problems(self.path)
        self.assertEqual(len(wrong), 2, wrong)
        self.assertTrue(any("memory.provider" in w for w in wrong))
        self.assertTrue(any("memory.memory_enabled" in w for w in wrong))

if __name__ == "__main__":
    unittest.main()
