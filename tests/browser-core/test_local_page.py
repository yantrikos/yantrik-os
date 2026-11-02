#!/usr/bin/env python3
"""A Mind's own pages (#1068): `go` takes `file:` URLs from the person's home and nothing else on
disk, a page of the Mind's goes to a render instance and not the person's browser, and nothing is
read back from a file a Mind may not read. No browser is needed: the shell's `files_stat` is stood
in for, and the browsers are stubs that record where they were sent.

    python3 tests/browser-core/test_local_page.py
"""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]

from yantrik_browser import cdp, driver as drv, local_page, render_instance  # noqa: E402
from yantrik_surface import files  # noqa: E402

drv.hands.settle = lambda *a, **k: (0.0, "")   # no page to wait for

ANSWERS = {
    "/home/p/Sites/index.html": {"exists": True, "kind": "file"},
    "/home/p/Sites/style.css": {"exists": True, "kind": "file"},
    "/home/p/Sites": {"exists": True, "kind": "directory"},
    "/home/p/.ssh/id_ed25519": {"exists": "unknown", "reason": "protected"},
    "/home/p/.ssh/vault.html": {"exists": "unknown", "reason": "protected"},
    "/home/p/Sites/link.html": {"exists": "unknown", "reason": "outside"},  # a link out of the home
    "/etc/passwd": {"exists": "unknown", "reason": "outside"},
}


def stat(asked):
    return ANSWERS.get(asked, {"exists": False, "reason": "not_found"})


files.shell_stat = stat   # the shell's answer, for every check below


class FakeTab:
    def __init__(self, url="about:blank", title=""):
        self.url, self.id, self.session, self.title = url, "t1", "s1", title
        self.dialog = None
        self.navigations = 0
        self.opened_by = None


class FakeBrowser:
    def __init__(self):
        self.tab_now = FakeTab()
        self.sent = []
        self.active = self.tab_now
        self.extra = []

    def tab(self, _tab=None):
        return self.tab_now

    def call(self, method, params=None, session=None):
        self.sent.append((method, params))
        if method == "Page.navigate":
            self.tab_now.url = params["url"]
        return {}

    def pages(self):
        return [self.tab_now] + self.extra

    def attach(self, t):
        return t

    def connect(self):
        pass

    def new_tab(self, url):
        self.sent.append(("new_tab", url))
        return self.tab_now

    def run(self, tab, fn, *args, **kw):
        return {"url": tab.url, "title": "TITLE-SECRET", "elements": []}


class Driver(drv.Driver):
    """The real `go`, `_tab`, `_vet` and `_consequence`, with no browser to start."""

    def __init__(self):
        super().__init__(FakeBrowser(), judge=False)
        self.used = []

    def _use(self, render):
        self.used.append(render)
        self.browser = self.render_browser if render else self.main_browser

    render_browser = None

    def _act(self, tab, doing, fn, **_):
        fn()
        return {"did": doing}

    def _consequence(self, *a, **k):
        return super()._consequence(*a, **k)


