#!/usr/bin/env python3
"""`go file:///home/<person>/...` (#1152), against the live bench of test_live.py: a Mind opens the
build it just wrote, under the home only, and a page cannot lead the tab out of it.

    python3 tests/browser-core/test_local_files.py

One browser and one site for the run, from test_live; skips without a browser, as it does.
"""

import os
import shutil
import tempfile
import unittest

from test_live import Bench, setUpModule, tearDownModule  # noqa: F401  (the bench, started for this module too)
from yantrik_browser import driver as drv  # noqa: E402  (on the path once test_live is imported)


class LocalFiles(unittest.TestCase):
    """`go file:///home/<person>/...` (#1152, FABLE.md 9 Nov "got in the way" 1): a Mind opens the
    build it just wrote. Under the home only, through no link that leaves it, regular files only;
    and a page that leads the tab out of the home is not read. A temp directory stands in for the
    home here; the OS's agent file rule (the shell's files_stat) is replaced by a stand-in."""

    @classmethod
    def setUpClass(cls):
        cls.home = tempfile.mkdtemp(prefix="yb-home-")
        cls.outside = tempfile.mkdtemp(prefix="yb-outside-")
        cls.d = drv.Driver(Bench.d.sides.fixed, judge=False, file_rule=False, home=lambda: cls.home)
        os.makedirs(os.path.join(cls.home, "site"))
        with open(os.path.join(cls.outside, "secret.html"), "w") as f:
            f.write("<title>Secret</title><p>the secret text</p>")
        with open(os.path.join(cls.home, "site", "index.html"), "w") as f:
            f.write("<title>My Build</title><h1>My Build</h1><p>built at home</p>"
                    "<a href=\"file://%s/secret.html\">Leak</a>"
                    "<a href=\"file://%s/site/other.html\">Other</a>" % (cls.outside, cls.home))
        with open(os.path.join(cls.home, "site", "other.html"), "w") as f:
            f.write("<title>Other Build Page</title><p>also at home</p>")
        os.symlink(os.path.join(cls.outside, "secret.html"), os.path.join(cls.home, "site", "link.html"))
        os.symlink(cls.outside, os.path.join(cls.home, "site", "dir-link"))

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.home, ignore_errors=True)
        shutil.rmtree(cls.outside, ignore_errors=True)

    def refused(self, fn, *args, **kwargs):
        with self.assertRaises(drv.Refused) as caught:
            fn(*args, **kwargs)
        return str(caught.exception)

    def test_a_file_under_the_home_opens_as_a_url_as_a_path_and_as_tilde(self):
        index = os.path.join(self.home, "site", "index.html")
        for form in ("file://" + index, index, "~/site/index.html", "file://" + index + "?v=2"):
            out = self.d.go(form)
            self.assertTrue(out.get("navigated"), (form, out))
            self.assertEqual(out["title"], "My Build", form)
        self.assertIn("built at home", self.d.text()["text"])

    def test_outside_the_home_is_refused_before_anything_loads(self):
        self.d.go(self.home + "/site/other.html")
        why = self.refused(self.d.go, "file://" + self.outside + "/secret.html")
        self.assertIn("outside", why)
        self.assertIn("Nothing was opened", why)
        why = self.refused(self.d.go, "/etc/hostname")
        self.assertIn("outside", why)
        self.assertEqual(self.d.read()["title"], "Other Build Page", "the tab did not move")

    def test_a_link_out_of_the_home_is_refused_as_a_file_and_as_a_folder(self):
        why = self.refused(self.d.go, self.home + "/site/link.html")
        self.assertIn("link to", why)
        why = self.refused(self.d.go, self.home + "/site/dir-link/secret.html")
        self.assertIn("link to", why)

    def test_a_folder_a_missing_file_and_a_host_are_refused(self):
        self.assertIn("folder", self.refused(self.d.go, self.home + "/site"))
        self.assertIn("no file", self.refused(self.d.go, self.home + "/site/nothing.html"))
        self.assertIn("no host", self.refused(self.d.go, "file://server/share/x.html"))

    def test_the_os_file_rule_is_asked_for_an_agent_and_its_refusal_is_the_answer(self):
        from yantrik_surface.surface import Refusal
        asked = []

        def rule(path):
            asked.append(path)
            if path.endswith("other.html"):
                raise Refusal("an agent reads and writes files only in the person's home, outside its "
                              "protected places; %s is protected_place" % path)
        d = drv.Driver(Bench.d.sides.fixed, judge=False, file_rule=rule, home=lambda: self.home)
        d.go(self.home + "/site/index.html")
        self.assertEqual(asked, [self.home + "/site/index.html"])
        with self.assertRaises(drv.PageRefused) as caught:
            d.go(self.home + "/site/other.html")
        self.assertIn("protected_place", str(caught.exception))
        self.assertIn("Nothing was opened", str(caught.exception))

    def test_a_pages_link_out_of_the_home_is_not_read_and_the_tab_is_blanked(self):
        self.d.go(self.home + "/site/index.html")
        leak = next(e["ref"] for e in self.d.read()["elements"] if e.get("name") == "Leak")
        with self.assertRaises(drv.PageRefused) as caught:
            self.d.click(leak)
        self.assertIn("does not open", str(caught.exception))
        self.assertIn("about:blank", str(caught.exception))
        self.assertEqual(self.d.read()["url"], "about:blank")
        self.assertNotIn("secret text", self.d.text()["text"])

    def test_a_pages_link_within_the_home_is_followed(self):
        self.d.go(self.home + "/site/index.html")
        other = next(e["ref"] for e in self.d.read()["elements"] if e.get("name") == "Other")
        out = self.d.click(other)
        self.assertEqual(out["title"], "Other Build Page", out)

    def test_the_person_is_not_asked_the_agent_rule_but_is_held_to_the_home(self):
        # The default rule (yantrik_surface.files.may_read_file) returns at once when no agent is
        # calling; outside a dispatch nobody is, so this drives the default with the temp home.
        d = drv.Driver(Bench.d.sides.fixed, judge=False, home=lambda: self.home)
        self.assertEqual(d.go(self.home + "/site/index.html")["title"], "My Build")
        self.assertIn("outside", self.refused(d.go, "/etc/hostname"))


if __name__ == "__main__":
    unittest.main()
