"""Hermes in the image (#1166): the pin it shares with the runtime install, the checksum that stops
a bad fetch, and the first run that finds Hermes already there.

The real installer is a gigabyte-sized download, so a fake `curl`, a fake `hermes` and a fake
installer stand in; what is under test is the shell around them. POSIX sh only, offline.
"""

import os
import re
import shutil
import stat
import subprocess
import tempfile
import unittest
from hashlib import sha256
from pathlib import Path

import support

INSTALL = support.HARNESSES / "lib" / "install"
BAKE_ISO = support.ROOT / "deploy" / "yantrik-os" / "bake-hermes.sh"
SANDBOX = support.ROOT / "deploy" / "yantrik-os" / "bake-hermes-sandbox.sh"
CHECKS = INSTALL / "hermes-bake-checks.sh"
FAKE_INSTALLER = "#!/bin/bash\necho \"$@\" > \"$INSTALLER_RAN\"\n"


def script(path, body):
    path.write_bytes(("#!/bin/sh\n" + body).encode())
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


@unittest.skipUnless(shutil.which("sh") and shutil.which("bash"), "POSIX sh and bash")
class HermesBakeTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.home = self.root / "home"
        self.home.mkdir()
        self.curl_log = self.root / "curl.log"
        self.ran = self.root / "installer-ran"
        self.served = self.root / "served"

    def env(self):
        return {
            "PATH": "%s:/usr/bin:/bin" % self.bin,
            "HOME": str(self.home),
            "INSTALLER_RAN": str(self.ran),
            "CURL_LOG": str(self.curl_log),
            "SERVED": str(self.served),
        }

    def fake_curl(self):
        script(self.bin / "git", "exit 0\n")  # only asked to be there, as the installer needs it
        # The last argument is the URL, the one before the -o target; writes what SERVED holds.
        script(self.bin / "curl", 'echo "$@" >> "$CURL_LOG"\n'
               'while [ $# -gt 0 ]; do [ "$1" = -o ] && out=$2; shift; done\n'
               'cp "$SERVED" "$out"\n')

    def install_with(self, served, sha):
        self.served.write_bytes(served.encode())
        self.fake_curl()
        return subprocess.run(
            ["sh", "-c", '. "$1"; . "$2"; hermes_run_installer http://x/install.sh "$3" abc123',
             "sh", str(INSTALL / "common.sh"), str(INSTALL / "hermes-pin.sh"), sha],
            capture_output=True, text=True, env=self.env(),
        )

    def test_an_installer_that_does_not_match_its_checksum_is_never_run(self):
        result = self.install_with(FAKE_INSTALLER, "0" * 64)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("did not match its checksum", result.stderr)
        self.assertFalse(self.ran.exists(), "a file that failed its check must not run")

    def test_an_installer_that_matches_runs_for_the_pinned_commit_without_a_terminal(self):
        result = self.install_with(FAKE_INSTALLER, sha256(FAKE_INSTALLER.encode()).hexdigest())
        self.assertEqual(result.returncode, 0, result.stderr)
        args = self.ran.read_text().split()
        self.assertEqual(args[:2], ["--commit", "abc123"])
        for flag in ("--non-interactive", "--skip-browser", "--skip-computer-use"):
            self.assertIn(flag, args)

    def test_the_first_run_skips_the_download_when_hermes_is_present(self):
        script(self.bin / "hermes", 'case "$1" in --version) echo "Hermes Agent v0.21";; *) exit 1;; esac\n')
        self.fake_curl()
        self.served.write_bytes(b"never read")
        result = subprocess.run(
            ["sh", str(INSTALL / "hermes.sh"), str(support.HARNESSES / "hermes")],
            capture_output=True, text=True, env=self.env(), input="",
        )
        self.assertNotIn("fetching Hermes", result.stdout)
        self.assertIn("is here", result.stdout)
        self.assertFalse(self.curl_log.exists(), "nothing was downloaded")

    def test_the_first_run_downloads_when_hermes_is_missing(self):
        self.fake_curl()
        self.served.write_bytes(b"not the installer")
        result = subprocess.run(
            ["sh", str(INSTALL / "hermes.sh"), str(support.HARNESSES / "hermes")],
            capture_output=True, text=True, env=self.env(), input="",
        )
        self.assertIn("fetching Hermes", result.stdout)
        self.assertIn("did not match its checksum", result.stderr)

    def test_the_image_and_the_runtime_install_use_the_one_pin(self):
        pin = (INSTALL / "hermes-pin.sh").read_text()
        self.assertRegex(pin, r"(?m)^HERMES_COMMIT=[0-9a-f]{40}$")
        self.assertRegex(pin, r"(?m)^HERMES_INSTALLER_SHA256=[0-9a-f]{64}$")
        for name in ("hermes.sh", "hermes-bake.sh"):
            text = (INSTALL / name).read_text()
            self.assertIn("hermes-pin.sh", text, name)
            self.assertNotRegex(text, r"(?m)^HERMES_COMMIT=|^HERMES_INSTALLER_SHA256=", "%s has its own pin" % name)

    def test_the_bake_checks_the_commit_it_got_and_leaves_out_what_is_not_permissive(self):
        text = (INSTALL / "hermes-bake.sh").read_text()
        self.assertRegex(text, r'rev-parse HEAD[\s\S]*"\$got" = "\$HERMES_COMMIT"')
        self.assertRegex(text, r"rm -rf .*tools\"/ffmpeg-")
        # The licence gate reads the tree last, after every command Hermes runs in the bake.
        self.assertGreater(text.index("hermes_licences.py"), text.index("plugins enable yantrik-desktop"))

    def test_the_bake_proves_the_first_run_owes_no_sync(self):
        # Review of #1169, H1: the plugin is enabled in the bake, the next command must fetch no
        # ffmpeg, and the venv must be current after it, or the image build stops.
        text = (INSTALL / "hermes-bake.sh").read_text()
        enable = text.index("hermes plugins enable yantrik-desktop")
        self.assertGreater(enable, text.index("hermes config path"), "after Hermes's own setup")
        after = text[enable:]
        self.assertIn("hermes config path", after, "the command the first run would run next")
        self.assertRegex(after, r'ls -d "\$HOME/.hermes/tools"/ffmpeg-\*[\s\S]*\|\| fail')
        self.assertRegex(after, r"pm\.venv_is_current\(project_root=[\s\S]*\|\| fail")

    def test_the_fetched_code_runs_in_its_own_pid_namespace(self):
        # Review of #1169, L1: a fresh /proc over the chroot's, so nothing of the host is seen.
        text = BAKE_ISO.read_text()
        self.assertRegex(text, r'unshare --fork --pid [^\n]*--mount-proc="\$rootfs/proc"[\s\S]*chroot "\$rootfs" runuser')

    def test_the_fetched_code_has_its_own_ipc_and_its_own_dev(self):
        # Review of #1169, L-E: not the host's SysV IPC, and not the host's /dev with its shared /dev/shm.
        text = BAKE_ISO.read_text()
        self.assertRegex(text, r'unshare [^\n]*--ipc[^\n]*--mount [^\n]*--mount-proc')
        self.assertRegex(text, r'--mount-proc="\$rootfs/proc" \\\n\s+sh "\$\(dirname "\$0"\)/bake-hermes-sandbox.sh" "\$rootfs" \\\n\s+chroot')

    def test_the_private_dev_holds_only_what_an_installer_opens(self):
        rootfs = self.root / "rootfs"
        (rootfs / "dev").mkdir(parents=True)
        probe = ('readlink /proc/self/ns/ipc; ls -A "%s/dev"; echo x > "%s/dev/shm/mine"; echo ok > "%s/dev/null"'
                 % (rootfs, rootfs, rootfs))
        host_ipc = os.readlink("/proc/self/ns/ipc") if os.path.exists("/proc/self/ns/ipc") else None
        result = subprocess.run(["unshare", "-r", "--mount", "--ipc", "sh", str(SANDBOX), str(rootfs), "sh", "-c", probe],
                                capture_output=True, text=True)
        if result.returncode != 0 and "ermission" in result.stderr + result.stdout or host_ipc is None:
            self.skipTest("no user namespaces here: %s" % result.stderr.strip())
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = result.stdout.split()
        self.assertNotEqual(lines[0], host_ipc, "its own IPC namespace")
        self.assertEqual(sorted(lines[1:]), sorted("fd full null random shm stderr stdin stdout tty urandom zero".split()))
        self.assertEqual(list((rootfs / "dev").iterdir()), [], "nothing of it is on the host's side of the mount")

    def test_the_sandbox_takes_a_rootfs_and_a_command(self):
        result = subprocess.run(["sh", str(SANDBOX), "only-one"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)

    def test_an_image_built_without_hermes_on_purpose_touches_nothing(self):
        rootfs = self.root / "rootfs"
        rootfs.mkdir()
        env = dict(self.env(), YANTRIK_ISO_WITHOUT_HERMES="1")
        result = subprocess.run(["sh", str(BAKE_ISO), str(rootfs)], capture_output=True, text=True, env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(list(rootfs.iterdir()), [])

    def test_the_image_build_stops_if_the_harnesses_are_not_staged(self):
        rootfs = self.root / "rootfs"
        rootfs.mkdir()
        result = subprocess.run(["sh", str(BAKE_ISO), str(rootfs)], capture_output=True, text=True, env=self.env())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not staged", result.stderr)

    def tree(self, *, completion=(), fetched=()):
        """A ~/.hermes with the named files in it, as the bake's own checks see it."""
        hermes = self.home / ".hermes"
        for name in completion + fetched:
            (hermes / name).parent.mkdir(parents=True, exist_ok=True)
            (hermes / name).write_text("")
        hermes.mkdir(exist_ok=True)
        return hermes

    def ask(self, fn, *args):
        result = subprocess.run(["sh", "-c", '. "$1"; shift; "$@"', "sh", str(CHECKS), fn, *map(str, args)],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.split()

    def test_an_owed_source_update_tail_is_found_wherever_hermes_keeps_its_state(self):
        # Review of #1169, L-B: prepare_launch finishes it over the network, even with the venv current.
        self.assertEqual(self.ask("hermes_completion_owed", self.tree()), [])
        for owed in ("source-completion-pending", "source-completion-attempts"):
            hermes = self.tree(completion=("state/abc/" + owed,))
            self.assertEqual(self.ask("hermes_completion_owed", hermes), [str(hermes / "state/abc" / owed)])
            os.remove(hermes / "state/abc" / owed)
        self.tree(completion=("hermes-agent/node_modules/x/source-completion-pending",))
        self.assertEqual(self.ask("hermes_completion_owed", self.home / ".hermes"), [], "node_modules is not looked in")

    def test_what_must_not_ship_is_found_after_the_caches_come_back(self):
        # Review of #1169, L-C: the check runs after the last Hermes command, and it sees the caches too.
        npm = self.home / ".npm"
        self.assertEqual(self.ask("hermes_unshippable", self.tree(), npm), [])
        names = ("tools/ffmpeg-9-linux-x64/ffmpeg", "hermes-agent/node_modules/gsap/package.json", "cache/uv/x")
        hermes = self.tree(fetched=names)
        (npm / "_cacache").mkdir(parents=True)
        found = self.ask("hermes_unshippable", hermes, npm)
        self.assertEqual(sorted(found), sorted(str(p) for p in (
            hermes / "tools/ffmpeg-9-linux-x64", hermes / "hermes-agent/node_modules/gsap", hermes / "cache/uv", npm)))

    def test_the_bake_looks_for_the_owed_tail_and_clears_the_caches_after_hermes_has_run(self):
        text = (INSTALL / "hermes-bake.sh").read_text()
        last_hermes = max(m.start() for m in re.finditer(r"^timeout \d+ hermes |^\(cd .*venv_is_current", text, re.M))
        owed = text.index("hermes_completion_owed")
        self.assertGreater(owed, text.index("pm.venv_is_current"), "asked with the venv proof")
        caches = text.index('rm -rf "$HOME/.hermes/cache/uv" "$HOME/.npm"')
        self.assertGreater(caches, last_hermes, "the caches are cleared after the last Hermes command")
        self.assertGreater(text.index("hermes_unshippable"), caches)
        self.assertLess(text.index("hermes_unshippable"), text.index("hermes_licences.py"), "and checked before the licence gate")
        self.assertNotIn('rm -rf "$HOME/.hermes/tools"/ffmpeg-* "$HOME/.hermes/hermes-agent/node_modules/gsap" "$HOME/.hermes/cache/uv"', text)

    def test_the_baked_scripts_are_staged_and_executable_where_they_have_a_shebang(self):
        staged = (support.ROOT / "deploy" / "yantrik-os" / "stage-harnesses.sh").read_text()
        self.assertIn("hermes-pin.sh hermes-bake.sh hermes-bake-checks.sh hermes_licences.py", staged)
        for path in (INSTALL / "hermes-bake.sh", BAKE_ISO, SANDBOX):
            self.assertTrue(path.read_text().startswith("#!/bin/sh"), path)


if __name__ == "__main__":
    unittest.main()
