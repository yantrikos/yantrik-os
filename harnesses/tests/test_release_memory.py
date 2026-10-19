"""The release bundle carries memory/ (#677): what every harness install reads at /opt/yantrik/memory.

VM 520 on build 1448 got its build by update, not from an ISO, and had no /opt/yantrik/memory:
only the ISO baked it, so every harness install from Settings stopped at "this image has no
yantrikdb-mcp". Now deploy/yantrik-os/bake-memory.sh builds the tree, build-release.sh puts it in
the bundle, `yantrik-update apply` installs it and the ISO copies it from the bundle.

The bundle itself needs cargo and the network to build, so this pins the file list instead:
`bake-memory.sh --check` passes on a bundle laid out as the installers read it and fails when any
part of the memory tree is missing, every path an installer reads under the memory directory is
one the check requires, and build-release.sh bakes and checks memory/ at the top of the bundle it
tars. Offline and stdlib-only, like the rest of harnesses/tests.
"""

import os
import re
import shutil
import stat
import subprocess
import tempfile
import unittest

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DEPLOY = os.path.join(REPO_ROOT, "deploy", "yantrik-os")
BAKE = os.path.join(DEPLOY, "bake-memory.sh")
INSTALLERS = os.path.join(REPO_ROOT, "harnesses", "lib", "install")

# The bundle's memory/ file list: each entry, and what a harness install does with it.
BUNDLE_MEMORY = {
    "VERSIONS": "what is in it, and the Python mcp/ is for",
    "bin/yantrikdb-mcp": "Pi, DeepSeek and OpenClaw start their memory server with it",
    "wheels/yantrikdb-0.23.2-cp310-abi3-manylinux_2_28_x86_64.whl": "the engine, into Hermes",
    "wheels/yantrikdb_hermes_plugin-0.28.0-py3-none-any.whl": "the Hermes provider",
    "wheels/uuid_utils-1.0.0-cp313-cp313-manylinux_2_17_x86_64.whl": "the provider's dependency",
    "wheels/click-8.3.1-py3-none-any.whl": "the provider's dependency",
    "mcp/yantrikdb_mcp/__init__.py": "yantrikdb-mcp itself",
    "mcp/yantrikdb/__init__.py": "the engine yantrikdb-mcp runs on",
}
LAUNCHER = '#!/bin/sh\nexec /usr/bin/python3 -I -c \'import sys\' "$@"\n'


def bundle(top, skip=()):
    """A bundle top directory whose memory/ holds BUNDLE_MEMORY, less the entries in skip."""
    mem = os.path.join(top, "memory")
    for rel in BUNDLE_MEMORY:
        if rel in skip:
            continue
        path = os.path.join(mem, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(LAUNCHER if rel == "bin/yantrikdb-mcp" else "")
        os.chmod(path, 0o755 if rel.startswith("bin/") else 0o644)
    for d, _, _ in os.walk(top):
        os.chmod(d, 0o755)
    return mem


def check(mem):
    return subprocess.run(["sh", BAKE, "--check", mem], capture_output=True, text=True)


class BundleFileListTests(unittest.TestCase):
    def setUp(self):
        self.top = tempfile.mkdtemp(prefix="bundle-")
        self.addCleanup(shutil.rmtree, self.top, ignore_errors=True)

    def test_a_bundle_with_the_memory_tree_passes(self):
        result = check(bundle(self.top))
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_bundle_without_memory_fails(self):
        os.makedirs(os.path.join(self.top, "bin"))
        result = check(os.path.join(self.top, "memory"))
        self.assertNotEqual(result.returncode, 0)

    def test_each_missing_entry_fails_the_check(self):
        for rel in BUNDLE_MEMORY:
            with self.subTest(missing=rel):
                top = tempfile.mkdtemp(prefix="bundle-", dir=self.top)
                result = check(bundle(top, skip={rel}))
                self.assertNotEqual(result.returncode, 0, "a bundle without %s passed" % rel)

    def test_a_launcher_without_isolated_python_fails(self):
        mem = bundle(self.top)
        with open(os.path.join(mem, "bin", "yantrikdb-mcp"), "w") as f:
            f.write("#!/bin/sh\nexec /usr/bin/python3 -s -c 'import sys' \"$@\"\n")
        self.assertIn("-I", check(mem).stderr)

    def test_a_wheel_others_can_write_fails(self):
        # The property the security review checked: neither the person nor a harness can swap a
        # wheel that the next Hermes install feeds to pip.
        mem = bundle(self.top)
        wheel = os.path.join(mem, "wheels", "click-8.3.1-py3-none-any.whl")
        os.chmod(wheel, os.stat(wheel).st_mode | stat.S_IWGRP)
        self.assertIn("others can write", check(mem).stderr)


class InstallersReadOnlyWhatShipsTests(unittest.TestCase):
    def test_every_memory_path_an_installer_reads_is_in_the_bundle(self):
        with open(os.path.join(INSTALLERS, "common.sh")) as f:
            common = f.read()
        paths = set(re.findall(r'"\$MEMORY_DIR/([^"]+)"', common))
        self.assertEqual(paths, {"wheels", "bin/yantrikdb-mcp"})
        listed = {rel.split("/")[0] for rel in BUNDLE_MEMORY} | set(BUNDLE_MEMORY)
        for p in paths:
            self.assertIn(p, listed)

    def test_hermes_installs_only_wheels_the_bundle_carries(self):
        # hermes.sh's provider install, shared with the refresh after an update.
        with open(os.path.join(INSTALLERS, "hermes-plugins.sh")) as f:
            hermes = f.read()
        # Both ways in (uv, and pip on an older Hermes) name the same packages.
        named = re.findall(r"--no-deps ([a-z0-9 -]+) </dev/null", hermes)
        self.assertEqual(len(named), 2, named)
        self.assertEqual(named[0], named[1])
        wanted = set(named[0].split())
        shipped = {os.path.basename(rel).split("-")[0].replace("_", "-")
                   for rel in BUNDLE_MEMORY if rel.startswith("wheels/")}
        self.assertEqual(wanted, shipped)


class ReleaseBuildTests(unittest.TestCase):
    def setUp(self):
        with open(os.path.join(DEPLOY, "build-release.sh")) as f:
            self.release = f.read()

    def test_the_release_bakes_and_checks_memory_at_the_bundle_top(self):
        self.assertIn('sh "$SCRIPT_DIR/bake-memory.sh" "$ROOT/memory"', self.release)
        self.assertIn('sh "$SCRIPT_DIR/bake-memory.sh" --check "$ROOT/memory"', self.release)

    def test_the_tarball_is_the_whole_bundle_top(self):
        # memory/ sits beside bin/ under $ROOT=$STAGE/$NAME, and the tarball is all of $NAME.
        self.assertIn('ROOT="$STAGE/$NAME"', self.release)
        self.assertRegex(self.release, r'tar --zstd -cf "\$TARBALL" -C "\$STAGE" "\$NAME"\n')

    def test_only_bake_memory_builds_the_tree(self):
        # One script, so the ISO and the bundle cannot drift: nothing else pins a version or
        # writes the launcher.
        for name in ("build-release.sh", "build-debian-iso.sh", "yantrik-update"):
            with open(os.path.join(DEPLOY, name)) as f:
                text = f.read()
            with self.subTest(script=name):
                self.assertNotIn("YANTRIKDB_MCP_VERSION=", text)
                self.assertNotIn("yantrikdb-hermes-plugin@", text)
                self.assertNotIn("from yantrikdb_mcp import main", text)


if __name__ == "__main__":
    unittest.main()