class TestLocalPage(unittest.TestCase):
    def driver(self):
        d = Driver()
        d.render_browser = FakeBrowser()
        return d

    def test_a_file_in_the_home_opens_in_the_render_instance_not_the_persons_browser(self):
        d = self.driver()
        d.go("file:///home/p/Sites/index.html")
        self.assertEqual(d.used, [True])
        self.assertEqual(d.render_browser.sent, [("Page.navigate", {"url": "file:///home/p/Sites/index.html"})])
        self.assertEqual(d.main_browser.sent, [], "the person's browser was not touched")
        d.go("file:///home/p/Sites/index.html?x=1#top", new_tab=True)
        self.assertEqual(d.render_browser.sent[-1][0], "new_tab")
        d.go("https://example.com/")
        self.assertEqual(d.used[-1], False, "a web page goes to the desktop's browser")
        self.assertIs(d.browser, d.main_browser)

    def test_outside_the_roots_is_refused_and_nothing_is_sent(self):
        for url, why in [
            ("file:///etc/passwd", "is outside"),
            ("file:///home/p/.ssh/id_ed25519", "is protected"),
            ("file:///home/p/Sites/link.html", "is outside"),          # a symlink escape
            ("file:///home/p/Sites", "is not a file"),                  # no folder listings
            ("file:///home/p/gone.html", "there is no file"),
            ("file:///home/p/Sites/../.ssh/id_ed25519", "`..`"),        # traversal
            ("file:///home/p/Sites/%2e%2e/.ssh/id_ed25519", "`..`"),    # traversal, escaped
            ("file://other-host/home/p/Sites/index.html", "another host"),
            ("file:home/p/Sites/index.html", "full path"),
        ]:
            d = self.driver()
            with self.assertRaises(drv.Refused) as caught:
                d.go(url)
            self.assertIn(why, str(caught.exception), url)
            self.assertEqual((d.main_browser.sent, d.render_browser.sent, d.used), ([], [], []), url)

    def test_other_schemes_are_still_not_pages(self):
        with self.assertRaises(drv.Refused):
            self.driver().go("ftp://example.com/x")

    def test_a_tab_that_a_link_took_outside_the_roots_is_taken_off_it_in_the_render_instance(self):
        d = self.driver()
        d.go("file:///home/p/Sites/index.html")
        d.render_browser.tab_now.url = "file:///etc/passwd"
        with self.assertRaises(drv.Refused) as caught:
            d.text()
        self.assertIn("taken off it", str(caught.exception))
        self.assertEqual(d.render_browser.sent[-1], ("Page.navigate", {"url": "about:blank"}))

    def test_the_persons_own_tab_is_refused_not_blanked(self):
        d = self.driver()
        d.main_browser.tab_now.url = "file:///home/p/.ssh/vault.html"
        with self.assertRaises(drv.Refused) as caught:
            d.text()
        self.assertIn("nothing was read", str(caught.exception))
        self.assertEqual(d.main_browser.sent, [], "the person's tab was left as it was")

    def test_what_an_action_lands_on_is_checked_before_it_is_read_back(self):
        # H1: the page `go` checked redirected to a protected file; the snapshot must not be returned.
        d = self.driver()
        d.go("file:///home/p/Sites/index.html")
        tab = d.render_browser.tab_now
        tab.url = "file:///home/p/.ssh/vault.html"      # where the meta refresh landed
        with self.assertRaises(drv.Refused):
            d._consequence(tab, "went", 0, "file:///home/p/Sites/index.html", {tab.id}, 0.1, "", False)
        self.assertEqual(d.render_browser.sent[-1], ("Page.navigate", {"url": "about:blank"}))
        # And a new tab a click opened onto a protected file, before it is attached or read.
        d = self.driver()
        d.go("file:///home/p/Sites/index.html")
        other = FakeTab("file:///home/p/.ssh/vault.html")
        other.id = "t2"
        d.render_browser.extra = [other]
        with self.assertRaises(drv.Refused):
            d._consequence(d.render_browser.tab_now, "clicked", 0, d.render_browser.tab_now.url, {"t1"}, 0.1, "", False)

    def test_tabs_do_not_name_a_file_a_mind_may_not_read(self):
        d = self.driver()
        d.main_browser.tab_now.url = "file:///home/p/.ssh/vault.html"
        d.main_browser.tab_now.title = "TITLE-SECRET-hunter2"
        shown = d.status()
        text = str(shown)
        self.assertNotIn("TITLE-SECRET", text)
        self.assertNotIn("vault", text)
        self.assertIn("a file a Mind may not read", text)
        d.main_browser.tab_now.url = "file:///home/p/Sites/index.html"
        self.assertIn("index.html", str(d.status()))

    def test_the_render_instance_loads_only_local_files_a_mind_may_read(self):
        render_instance.guard("file:///home/p/Sites/style.css")
        render_instance.guard("data:text/plain,hi")
        for url in ["http://example.com/", "https://x.test/?d=secret", "http://127.0.0.1:9222/json",
                    "ws://127.0.0.1:9222/devtools", "file:///home/p/.ssh/vault.html", "file:///home/p/Sites/",
                    "file:///etc/passwd"]:
            with self.assertRaises(local_page.LocalRefused, msg=url):
                render_instance.guard(url)

    def test_the_render_instance_has_no_network_no_profile_of_the_persons_and_its_own_port(self):
        args = render_instance.args_for("/tmp/prof", 40123)
        self.assertIn("--headless=new", args)
        self.assertIn("--user-data-dir=/tmp/prof", args)
        self.assertIn("--host-resolver-rules=MAP * ~NOTFOUND", args)
        self.assertIn("--proxy-bypass-list=<-loopback>", args)
        self.assertIn("--block-new-web-contents", args)
        self.assertIn("--remote-debugging-port=40123", args)
        self.assertIn("--remote-debugging-address=127.0.0.1", args)
        self.assertNotIn("--remote-debugging-port=9222", args)
        self.assertTrue(any(a.startswith("--proxy-server=") for a in args))

    def test_every_request_of_a_guarded_browser_is_judged_before_it_is_made(self):
        b = cdp.Browser("http://127.0.0.1:1")
        sent = []
        b.call = lambda m, p=None, session=None, **k: sent.append((m, p)) or {}
        b.guard = render_instance.guard
        b._judge("s1", {"requestId": "r1", "request": {"url": "http://example.com/"}})
        b._judge("s1", {"requestId": "r2", "request": {"url": "file:///home/p/Sites/style.css"}})
        import time
        for _ in range(50):
            if len(sent) == 2:
                break
            time.sleep(0.05)
        verdicts = {p["requestId"]: m for m, p in sent}
        self.assertEqual(verdicts, {"r1": "Fetch.failRequest", "r2": "Fetch.continueRequest"})
        # With a guard, a tab is attached with Fetch on, and only then sent to its address.
        src = open(os.path.join(ROOT, "apps", "browser", "yantrik_browser", "cdp.py"), encoding="utf-8").read()
        self.assertIn('"Fetch.enable"', src)
        self.assertIn('"about:blank" if self.guard', src)

    def test_the_check_is_the_one_every_agents_file_meets(self):
        # No second rule: the verdict is `files.may_read_file`'s, asked for any caller.
        with open(os.path.join(ROOT, "apps", "browser", "yantrik_browser", "local_page.py"), encoding="utf-8") as f:
            self.assertIn("files.may_read_file(path, stat, anyone=True)", f.read())
        with self.assertRaises(files.Refusal):
            files.may_read_file("/etc/passwd", stat, anyone=True)


