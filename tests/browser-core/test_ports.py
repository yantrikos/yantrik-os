#!/usr/bin/env python3
"""Which port and profile each browser has: one definition, read by Rust and Python (#1085).

    python3 tests/browser-core/test_ports.py

No browser, no /proc: the file, the readers, and the sources that must not spell the numbers again.
"""

import os
import re
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]

from yantrik_browser import ports  # noqa: E402

CONF = os.path.join(ROOT, "crates", "yantrik-browser-ports", "browsers.conf")
OLD_MIND_VIEW_PORT = "92" + "23"      # written so that this file does not spell it either


def read(*path):
    with open(os.path.join(ROOT, *path), encoding="utf-8") as f:
        return f.read()


def code_of(text):
    """A source without its comments and test modules: what actually runs."""
    text = re.split(r"#\[cfg\((?:all\()?test", text)[0]
    return "\n".join(l for l in text.splitlines() if not re.match(r"\s*(//|#)", l))


class TheDefinition(unittest.TestCase):
    def test_neither_browser_has_a_port_to_ask_for(self):
        """The person's Browser (#1137) and Mind View's (#1200) are held over pipes. Only the
        number the person's Browser used to listen on is left, to be refused, and Mind View's old
        number is spelled nowhere at all."""
        self.assertEqual(ports.PERSON_PORT, 9222)
        for gone in ("MIND_VIEW_PORT", "port_of", "address"):
            self.assertFalse(hasattr(ports, gone), "ports.%s is a way back to a port" % gone)
        self.assertNotIn("mind_view_port", ports.read_conf())

    def test_the_browsers_never_share_a_port_or_a_profile(self):
        self.assertFalse(hasattr(ports, "IDENTITY_PORTS"), "the identities run with no DevTools switch at all")
        self.assertNotEqual(ports.MIND_VIEW_PROFILE, ports.IDENTITY_PROFILES)

    def test_mind_views_profile_is_under_the_data_home_and_the_persons_is_not(self):
        env = {"HOME": "/home/p"}
        self.assertEqual(ports.mind_view_profile(env), "/home/p/.local/share/yantrik/mind-view-browser/chromium")
        env = {"HOME": "/home/p", "XDG_DATA_HOME": "/data"}
        self.assertEqual(ports.mind_view_profile(env), "/data/yantrik/mind-view-browser/chromium")
        # A relative or empty XDG_DATA_HOME is not a place.
        for bad in ("", "relative/dir"):
            env = {"HOME": "/home/p", "XDG_DATA_HOME": bad}
            self.assertEqual(ports.mind_view_profile(env), "/home/p/.local/share/yantrik/mind-view-browser/chromium")

    def test_the_loopback_guard_covers_every_port_the_definition_names(self):
        update = read("deploy", "yantrik-os", "yantrik-update")
        m = re.search(r"tcp dport \{ [^}]*?(\d+)-(\d+) \} meta skuid", update)
        self.assertTrue(m, "the guard's port range")
        low, high = int(m.group(1)), int(m.group(2))
        for port in (ports.PERSON_PORT,):
            self.assertTrue(low <= port <= high, "%d is outside the guard %d-%d" % (port, low, high))


