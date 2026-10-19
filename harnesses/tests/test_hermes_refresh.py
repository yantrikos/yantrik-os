"""refresh-plugins.sh: an OS update brings the Hermes plugins in the person's home up to the build's.

Hermes loads ~/.hermes/plugins/, which hermes.sh wrote once, at install, so on VM 541 (14 Oct) the
adapter the build shipped had send_clarify and the one Hermes ran did not. These run the real script
as it ships (a share/harnesses tree laid out the way stage-harnesses.sh lays it) against a fake home,
with a `systemctl` that only records what it was asked, a Hermes whose Python is this one, and a
`uv` that writes a provider whose content a test chooses. Offline and stdlib-only, PyYAML aside.
"""

import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import support

try:
    import yaml
except ImportError:  # pragma: no cover - CI has it
    yaml = None

INSTALL = support.HARNESSES / "lib" / "install"
UPDATER = support.ROOT / "deploy" / "yantrik-os" / "yantrik-update"

SYSTEMCTL = """#!/bin/sh
printf '%s\\n' "$*" >> "$FAKE_LOG"
case "$*" in
    *is-active*) [ "${FAKE_GATEWAY:-active}" = active ] ;;
    *) exit 0 ;;
esac
"""

# What `uv pip install --target DIR` leaves, standing in for the image's wheels. FAKE_PROVIDER is
# the provider's version, so a test can ship a different one; the RECORD changes on every run, as
# a real one's bytecode hashes can.
UV = """#!/bin/sh
target=""
while [ $# -gt 0 ]; do
    [ "$1" = --target ] && { target=$2; shift; }
    shift
done
mkdir -p "$target/yantrikdb" "$target/yantrikdb_hermes_plugin" "$target/yantrikdb_hermes_plugin-1.dist-info" \
    "$target/requests"
: > "$target/yantrikdb/__init__.py"
# requests is Hermes's own; this Python stands in for Hermes's and may not have it.
: > "$target/requests/__init__.py"
printf 'VERSION = "%s"\\nclass YantrikDBMemoryProvider:\\n    pass\\n' "${FAKE_PROVIDER:-1}" \\
    > "$target/yantrikdb_hermes_plugin/__init__.py"
printf 'name: yantrikdb\\npip_dependencies:\\n  - yantrikdb\\n' > "$target/yantrikdb_hermes_plugin/plugin.yaml"
date +%s%N > "$target/yantrikdb_hermes_plugin-1.dist-info/RECORD"
"""


