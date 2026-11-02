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

CONF = os.path.join(ROOT, "apps", "browser", "yantrik_browser", "browsers.conf")


def read(*path):
    with open(os.path.join(ROOT, *path), encoding="utf-8") as f:
        return f.read()


def code_of(text):
    """A source without its comments and test modules: what actually runs."""
    text = re.split(r"#\[cfg\((?:all\()?test", text)[0]
    return "\n".join(l for l in text.splitlines() if not re.match(r"\s*(//|#)", l))


class TheDefinition(unittest.TestCase):
    def test_the_ports_are_the_decided_ones(self):
        self.assertEqual((ports.PERSON_PORT, ports.MIND_VIEW_PORT), (9222, 9223))
        self.assertEqual(ports.port_of(ports.PERSON), 9222)
        self.assertEqual(ports.port_of(ports.MIND), 9223)

    def test_the_browsers_never_share_a_port_or_a_profile(self):
        first, last = ports.IDENTITY_PORTS
        self.assertNotIn(ports.PERSON_PORT, range(first, last + 1))
        self.assertNotIn(ports.MIND_VIEW_PORT, range(first, last + 1))
        self.assertLess(ports.MIND_VIEW_PORT, first)
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
        for port in (ports.PERSON_PORT, ports.MIND_VIEW_PORT, *ports.IDENTITY_PORTS):
            self.assertTrue(low <= port <= high, "%d is outside the guard %d-%d" % (port, low, high))

    def test_yos_reads_the_persons_port_from_the_same_file(self):
        import importlib.machinery
        import importlib.util
        loader = importlib.machinery.SourceFileLoader("yos_for_test", os.path.join(ROOT, "deploy", "yantrik-os", "yos"))
        spec = importlib.util.spec_from_loader("yos_for_test", loader)
        yos = importlib.util.module_from_spec(spec)
        loader.exec_module(yos)
        self.assertEqual(yos.CDP, "http://127.0.0.1:%d" % ports.PERSON_PORT)
        with self.assertRaises(AttributeError):
            yos.NOT_A_THING  # noqa: B018


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
            for number in (str(ports.PERSON_PORT), str(ports.MIND_VIEW_PORT)):
                self.assertIsNone(re.search(r"(?<![\d.])%s(?![\d])" % number, code), "%s spells %s" % ("/".join(path), number))
            self.assertNotIn("mind-view-browser", code, "/".join(path))

    def test_the_rust_constants_are_built_from_the_python_packages_file(self):
        build = read("crates", "yantrik-browser-ports", "build.rs")
        self.assertIn("apps/browser/yantrik_browser/browsers.conf", build)
        self.assertTrue(os.path.exists(CONF))
        lib = read("crates", "yantrik-browser-ports", "src", "lib.rs")
        self.assertNotRegex(code_of(lib), r"\b922[23]\b")

    def test_the_file_ships_with_the_python_package(self):
        # build-release.sh copies the whole package directory; a conf outside it would not ship.
        self.assertEqual(os.path.dirname(CONF), os.path.join(ROOT, "apps", "browser", "yantrik_browser"))
        self.assertIn('cp -r "$PROJECT_ROOT/apps/browser/yantrik_browser"', read("deploy", "yantrik-os", "build-release.sh"))


if __name__ == "__main__":
    unittest.main()