class NothingSpellsThemTwice(unittest.TestCase):
    """dock.rs and target.py each spelled the profile and the port; now only the file does."""

    SOURCES = [
        ("apps", "browser", "yantrik_browser", "cdp.py"),
        ("apps", "browser", "yantrik_browser", "driver.py"),
        ("apps", "browser", "yantrik_browser", "guard.py"),
        ("apps", "browser", "yantrik_browser", "sides.py"),
        ("apps", "browser", "yantrik_browser", "service.py"),
        ("apps", "arcade", "src", "play.rs"),
        ("crates", "yantrik-ui", "src", "wire", "dock.rs"),
        ("crates", "yantrik-ui", "src", "wire", "browser_launch.rs"),
        ("crates", "yantrik-ui", "src", "mind_view.rs"),
        ("crates", "yantrik-companion-tools", "src", "browser.rs"),
        ("crates", "yantrik-companion-tools", "src", "browser_side.rs"),
        ("crates", "yantrik-companion-tools", "src", "browser_lifecycle.rs"),
        ("crates", "yantrik-companion", "src", "life_assistant", "orchestrator.rs"),
        ("deploy", "yantrik-os", "yos"),
    ]

    def test_no_code_outside_the_definition_spells_a_port_or_the_profile(self):
        for path in self.SOURCES:
            code = code_of(read(*path))
            for number in (str(ports.PERSON_PORT), OLD_MIND_VIEW_PORT):
                self.assertIsNone(re.search(r"(?<![\d.])%s(?![\d])" % number, code), "%s spells %s" % ("/".join(path), number))
            self.assertNotIn("mind-view-browser", code, "/".join(path))

    def test_mind_views_old_port_is_in_no_source_a_reader_could_fall_back_to(self):
        """#1200: every reader of that port is gone, so nothing may fall back to it. Tests may
        name it (to prove it is not listened on); production sources and the conf may not."""
        roots = [("apps", "browser"), ("crates",), ("deploy",), ("apps", "arcade")]
        skip = {"dispatch-vectors.json", "args.rs", "vectors.rs", "yos-selftest.py"}
        pattern = re.compile(r"(?<![\d.])%s(?![\d])|mind_view_port|MIND_VIEW_PORT" % OLD_MIND_VIEW_PORT)
        offenders = []
        for root in roots:
            for here, dirs, names in os.walk(os.path.join(ROOT, *root)):
                dirs[:] = [d for d in dirs if d not in ("target", "tests", "__pycache__")]
                for name in names:
                    if name in skip or not name.endswith((".rs", ".py", ".conf", ".sh", ".toml")) and name not in ("yos", "yantrik-update"):
                        continue
                    path = os.path.join(here, name)
                    with open(path, encoding="utf-8", errors="replace") as f:
                        text = f.read()
                    if name.endswith(".rs"):
                        text = code_of(text)
                    if pattern.search(text):
                        offenders.append(os.path.relpath(path, ROOT))
        self.assertEqual(offenders, [])

    def test_no_production_source_asks_a_browser_for_a_port(self):
        """#1200: no DevTools TCP listener anywhere on the machine, so no source that runs may
        start a browser with a debugging port, in either dash spelling. The Arcade's verifier, the
        companion's headless browser, `launch_browser` and `open_apart` once did. A source may name
        the switch only in a comment or a test (the checks that refuse it are written without it)."""
        offenders = []
        for root in (("apps",), ("crates",), ("deploy",), ("sdk",)):
            for here, dirs, names in os.walk(os.path.join(ROOT, *root)):
                dirs[:] = [d for d in dirs if d not in ("target", "tests", "__pycache__", "node_modules", "vendor")]
                for name in names:
                    if not name.endswith((".rs", ".py", ".sh", ".toml", ".conf", ".service")) and name not in ("yos", "yantrik-update"):
                        continue
                    path = os.path.join(here, name)
                    with open(path, encoding="utf-8", errors="replace") as f:
                        text = f.read()
                    code = code_of(text) if name.endswith(".rs") else "\n".join(
                        l for l in text.splitlines() if not re.match(r"\s*#", l))
                    if re.search(r"remote-debugging-port", code):
                        offenders.append(os.path.relpath(path, ROOT))
        self.assertEqual(offenders, [])

    def test_the_rust_constants_are_built_from_the_python_packages_file(self):
        build = read("crates", "yantrik-browser-ports", "build.rs")
        self.assertIn('const CONF: &str = "browsers.conf"', build)
        self.assertNotIn("..", code_of(build).split("const CONF")[1].split(";")[0], "build.rs must not reach into another folder")
        self.assertTrue(os.path.exists(CONF))
        lib = read("crates", "yantrik-browser-ports", "src", "lib.rs")
        self.assertNotRegex(code_of(lib), r"\b922[23]\b")

    def test_the_file_ships_beside_the_python_package_and_python_finds_it_in_both_places(self):
        release = read("deploy", "yantrik-os", "build-release.sh")
        self.assertIn("crates/yantrik-browser-ports/browsers.conf", release)
        self.assertIn("share/browser/yantrik_browser/browsers.conf", release)
        self.assertEqual(os.path.realpath(ports.CONF), os.path.realpath(CONF))


if __name__ == "__main__":
    unittest.main()