def tree(path):
    """Every file under `path` with its sha256, bytecode left out."""
    return {
        str(p.relative_to(path)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(path.rglob("*"))
        if p.is_file() and "__pycache__" not in p.parts
    }


@unittest.skipUnless(shutil.which("sh"), "the refresh is a POSIX sh script")
class RefreshTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.home = root / "home"
        self.home.mkdir()
        # The build's share/harnesses: lib as it ships, and Hermes's plugin as a test writes it.
        self.share = root / "share" / "harnesses"
        shutil.copytree(support.HARNESSES / "lib", self.share / "lib",
                        ignore=shutil.ignore_patterns("__pycache__"))
        self.shipped = self.share / "hermes"
        self.shipped.mkdir()
        (self.shipped / "plugin.yaml").write_text("name: yantrik-desktop\n")
        (self.shipped / "adapter.py").write_text("def send_clarify():\n    pass\n")
        self.memory = root / "memory"
        (self.memory / "wheels").mkdir(parents=True)
        self.bin = root / "bin"
        self.bin.mkdir()
        for name, text in (("systemctl", SYSTEMCTL), ("uv", UV)):
            (self.bin / name).write_text(text)
            (self.bin / name).chmod(0o755)
        self.log = root / "systemctl.log"

    def env(self, **extra):
        env = {
            "HOME": str(self.home),
            "PATH": "%s:/usr/bin:/bin" % self.bin,
            "FAKE_LOG": str(self.log),
            "YANTRIK_MEMORY_DIR": str(self.memory),
        }
        env.update(extra)
        return env

    def refresh(self, *args, **extra):
        return subprocess.run(
            ["sh", str(self.share / "lib" / "install" / "refresh-plugins.sh"), *args],
            capture_output=True, text=True, env=self.env(**extra), stdin=subprocess.DEVNULL,
        )

    def restarts(self):
        if not self.log.exists():
            return []
        return [line for line in self.log.read_text().splitlines() if line.startswith("--user restart")]

    def install_hermes(self):
        """What a Hermes install leaves that the refresh must not touch, and a Hermes whose Python
        (beside ~/.local/bin/hermes, as on an older Hermes) is this one."""
        hermes = self.home / ".hermes"
        (hermes / "sessions").mkdir(parents=True)
        (hermes / "config.yaml").write_text("model: theirs\n")
        (hermes / ".env").write_text("YANTRIK_ALLOWED_USERS=owner\n")
        (hermes / "sessions" / "one.json").write_text("{}")
        (hermes / "plugins" / "someone-else").mkdir(parents=True)
        (hermes / "plugins" / "someone-else" / "plugin.yaml").write_text("name: theirs\n")
        venv = hermes / "venv" / "bin"
        venv.mkdir(parents=True)
        (venv / "hermes").write_text("#!/bin/sh\n")
        (venv / "hermes").chmod(0o755)
        os.symlink(sys.executable, venv / "python")
        (self.home / ".local" / "bin").mkdir(parents=True)
        os.symlink(venv / "hermes", self.home / ".local" / "bin" / "hermes")

    def install_desktop_plugin(self, adapter):
        dest = self.home / ".hermes" / "plugins" / "yantrik"
        dest.mkdir(parents=True)
        (dest / "plugin.yaml").write_text("name: yantrik-desktop\n")
        (dest / "adapter.py").write_text(adapter)
        return dest

    def assert_only_our_plugins_and_theirs(self):
        """Hermes 0.21 loads every child of ~/.hermes/plugins with a plugin.yaml, dot-directories
        included, and of two with one name the later in sorted order wins: a staging or backup copy
        left there can be the plugin Hermes runs (VM 541, 14 Oct). Nothing is ever left there, and
        the staging directory beside it is gone once the refresh is done."""
        plugins = self.home / ".hermes" / "plugins"
        self.assertLessEqual({p.name for p in plugins.iterdir()}, {"yantrik", "yantrikdb", "someone-else"})
        self.assertFalse((self.home / ".hermes" / ".yantrik-staging").exists())

    def theirs(self):
        """Everything under ~/.hermes that is not one of the two plugins an install writes."""
        hermes = self.home / ".hermes"
        return {k: v for k, v in tree(hermes).items()
                if not k.startswith(("plugins/yantrik/", "plugins/yantrikdb/"))}

    def test_an_older_desktop_plugin_is_replaced_and_the_gateway_left_for_its_next_start(self):
        # A restart kills a turn in flight, which Hermes then replays into the next turn as a
        # "Recovered reply" (review of #808): the update never restarts the gateway itself.
        self.install_hermes()
        dest = self.install_desktop_plugin("def send_message():\n    pass\n")
        (dest / "stale.py").write_text("gone from the build\n")
        result = self.refresh()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(tree(dest), tree(self.shipped))
        self.assertIn("def send_clarify", (dest / "adapter.py").read_text())
        self.assertEqual(self.restarts(), [])
        self.assertFalse(self.log.exists(), "not even asked whether the gateway runs")
        self.assertIn("when its gateway next starts", result.stdout)
        self.assert_only_our_plugins_and_theirs()

    def test_with_restart_an_older_desktop_plugin_is_replaced_and_the_gateway_restarted_once(self):
        self.install_hermes()
        dest = self.install_desktop_plugin("def send_message():\n    pass\n")
        (dest / "stale.py").write_text("gone from the build\n")
        (dest / "__pycache__").mkdir()
        result = self.refresh("--restart")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(tree(dest), tree(self.shipped))
        self.assertIn("def send_clarify", (dest / "adapter.py").read_text())
        self.assertEqual(self.restarts(), ["--user restart hermes-gateway"])
        self.assertIn("refreshed Hermes's desktop plugin", result.stdout)
        self.assertIn("restarted Hermes's gateway", result.stdout)
        logged = (self.home / ".local/state/yantrik/harnesses/hermes/refreshed").read_text()
        self.assertTrue(logged.rstrip().endswith(" yantrik"), logged)
        self.assert_only_our_plugins_and_theirs()

    def test_an_identical_desktop_plugin_is_left_alone_with_no_restart(self):
        self.install_hermes()
        dest = self.install_desktop_plugin("def send_clarify():\n    pass\n")
        # Bytecode Hermes wrote when it imported the plugin is not a difference.
        (dest / "__pycache__").mkdir()
        (dest / "__pycache__" / "adapter.cpython-313.pyc").write_bytes(b"\0")
        inode = (dest / "adapter.py").stat().st_ino
        result = self.refresh("--restart")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual((dest / "adapter.py").stat().st_ino, inode)
        self.assertEqual(self.restarts(), [])
        self.assertFalse((self.home / ".local/state/yantrik/harnesses/hermes/refreshed").exists())

    def test_config_yaml_and_everything_else_under_hermes_is_untouched(self):
        self.install_hermes()
        self.install_desktop_plugin("old\n")
        before = self.theirs()
        stat = (self.home / ".hermes" / "config.yaml").stat()
        result = self.refresh()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.theirs(), before)
        self.assertEqual((self.home / ".hermes" / "config.yaml").stat().st_mtime_ns, stat.st_mtime_ns)

    def test_a_machine_with_no_hermes_does_nothing(self):
        result = self.refresh()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout + result.stderr, "")
        self.assertEqual(list(self.home.iterdir()), [])
        self.assertFalse(self.log.exists())

    def test_a_hermes_without_our_plugin_is_left_alone(self):
        self.install_hermes()
        before = tree(self.home)
        result = self.refresh()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(tree(self.home), before)
        self.assertFalse((self.home / ".hermes/plugins/yantrik").exists())
        self.assertFalse(self.log.exists())

    def test_a_stopped_gateway_is_not_started(self):
        self.install_hermes()
        self.install_desktop_plugin("old\n")
        result = self.refresh("--restart", FAKE_GATEWAY="inactive")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.restarts(), [])
        self.assertIn("not running", result.stdout)

    @unittest.skipIf(yaml is None, "PyYAML is not installed")
    def test_the_yantrikdb_plugin_is_rebuilt_only_when_the_build_ships_another(self):
        self.install_hermes()
        self.install_desktop_plugin("def send_clarify():\n    pass\n")
        dest = self.home / ".hermes" / "plugins" / "yantrikdb"
        dest.mkdir()
        (dest / "__init__.py").write_text("an older shim\n")
        result = self.refresh("--restart", FAKE_PROVIDER="2")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("refreshed Hermes's YantrikDB plugin", result.stdout, result.stderr)
        self.assertIn('VERSION = "2"', (dest / "vendor/yantrikdb_hermes_plugin/__init__.py").read_text())
        self.assertIn("_VENDOR", (dest / "__init__.py").read_text())
        self.assertNotIn("pip_dependencies", (dest / "plugin.yaml").read_text())
        self.assertEqual(self.restarts(), ["--user restart hermes-gateway"])

        # The same wheels again: a RECORD that differs is not a new provider.
        self.log.unlink()
        again = self.refresh("--restart", FAKE_PROVIDER="2")
        self.assertEqual(again.returncode, 0, again.stderr)
        self.assertEqual(again.stdout, "")
        self.assertEqual(self.restarts(), [])
        self.assert_only_our_plugins_and_theirs()

    def test_a_yantrikdb_build_that_fails_keeps_the_one_in_place(self):
        self.install_hermes()
        self.install_desktop_plugin("def send_clarify():\n    pass\n")
        dest = self.home / ".hermes" / "plugins" / "yantrikdb"
        dest.mkdir()
        (dest / "__init__.py").write_text("the working shim\n")
        (self.bin / "uv").write_text("#!/bin/sh\nexit 1\n")
        result = self.refresh()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((dest / "__init__.py").read_text(), "the working shim\n")
        self.assertIn("left Hermes's YantrikDB plugin as it was", result.stdout)
        self.assert_only_our_plugins_and_theirs()
        self.assertEqual(self.restarts(), [])

    def test_a_swap_whose_second_move_fails_puts_the_old_plugin_back_and_leaves_nothing_in_plugins(self):
        # The half-swapped case: the old plugin moved aside, the new one not moved in. Staged
        # inside plugins/, that left `.yantrik-old` beside `.yantrik-new`, and Hermes would run the
        # old one (review of #808).
        self.install_hermes()
        dest = self.install_desktop_plugin("the working adapter\n")
        missing = self.home / ".hermes" / ".yantrik-staging" / "yantrik-new"
        result = subprocess.run(
            ["sh", "-c", '. "$1"; hermes_swap_dir "$2" "$3"', "sh",
             str(INSTALL / "hermes-plugins.sh"), str(missing), str(dest)],
            capture_output=True, text=True, env=self.env(),
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((dest / "adapter.py").read_text(), "the working adapter\n")
        self.assertLessEqual({p.name for p in (self.home / ".hermes" / "plugins").iterdir()},
                             {"yantrik", "someone-else"})

    def test_a_fresh_install_with_no_plugins_folder_yet_gets_its_plugin(self):
        # A fresh Hermes has no ~/.hermes/plugins. With the staging moved out of it, nothing made
        # it, and every fresh install failed at "could not put the desktop plugin in
        # ~/.hermes/plugins/yantrik" (VM 542, 15 Oct 2026).
        (self.home / ".hermes").mkdir()
        plugins = self.home / ".hermes" / "plugins"
        self.assertFalse(plugins.exists())
        new = self.home / ".hermes" / ".yantrik-staging" / "yantrik-new"
        new.mkdir(parents=True)
        (new / "adapter.py").write_text("the new adapter\n")
        result = subprocess.run(
            ["sh", "-c", '. "$1"; hermes_swap_dir "$2" "$3"', "sh",
             str(INSTALL / "hermes-plugins.sh"), str(new), str(plugins / "yantrik")],
            capture_output=True, text=True, env=self.env(),
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((plugins / "yantrik" / "adapter.py").read_text(), "the new adapter\n")
        self.assertEqual({p.name for p in plugins.iterdir()}, {"yantrik"})
        self.assertFalse((self.home / ".hermes" / ".yantrik-staging").exists(), "the staging is cleared")

    def test_a_staging_copy_an_interrupted_refresh_left_is_not_a_plugin_and_is_replaced(self):
        self.install_hermes()
        dest = self.install_desktop_plugin("old\n")
        stale = self.home / ".hermes" / ".yantrik-staging" / "yantrik-new"
        stale.mkdir(parents=True)
        (stale / "half-written.py").write_text("")
        result = self.refresh()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(tree(dest), tree(self.shipped))
        self.assert_only_our_plugins_and_theirs()


class OneImplementationTests(unittest.TestCase):
    """The refresh writes what an install writes because both call the same functions."""

    def test_hermes_sh_writes_both_plugins_through_hermes_plugins_sh(self):
        text = (INSTALL / "hermes.sh").read_text()
        self.assertIn('. "$(dirname "$0")/hermes-plugins.sh"', text)
        self.assertIn('hermes_desktop_plugin_build "$plugin"', text)
        self.assertIn('hermes_yantrikdb_plugin_build "$new_plugin" "$hermes_python"', text)
        # No second copy of the provider's install left behind in the installer.
        self.assertNotIn("pip install", text)
        self.assertNotIn('cp -r "$plugin/."', text)

    def test_nothing_is_ever_staged_inside_the_directory_hermes_scans(self):
        # A copy interrupted mid-build stays where it was staged, and under plugins/ Hermes would
        # load it (review of #808). So staging is a directory beside plugins/, never inside it.
        with tempfile.TemporaryDirectory() as home:
            out = subprocess.run(
                ["sh", "-c", '. "$1"; hermes_staging yantrik', "sh", str(INSTALL / "hermes-plugins.sh")],
                capture_output=True, text=True, env={"HOME": home, "PATH": "/usr/bin:/bin"},
            ).stdout.strip()
            self.assertEqual(out, os.path.join(home, ".hermes", ".yantrik-staging", "yantrik"))
        for name in ("hermes.sh", "hermes-plugins.sh", "refresh-plugins.sh"):
            text = (INSTALL / name).read_text()
            self.assertNotIn('plugins/.yantrik', text, name)
            self.assertNotIn('$plugins/.', text, name)

    def test_the_refresh_script_is_executable_and_staged(self):
        self.assertTrue(os.access(INSTALL / "refresh-plugins.sh", os.X_OK))
        stage = (support.ROOT / "deploy" / "yantrik-os" / "stage-harnesses.sh").read_text()
        self.assertIn("hermes-plugins.sh", stage)
        self.assertIn("refresh-plugins.sh", stage)


class UpdaterTests(unittest.TestCase):
    """yantrik-update runs the refresh on apply and on reconcile, as the owner, never as root."""

    def setUp(self):
        self.text = UPDATER.read_text()
        start = self.text.index("reconcile_harness_plugins() {")
        self.body = self.text[start:self.text.index("\n}\n", start)]

    def test_the_updater_never_asks_for_a_gateway_restart(self):
        # A turn in flight would come back as a "Recovered reply" in the person's next turn.
        self.assertIn('/bin/sh "$script" )', self.body)
        self.assertNotIn("--restart", self.body)

    def test_reconcile_and_an_up_to_date_apply_both_refresh(self):
        reconcile = self.text[self.text.index("cmd_reconcile() {"):]
        reconcile = reconcile[:reconcile.index("\n}\n")]
        self.assertIn("reconcile_harness_plugins", reconcile)
        # The launcher is healed before the refreshed plugins need the gateway to start (#712).
        self.assertLess(reconcile.index("reconcile_hermes_launcher"), reconcile.index("reconcile_harness_plugins"))
        self.assertIn("reconcile_harness_plugins || true", self.text)
        # A full apply reconciles through the new build's own updater, which runs the refresh.
        self.assertIn('"$BIN_DIR/yantrik-update" reconcile', self.text)

    def test_it_drops_to_the_owner_with_no_new_privileges(self):
        self.assertIn("tree_is_roots || return 0", self.body)
        self.assertIn('--reuid="$ouid" --regid="$ogid" --clear-groups --no-new-privs', self.body)
        self.assertIn("0:*|*:0", self.body)

    def test_root_runs_nothing_found_on_the_owners_path(self):
        # After `env -i`, PATH starts with the owner's ~/.local/bin: every program root starts there
        # is named whole.
        line = self.body[self.body.index("exec env -i"):]
        line = line[:line.index("|| rc=$?")]
        self.assertIn('"$timeout_bin" -k 10 600 "$setsid_bin" -w "$setpriv"', line)
        self.assertIn("/bin/sh \"$script\"", line)

    def test_the_owners_programs_get_no_terminal_and_cannot_hold_the_update(self):
        # A program of the owner's in root's session could open /dev/tty and push keystrokes into
        # root's shell (TIOCSTI); one holding a pipe open would keep the update waiting.
        line = self.body[self.body.index("exec env -i"):]
        line = line[:line.index("|| rc=$?")]
        self.assertIn('"$setsid_bin" -w', line)
        self.assertIn('</dev/null >"$log" 2>&1 9>&-', line)
        self.assertIn('setsid_bin="$(command -v setsid)"', self.body)


if __name__ == "__main__":
    unittest.main()
