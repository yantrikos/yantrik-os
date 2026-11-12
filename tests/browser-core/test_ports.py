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


def _skip_item(text, i):
    """The index after the item that starts at `i` (right after a `#[cfg(test)]` attribute): up to
    the `;` of `mod x;` or a single statement, or through the matching `}` of a block. Strings,
    raw strings, char literals and comments are stepped over, so a brace in them does not count."""
    n, depth = len(text), 0
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            i = text.find("\n", i) if text.find("\n", i) >= 0 else n
        elif text.startswith("/*", i):
            i = (text.find("*/", i) + 2) if text.find("*/", i) >= 0 else n
        elif c == "r" and re.match(r'r(#*)"', text[i:i + 12]) and not (i and (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = re.match(r'r(#*)"', text[i:]).group(1)
            end = text.find('"' + hashes, i + 2 + len(hashes))
            i = (end + 1 + len(hashes)) if end >= 0 else n
        elif c == '"':
            i += 1
            while i < n and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
            i += 1
        elif c == "'" and re.match(r"'(\\.[^']*|[^\\'])'", text[i:i + 12]):
            i += re.match(r"'(\\.[^']*|[^\\'])'", text[i:i + 12]).end()
        elif c == "{":
            depth += 1
            i += 1
        elif c == "}":
            depth -= 1
            i += 1
            if depth <= 0:
                return i
        elif c == ";" and depth == 0:
            return i + 1
        else:
            i += 1
    return n


def code_of(text):
    """A Rust source without its comments and its test items: what actually runs. Only the item a
    `#[cfg(test)]` attribute is on is dropped (a `mod x;`, one function, a whole `mod tests { }`),
    not the rest of the file after it, where production code may follow."""
    out, last = [], 0
    for m in re.finditer(r"#\[cfg\((?:all\()?test\b[^\]]*\]", text):
        if m.start() < last:
            continue
        out.append(text[last:m.start()])
        last = _skip_item(text, m.end())
    out.append(text[last:])
    return "\n".join(l for l in "".join(out).splitlines() if not re.match(r"\s*(//|#)", l))


SOURCE_ROOTS = (("apps",), ("crates",), ("deploy",), ("sdk",))
SKIP_DIRS = ("target", "tests", "__pycache__", "node_modules", "vendor", ".git")
SOURCE_SUFFIXES = (".rs", ".py", ".sh", ".toml", ".conf", ".service", ".js", ".json")


def production_sources(roots=SOURCE_ROOTS):
    """(relative path, the code of it) for every text file under the roots that can run: the known
    suffixes, and any file with a shebang (an extensionless executable such as `deploy/live/...`),
    not tests, docs, builds or vendored code."""
    for root in roots:
        for here, dirs, names in os.walk(os.path.join(ROOT, *root)):
            dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
            for name in names:
                path = os.path.join(here, name)
                if os.path.islink(path) or not os.path.isfile(path):
                    continue
                try:
                    with open(path, encoding="utf-8") as f:
                        text = f.read()
                except (UnicodeDecodeError, OSError):
                    continue
                if not (name.endswith(SOURCE_SUFFIXES) or text.startswith("#!")):
                    continue
                if name.endswith(".rs"):
                    code = code_of(text)
                else:
                    code = "\n".join(l for l in text.splitlines() if not re.match(r"\s*(#|//)", l))
                yield os.path.relpath(path, ROOT), code


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
        skip = {"dispatch-vectors.json", "args.rs", "vectors.rs", "yos-selftest.py"}
        pattern = re.compile(r"(?<![\d.])%s(?![\d])|mind_view_port|MIND_VIEW_PORT" % OLD_MIND_VIEW_PORT)
        offenders = [path for path, code in production_sources()
                     if os.path.basename(path) not in skip and pattern.search(code)]
        self.assertEqual(offenders, [])

    def test_no_production_source_asks_a_browser_for_a_port(self):
        """#1200: no DevTools TCP listener anywhere on the machine, so no source that runs may
        start a browser with a debugging port, in either dash spelling. The Arcade's verifier, the
        companion's headless browser, `launch_browser` and `open_apart` once did. A source may name
        the switch only in a comment or a test (the checks that refuse it are written without it).
        Every text file under the four roots is read, an extensionless script included."""
        offenders = [path for path, code in production_sources() if "remote-debugging-port" in code]
        self.assertEqual(offenders, [])

    def test_the_scan_reads_what_it_claims_to(self):
        """The scan itself: a test item is cut out and the production code after it is still read."""
        code = code_of('#[cfg(test)]\nmod helper;\nfn run() { start("--remote-debugging-" "port"); }\n'
                       '#[cfg(test)]\nmod tests { fn t() { let s = "}"; } }\nfn after() { x(); }\n')
        self.assertIn("fn run", code)
        self.assertIn("fn after", code)
        self.assertNotIn("fn t()", code)
        self.assertNotIn("mod helper", code)
        seen = {path for path, _ in production_sources()}
        self.assertIn(os.path.join("apps", "presentation", "src", "deck", "mod.rs"), seen)
        for script in (("deploy", "yantrik-os", "yos"), ("deploy", "yantrik-os", "yantrik-update")):
            self.assertIn(os.path.join(*script), seen)
        shebang = [p for p in seen if "." not in os.path.basename(p) and p.startswith("deploy" + os.sep + "live")]
        self.assertTrue(shebang, "extensionless executables under deploy/live are read")

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