class TestRenderInstanceLive(unittest.TestCase):
    """The real render instance, when a Chromium is on the machine: a Mind's page loads, reaches no
    network, and cannot be steered into a protected file."""

    @classmethod
    def setUpClass(cls):
        try:
            import websocket  # noqa: F401
        except ImportError:
            raise unittest.SkipTest("python3-websocket is not installed")
        if not render_instance.chrome_path():
            raise unittest.SkipTest("no Chromium on this machine")
        import http.server
        import tempfile
        import threading
        cls.hits = []
        hits = cls.hits

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                hits.append(self.path)
                self.send_response(200)
                self.end_headers()

            def log_message(self, *a):
                pass
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.tmp = os.path.realpath(tempfile.mkdtemp(prefix="yantrik-render-test-"))
        os.makedirs(os.path.join(cls.tmp, "Sites"))
        os.makedirs(os.path.join(cls.tmp, ".ssh"))

        def write(rel, text):
            with open(os.path.join(cls.tmp, rel), "w") as f:
                f.write(text)
        port = cls.server.server_address[1]
        write("Sites/index.html", "<html><title>MY-PAGE</title><body><h1>hello from a mind</h1></body></html>")
        write("Sites/net.html", "<html><title>NET</title><body>net<script>fetch('http://127.0.0.1:%d/script');"
              "</script><img src='http://127.0.0.1:%d/img'></body></html>" % (port, port))
        write(".ssh/vault.html", "<html><title>TITLE-SECRET-hunter2</title><body>SECRET-BODY</body></html>")
        write("Sites/redir.html", "<html><meta http-equiv=refresh content='0;url=file://%s/.ssh/vault.html'>"
              "<title>REDIR</title></html>" % cls.tmp)
        real_stat = ANSWERS

        def live_stat(asked):
            if asked.startswith(cls.tmp + "/.ssh"):
                return {"exists": "unknown", "reason": "protected"}
            if asked.startswith(cls.tmp + "/") and os.path.isfile(asked):
                return {"exists": True, "kind": "file"}
            return real_stat.get(asked, {"exists": False, "reason": "not_found"})
        cls.saved_stat = files.shell_stat
        files.shell_stat = live_stat

    @classmethod
    def tearDownClass(cls):
        files.shell_stat = cls.saved_stat
        cls.server.shutdown()
        shutil_rmtree = __import__("shutil").rmtree
        shutil_rmtree(cls.tmp, ignore_errors=True)

    def setUp(self):
        self.d = drv.Driver(cdp.Browser("http://127.0.0.1:1"), judge=False)
        # A CI container has no usable sandbox; the desktop's instance keeps it on.
        self.d.instance = render_instance.Instance(extra_args=("--no-sandbox",))
        self.addCleanup(self.d.instance.stop)

    def url(self, rel):
        return "file://%s/%s" % (self.tmp, rel)

    def test_a_mind_page_loads_in_its_own_browser(self):
        self.d.go(self.url("Sites/index.html"))
        self.assertIn("hello from a mind", str(self.d.text()))
        self.assertIsNot(self.d.browser, self.d.main_browser)

    def test_a_page_reaches_no_network_not_even_loopback(self):
        self.d.go(self.url("Sites/net.html"))
        import time
        time.sleep(1.5)
        self.assertEqual(self.hits, [], "a page of a Mind reached the network")

    def test_a_page_that_steers_to_a_protected_file_reads_back_nothing(self):
        try:
            got = str(self.d.go(self.url("Sites/redir.html")))
        except drv.Refused as e:
            got = str(e)
        import time
        time.sleep(1.0)
        try:
            got += str(self.d.read())
        except drv.Refused as e:
            got += str(e)
        self.assertNotIn("SECRET-BODY", got)
        self.assertNotIn("hunter2", got)


if __name__ == "__main__":
    unittest.main()
