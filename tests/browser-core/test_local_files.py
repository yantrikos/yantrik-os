#!/usr/bin/env python3
"""`go file:///home/<person>/...` (#1152), against the live bench of test_live.py: a Mind opens the
build it just wrote, under the home only, and a page cannot lead the tab out of it.

    python3 tests/browser-core/test_local_files.py

One browser and one site for the run, from test_live; skips without a browser, as it does.
"""

import os
import shutil
import tempfile
import time
import unittest
from unittest import mock

from test_live import Bench, setUpModule, tearDownModule  # noqa: F401  (the bench, started for this module too)
from test_screenshot import png_pixel  # noqa: E402
from yantrik_browser import driver as drv  # noqa: E402  (on the path once test_live is imported)
from yantrik_surface import files as home_files  # noqa: E402


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
        # Asked with the path as written, and again at every act and every request: never once per address.
        self.assertEqual(set(asked), {self.home + "/site/index.html"})
        self.assertGreater(len(asked), 1)
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


RED = (255, 0, 0)


class LocalPagesAreFenced(unittest.TestCase):
    """The security review of #1170, finding 1, 3 and 9: a local page shows only itself and what is
    inside an allowed tree. A protected file, a hidden place and anything outside the home are
    never drawn — not as a frame, an object or an embed — so a screenshot of the page never holds
    them. A link swapped behind a page is vetted again at every act, every reload and every step
    of the history. Each is red in the page it is hidden in, so a leak is a red pixel."""

    @classmethod
    def setUpClass(cls):
        from yantrik_surface.surface import Refusal
        cls.home = tempfile.mkdtemp(prefix="yb-home-")
        cls.outside = tempfile.mkdtemp(prefix="yb-outside-")
        red = "<body style=\"margin:0;background:#f00\"><p>SECRET-KEY-MATERIAL-1234</p>"
        green = "<body style=\"margin:0;background:#0f0\"><p>fine</p>"
        for folder in ("site", ".ssh", "vault", "w"):
            os.makedirs(os.path.join(cls.home, folder))
        for rel, body in (("site/ok.html", green), (".ssh/key.html", red), ("vault/private.html", red),
                          ("w/ok.html", "<title>ok page</title><p>ok page</p>"), ("w/other.html", "<p>other</p>"),
                          ("site/notes.pem", "-----BEGIN PRIVATE KEY-----"), ("site/.env", "TOKEN=1")):
            with open(os.path.join(cls.home, rel), "w") as f:
                f.write(body)
        with open(os.path.join(cls.outside, "secret.html"), "w") as f:
            f.write(red.replace("SECRET-KEY-MATERIAL-1234", "secret text"))
        frames = [("iframe", "src", "file://%s/secret.html" % cls.outside),     # outside the home
                  ("iframe", "src", "file://%s/.ssh/key.html" % cls.home),      # a hidden place
                  ("iframe", "src", "file://%s/vault/private.html" % cls.home),  # the OS rule's protected place
                  ("iframe", "src", "file://%s/site/ok.html" % cls.home),       # fine: inside, allowed
                  ("object", "data", "file://%s/secret.html" % cls.outside),
                  ("embed", "src", "file://%s/.ssh/key.html" % cls.home),
                  ("iframe", "src", "file://%s/.ssh/" % cls.home)]               # a folder listing
        cells = "".join("<%s %s=\"%s\" type=\"text/html\" style=\"position:absolute;border:0;left:%dpx;"
                        "top:%dpx;width:300px;height:300px\"></%s>" % (tag, attr, url, (i % 4) * 320, (i // 4) * 320, tag)
                        for i, (tag, attr, url) in enumerate(frames))
        with open(os.path.join(cls.home, "site", "embed.html"), "w") as f:
            f.write("<title>Embedder</title><body style=\"margin:0\">" + cells)

        def rule(path):
            if "/vault/" in path:
                raise Refusal("an agent reads and writes files only in the person's home, outside its "
                              "protected places; %s is protected_place" % path)
        cls.d = drv.Driver(Bench.d.sides.fixed, judge=False, file_rule=rule, home=lambda: cls.home)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.home, ignore_errors=True)
        shutil.rmtree(cls.outside, ignore_errors=True)

    def seen(self, fn, *args):
        """Everything an act answers with, or the refusal, as one string: where a secret would show."""
        try:
            return repr(fn(*args))
        except drv.Refused as e:
            return str(e)

    def test_nothing_protected_or_outside_is_drawn_into_a_local_page_or_its_screenshot(self):
        self.d.go(self.home + "/site/embed.html")
        time.sleep(1.0)
        shot = self.d.screenshot()
        cell = lambda i: png_pixel(shot["path"], (i % 4) * 320 + 150, (i // 4) * 320 + 150)  # noqa: E731
        self.assertEqual(cell(3), (0, 255, 0), "the allowed file inside the home is drawn")
        for i, what in ((0, "an iframe of a file outside the home"), (1, "an iframe of a hidden place"),
                        (2, "an iframe of a protected place"), (4, "an object of a file outside the home"),
                        (5, "an embed of a hidden place"), (6, "an iframe of a folder")):
            self.assertNotEqual(cell(i), RED, what + " was drawn")
        self.assertNotIn("SECRET-KEY-MATERIAL", self.d.text()["text"])

    def test_a_link_swapped_behind_a_vetted_page_opens_nothing_on_reload_or_read(self):
        link = os.path.join(self.home, "w", "l.html")
        for target, why in ((os.path.join(self.outside, "secret.html"), "a file outside the home"),
                            (os.path.join(self.home, ".ssh", "key.html"), "a hidden place")):
            if os.path.lexists(link):
                os.unlink(link)
            os.symlink(os.path.join(self.home, "w", "ok.html"), link)
            self.d.go(link)
            self.assertIn("ok page", self.d.text()["text"])
            os.unlink(link)
            os.symlink(target, link)                 # the link is re-pointed after it was vetted
            reloaded = self.seen(self.d.reload)
            self.assertNotIn("secret text", reloaded, why)
            self.assertNotIn("SECRET-KEY-MATERIAL", reloaded, why)
            self.assertIn("about:blank", reloaded + self.d.read()["url"], why)
            self.assertNotIn("secret", self.d.text()["text"], why)

    def test_a_link_swapped_behind_a_page_is_refused_on_a_plain_read_too(self):
        link = os.path.join(self.home, "w", "r.html")
        os.symlink(os.path.join(self.home, "w", "ok.html"), link)
        self.d.go(link)
        os.unlink(link)
        os.symlink(os.path.join(self.outside, "secret.html"), link)
        self.assertNotIn("secret text", self.seen(self.d.text))
        self.assertEqual(self.d.read()["url"], "about:blank")

    def test_a_link_swapped_behind_the_history_opens_nothing_on_back(self):
        link = os.path.join(self.home, "w", "h.html")
        os.symlink(os.path.join(self.home, "w", "ok.html"), link)
        self.d.go(link)
        self.d.go(self.home + "/w/other.html")
        os.unlink(link)
        os.symlink(os.path.join(self.outside, "secret.html"), link)
        self.assertNotIn("secret text", self.seen(self.d.history, -1) + self.seen(self.d.text))

    def test_hidden_places_and_files_that_are_not_web_files_are_not_opened(self):
        for path in (self.home + "/.ssh/key.html", self.home + "/site/notes.pem", self.home + "/site/.env"):
            with self.assertRaises(drv.PageRefused, msg=path):
                self.d.go(path)


FRAME = "position:absolute;border:0;left:0;top:0;width:300px;height:300px"


class PopupsAndOwnersAreFenced(unittest.TestCase):
    """The security re-review of #1170, A and B: a window a local page opens is fenced before its
    first request (a written popup draws no file the page could not), and a page is held to the rule
    of whoever loaded it, not of whoever called last."""

    @classmethod
    def setUpClass(cls):
        cls.home = tempfile.mkdtemp(prefix="yb-home-")
        cls.outside = tempfile.mkdtemp(prefix="yb-outside-")
        red = "<body style=\"margin:0;background:#f00\"><p>SECRET-KEY-MATERIAL-1234</p>"
        for folder in ("site", ".ssh", "vault"):
            os.makedirs(os.path.join(cls.home, folder))
        for rel in (".ssh/key.html", "vault/p.html"):
            with open(os.path.join(cls.home, rel), "w") as f:
                f.write(red)
        with open(os.path.join(cls.outside, "secret.html"), "w") as f:
            f.write(red)
        cls.targets = {"outside": "file://%s/secret.html" % cls.outside,
                       "hidden": "file://%s/.ssh/key.html" % cls.home,
                       "protected": "file://%s/vault/p.html" % cls.home}
        for name, url in cls.targets.items():      # the popup of the re-review: opened, written, closed
            with open(os.path.join(cls.home, "site", "pop-%s.html" % name), "w") as f:
                f.write("<button onclick=\"var x=window.open('');x.document.write('<iframe src=%s style=%s>"
                        "</iframe>');x.document.close()\">Open preview</button>" % (url, FRAME))
            with open(os.path.join(cls.home, "site", "lazy-%s.html" % name), "w") as f:     # a frame inserted later
                f.write("<p>waiting</p><script>setTimeout(() => { document.body.insertAdjacentHTML("
                        "'beforeend', '<iframe src=%s style=%s></iframe>'); }, 1200);</script>" % (url, FRAME))
        for n in range(6):
            with open(os.path.join(cls.home, "site", "f%d.html" % n), "w") as f:
                f.write("<p>frame %d</p>" % n)
        with open(os.path.join(cls.home, "site", "many.html"), "w") as f:      # six frames, a moment after it loads
            f.write("<p>many</p><script>setTimeout(() => { for (let n = 0; n < 6; n++) document.body.insertAdjacentHTML("
                    "'beforeend', '<iframe src=f' + n + '.html></iframe>'); }, 1200);</script>")
        os.link(os.path.join(cls.home, ".ssh", "key.html"), os.path.join(cls.home, "site", "hl.html"))
        cls.agent = {"is": True}
        cls.patches = [mock.patch.object(home_files, "agent_is_calling", lambda: cls.agent["is"]),
                       mock.patch.object(home_files, "shell_stat", cls.shell_stat)]
        for patch in cls.patches:
            patch.start()
        cls.d = drv.Driver(Bench.d.sides.fixed, judge=False, home=lambda: cls.home)    # the OS's own rule

    stat_delay = 0.0
    stats = 0

    @staticmethod
    def shell_stat(asked):
        """The shell's answer, after `stat_delay`: a protected place is `/vault/`, whatever else is a file."""
        PopupsAndOwnersAreFenced.stats += 1
        time.sleep(PopupsAndOwnersAreFenced.stat_delay)
        return {"exists": "unknown", "reason": "protected_place"} if "/vault/" in asked else {"exists": True, "kind": "file"}

    @classmethod
    def tearDownClass(cls):
        for patch in cls.patches:
            patch.stop()
        shutil.rmtree(cls.home, ignore_errors=True)
        shutil.rmtree(cls.outside, ignore_errors=True)

    def drawn(self, tab=None):
        """The (r, g, b) at the middle of the frame a page holds, from a screenshot of the tab."""
        return png_pixel(self.d.screenshot(tab)["path"], 150, 150)

    def test_a_window_the_page_opens_draws_no_file_the_page_could_not(self):
        for name in self.targets:
            self.d.go(self.home + "/site/pop-%s.html" % name)
            button = next(e["ref"] for e in self.d.read()["elements"] if e.get("name") == "Open preview")
            popup = self.d.click(button)["new_tabs"][0]["tab"]
            time.sleep(1.0)
            self.assertNotEqual(self.drawn(popup), RED, "the popup drew a file %s" % name)
            self.assertNotIn("SECRET-KEY-MATERIAL", self.seen_by_reading(popup), name)
            self.d.close_tab(popup)

    def seen_by_reading(self, tab):
        out = ""
        for fn in (self.d.read, self.d.text):
            try:
                out += repr(fn(tab))
            except drv.Refused as e:
                out += str(e)
        return out

    def test_a_page_an_agent_loaded_stays_under_the_agents_rule_whoever_calls_next(self):
        self.agent["is"] = True
        self.d.go(self.home + "/site/lazy-protected.html")
        self.agent["is"] = False
        try:
            self.d.read()                       # one call that is not an agent's, before the page inserts its frame
        finally:
            self.agent["is"] = True
        time.sleep(2.5)
        self.assertNotEqual(self.drawn(), RED, "a protected place was drawn into the agent's page")

    def test_a_page_the_person_loaded_is_held_to_the_persons_rule(self):
        self.agent["is"] = False
        try:
            self.d.go(self.home + "/site/lazy-protected.html", new_tab=True)
            time.sleep(2.5)
            self.assertEqual(self.drawn(), RED, "the person's own page is not held to the agent's rule")
        finally:
            self.agent["is"] = True
            self.d.close_tab(self.d.browser.active.id)

    def test_a_new_window_does_not_wait_behind_a_slow_stat(self):
        """G: the shell is slow to say whether an agent may read a file; a window opened meanwhile
        (nothing local in it) is ready at once, not after every request queued ahead of it."""
        self.d.go(self.home + "/site/many.html")
        type(self).stat_delay = 3.0
        try:
            asked = type(self).stats
            while type(self).stats == asked:   # the six frames are asked for now: 18 s of stats in a row
                time.sleep(0.1)
            began = time.monotonic()
            self.d.go(Bench.site + "/pages/blue.html", new_tab=True)
            took = time.monotonic() - began
        finally:
            type(self).stat_delay = 0.0
        self.assertLess(took, 2.5, "a new window waited %.1f s behind the stats" % took)
        self.d.close_tab(self.d.browser.active.id)
        time.sleep(7.0)                        # let the stats in flight finish before the next test

    def test_a_new_tab_is_never_made_with_a_file_url(self):
        """J: createTarget with a url is not held at its start; the driver opens blank and navigates."""
        browser = Bench.d.sides.fixed
        with mock.patch.object(browser, "call", wraps=browser.call) as spy:
            self.d.go(self.home + "/site/pop-outside.html", new_tab=True)
        made = [c.args[1]["url"] for c in spy.call_args_list if c.args[0] == "Target.createTarget"]
        self.assertEqual(made, ["about:blank"])
        self.d.close_tab(browser.active.id)

    def test_a_file_with_another_name_is_not_opened(self):
        with self.assertRaises(drv.PageRefused) as caught:
            self.d.go(self.home + "/site/hl.html")
        self.assertIn("hard link", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
