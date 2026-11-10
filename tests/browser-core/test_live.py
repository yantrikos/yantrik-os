#!/usr/bin/env python3
"""The browser surface against a real browser: headless Chrome, and pages that do what the web
does to an agent — shadow DOM, a frame, a cookie wall over the page, a controlled field, a
checkout, a confirm dialog, a link that opens a tab, a div made clickable by hand.

    python3 tests/browser-core/test_live.py

Needs google-chrome or chromium on PATH and python3-websocket. Skips (exit 0, says so) without
them. Nothing here touches a desktop: its own browser, its own profile, its own port.
"""

import functools
import http.server
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path[:0] = [os.path.join(ROOT, "apps", "browser"), os.path.join(ROOT, "sdk", "python")]

from yantrik_browser import cdp, commit, driver as drv  # noqa: E402


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def chrome():
    for name in ("google-chrome", "chromium", "chromium-browser"):
        path = shutil.which(name)
        if path:
            return path
    return None


def log_tail(path, lines=20, limit=2048):
    """The last few lines of a file, for a failure message."""
    try:
        with open(path, "rb") as f:
            f.seek(0, os.SEEK_END)
            f.seek(max(0, f.tell() - limit))
            text = f.read().decode("utf-8", "replace").strip()
    except OSError:
        return "(no output)"
    return "\n".join(text.splitlines()[-lines:]) or "(no output)"


def wait_ready(browser, proc, log_path, timeout=30, interval=0.1):
    """Wait until the driver can really use the browser: connect() answers and one page target
    exists. `/json/version` answering on its own is not enough — the tests need the websocket
    and the about:blank tab. Fails at once if Chrome exits, quoting its own output."""
    deadline = time.monotonic() + timeout
    while True:
        if proc.poll() is not None:
            browser.close()
            raise RuntimeError("browser exited with code %s before it was ready: %s"
                               % (proc.returncode, log_tail(log_path)))
        try:
            browser.connect()
            if browser.pages():
                return
        except (cdp.BrowserClosed, cdp.CdpError):
            pass
        if time.monotonic() > deadline:
            browser.close()
            raise RuntimeError("browser did not start in %d s: %s" % (timeout, log_tail(log_path)))
        time.sleep(interval)


class Site(http.server.SimpleHTTPRequestHandler):
    """The test pages, plus /counter.js: a script the server counts requests to and tells the
    browser to cache for an hour — what a reload that bypasses the cache must fetch again."""

    counter = 0

    def do_GET(self):
        if self.path.split("?")[0] == "/counter.js":
            Site.counter += 1
            body = ("document.addEventListener('DOMContentLoaded', () => { document.body.textContent = "
                    "'count %d'; });" % Site.counter).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/javascript")
            self.send_header("Cache-Control", "max-age=3600")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        super().do_GET()

    def log_message(self, *a, **k):
        pass


class Bench:
    """One browser and one site for the whole run."""

    @classmethod
    def start(cls):
        cls.tmp = tempfile.mkdtemp(prefix="yb-test-")
        handler = functools.partial(Site, directory=HERE)
        cls.http = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=cls.http.serve_forever, daemon=True).start()
        cls.site = "http://127.0.0.1:%d" % cls.http.server_address[1]
        port = free_port()
        cls.cdp = "http://127.0.0.1:%d" % port
        cls.log = open(os.path.join(cls.tmp, "chrome.log"), "wb")
        cls.proc = subprocess.Popen([
            chrome(), "--headless=new", "--no-first-run", "--no-default-browser-check",
            "--remote-debugging-address=127.0.0.1", "--remote-debugging-port=%d" % port,
            "--remote-allow-origins=%s" % cls.cdp, "--user-data-dir=%s" % cls.tmp,
            "--window-size=1280,800", "about:blank"],
            stdout=cls.log, stderr=cls.log)
        browser = cdp.Browser(cls.cdp)
        try:
            wait_ready(browser, cls.proc, cls.log.name)
        except Exception:
            # setUpModule raising skips tearDownModule: tear down what start made here, or the
            # Chrome process and the server leak.
            cls.proc.kill()
            cls.proc.wait()
            cls.http.shutdown()
            cls.log.close()
            shutil.rmtree(cls.tmp, ignore_errors=True)
            raise
        cls.d = drv.Driver(browser, judge=False)

    @classmethod
    def stop(cls):
        cls.d.sides.fixed.close()     # the one browser, whatever thread last held it
        cls.proc.kill()
        cls.proc.wait()
        cls.http.shutdown()
        cls.log.close()
        shutil.rmtree(cls.tmp, ignore_errors=True)


def setUpModule():
    if chrome():
        Bench.start()


def tearDownModule():
    if chrome():
        Bench.stop()


class Live(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.d, cls.site = Bench.d, Bench.site

    def open_shop(self):
        out = self.d.go(self.site + "/pages/shop.html")
        self.assertTrue(out.get("navigated"), out)
        return out

    def ref(self, name, role=None, page=None):
        page = page or self.d.read(all=True)
        for e in page["elements"]:
            if e.get("name") == name and (role is None or e["role"] == role):
                return e["ref"]
        self.fail("no element named %r (role %r) in %s" % (name, role, [e.get("name") for e in page["elements"]]))

    # ── reading ──

    def test_go_answers_with_the_page(self):
        out = self.open_shop()
        self.assertEqual(out["title"], "Test Shop")
        names = [e["name"] for e in out["page"]["elements"]]
        self.assertIn("Sign in", names)
        self.assertIn("Test Shop", names, "headings say where things are")

    def test_refs_stay_the_same_across_reads(self):
        self.open_shop()
        a = self.ref("Sign in", "button")
        self.d.scroll("down")
        self.assertEqual(self.ref("Sign in", "button"), a)

    def test_shadow_dom_and_a_same_origin_frame_are_read_and_pressed(self):
        self.open_shop()
        page = self.d.read(all=True)
        shadow = self.ref("Shadow button", page=page)
        framed = self.ref("Frame button", page=page)
        out = self.d.click(shadow)
        self.assertIn("Shadow pressed", " ".join(out.get("appeared", [])) + str(self.d.text()["text"]), out)
        out = self.d.click(framed)
        self.assertIn("Frame pressed", self.d.text()["text"], out)

    def test_below_the_fold_is_counted_and_find_reaches_it(self):
        self.open_shop()
        page = self.d.read()
        self.assertGreater(page["below"], 0)
        found = self.d.find("Far down")["found"]
        self.assertEqual(len(found), 1)
        self.assertFalse(found[0]["in_view"])

    def open_fold(self):
        out = self.d.go(self.site + "/pages/fold.html")
        self.assertTrue(out.get("navigated"), out)

    def test_find_places_a_match_below_the_fold(self):
        self.open_fold()
        got = self.d.find("More information")
        self.assertEqual(len(got["found"]), 1, got)
        self.assertEqual(got["total"], 1)
        hit = got["found"][0]
        self.assertFalse(hit["in_view"])
        self.assertEqual(hit["place"]["dir"], "below")
        self.assertGreaterEqual(hit["place"]["screens"], 2)

    def test_find_leaves_a_match_in_view_unplaced(self):
        self.open_fold()
        hit = self.d.find("Example Domain")["found"][0]
        self.assertTrue(hit["in_view"])
        self.assertIsNone(hit["place"])

    def test_find_reaches_words_that_only_the_text_holds(self):
        self.open_fold()
        got = self.d.find("quarterly ledger")
        self.assertEqual(got["found"], [])
        self.assertEqual(len(got["text"]), 1, got)
        self.assertEqual(got["text"][0]["place"]["dir"], "below")

    def test_find_with_no_match_anywhere_is_empty(self):
        self.open_fold()
        got = self.d.find("no such words")
        self.assertEqual((got["found"], got["text"], got["total"]), ([], [], 0))

    def test_find_counts_every_match_and_lists_a_bounded_few(self):
        self.open_fold()
        got = self.d.find("e")
        self.assertLessEqual(len(got["found"]), 40)
        self.assertGreaterEqual(got["total"], len(got["found"]))

    def test_a_click_on_a_match_below_the_fold_scrolls_to_it_first(self):
        self.open_fold()
        ref = self.d.find("More information")["found"][0]["ref"]
        out = self.d.click(ref)
        self.assertIn("clicked link", out["did"], out)
        self.assertIn("/info", out.get("url", ""), out)

    def test_states_are_read(self):
        self.open_shop()
        page = self.d.read(all=True)
        by = {e["name"]: e for e in page["elements"]}
        self.assertTrue(by["Apply coupon"].get("disabled"))
        self.assertEqual(by["Size"].get("value"), "Small")
        self.assertEqual(by["Gift wrap"].get("checked"), False)
        self.assertTrue(by["Password"].get("password"))

    # ── acting ──

    def test_a_click_answers_with_what_appeared(self):
        self.open_shop()
        self.d.type(self.ref("Password"), "wrong")
        out = self.d.click(self.ref("Sign in", "button"))
        self.assertIn("Invalid password", out.get("appeared", []), out)

    def test_typing_reaches_a_controlled_field(self):
        self.open_shop()
        out = self.d.type(self.ref("Nickname"), "zed")
        self.assertIn("Held: zed", self.d.text()["text"], out)
        out = self.d.type(self.ref("Nickname"), "amy")
        self.assertIn("Held: amy", self.d.text()["text"], "clear replaces what was there")

    def test_typing_with_enter_submits_a_harmless_form(self):
        self.open_shop()
        out = self.d.type(self.ref("Search products"), "lamp", enter=True)
        self.assertIn("Results for lamp", self.d.text()["text"], out)

    def test_select_and_checkbox(self):
        self.open_shop()
        out = self.d.select(self.ref("Size"), "Large")
        self.assertEqual(out["did"], 'chose "Large" in %s' % self.ref("Size"))
        self.d.click(self.ref("Gift wrap"))
        by = {e["name"]: e for e in self.d.read(all=True)["elements"]}
        self.assertEqual(by["Size"]["value"], "Large")
        self.assertTrue(by["Gift wrap"]["checked"])

    def test_a_handmade_clickable_div_is_found_and_pressed(self):
        self.open_shop()
        ref = self.ref("Save for later", "clickable")
        self.d.click(ref)
        self.assertIn("Added to wishlist", self.d.text()["text"])

    def test_a_disabled_control_is_refused_with_why(self):
        self.open_shop()
        with self.assertRaises(drv.Refused) as caught:
            self.d.click(self.ref("Apply coupon"))
        self.assertIn("disabled", str(caught.exception))

    def test_a_covered_control_names_what_covers_it(self):
        self.d.go(self.site + "/pages/cookies.html")
        read = self.ref("Read the article")
        with self.assertRaises(drv.Refused) as caught:
            self.d.click(read)
        self.assertIn("Cookie consent", str(caught.exception))
        self.d.click(self.ref("Accept all"))
        self.d.click(read)
        self.assertIn("Reading", self.d.text()["text"])

    def test_a_link_to_a_new_tab_follows_it(self):
        self.open_shop()
        out = self.d.click(self.ref("Open other in a tab"))
        self.assertTrue(out.get("new_tabs"), out)
        self.assertEqual(out["title"], "Other Page")
        self.d.close_tab(out["now_in_tab"])

    def test_back_and_forward(self):
        self.open_shop()
        self.d.click(self.ref("Other page"))
        self.assertEqual(self.d.history(-1)["title"], "Test Shop")
        self.assertEqual(self.d.history(1)["title"], "Other Page")

    def test_a_gone_ref_says_to_read_again(self):
        self.open_shop()
        ref = self.ref("Sign in", "button")
        self.d.go(self.site + "/pages/other.html")
        with self.assertRaises(drv.Refused) as caught:
            self.d.click(ref)
        self.assertIn("read it again", str(caught.exception).lower())

    # ── commitments ──

    def test_a_commitment_is_refused_by_click_and_by_enter(self):
        self.open_shop()
        place = self.ref("Place order")
        with self.assertRaises(drv.Refused) as caught:
            self.d.click(place)
        self.assertIn("commit ref=%s" % place, str(caught.exception))
        with self.assertRaises(drv.Refused) as caught:
            self.d.type(self.ref("Order note"), "leave at door", enter=True)
        self.assertIn("Place order", str(caught.exception))
        self.assertEqual(self.d.read()["title"], "Test Shop", "nothing was ordered")

    def test_commit_presses_only_what_the_person_was_shown(self):
        self.open_shop()
        place = self.ref("Place order")
        host = "127.0.0.1"
        with self.assertRaises(drv.Refused):
            self.d.commit(place, "Place order", "other.example")
        with self.assertRaises(drv.Refused):
            self.d.commit(place, "Pay now", host)
        out = self.d.commit(place, "Place order", host)
        self.assertEqual(out["title"], "ORDER PLACED", out)

    def test_a_dialog_that_reads_as_a_commitment_goes_to_commit(self):
        self.open_shop()
        out = self.d.click(self.ref("Close my account"))
        self.assertEqual(out["dialog"]["type"], "confirm", out)
        with self.assertRaises(drv.Refused) as caught:
            self.d.dialog(accept=True)
        self.assertIn("commit ref=dialog", str(caught.exception))
        self.d.dialog(accept=False)
        self.assertEqual(self.d.read()["title"], "Test Shop")

    def test_an_alert_is_just_answered(self):
        self.open_shop()
        out = self.d.click(self.ref("Say hello"))
        self.assertEqual(out["dialog"]["message"], "Hello there")
        self.d.dialog(accept=True)
        self.assertIsNone(self.d.read().get("dialog"))


class ModelJudges(unittest.TestCase):
    """The decision model, asked when the words say nothing (judgement.py). A stand-in answers
    here: yes when the text around the control says the card will be charged, and an abstention
    otherwise — the verdict's wire form, as the shell's `decide` gives it."""

    @classmethod
    def setUpClass(cls):
        from yantrik_browser import judgement
        cls.site = Bench.site
        cls.asked = []

        def fake(state):
            cls.asked.append(state)
            charged = "charged" in state.get("nearby_text", "")
            answer = {"type": "noul", "yes": 0.92} if charged else {"type": "abstain", "reason": "could not tell"}
            return {"result": {"result": {"answers": {"commit": answer},
                                          "by": {"provider": "kev", "model": "kev-latest"}}}}
        cls.d = drv.Driver(Bench.d.browser, judge=judgement.Judgement(ask=fake))

    def ref(self, name):
        for e in self.d.read(all=True)["elements"]:
            if e.get("name") == name:
                return e["ref"]
        self.fail("no %r" % name)

    def test_a_commitment_the_words_miss_is_caught_by_the_model(self):
        self.d.go(self.site + "/pages/tricky.html")
        with self.assertRaises(drv.PageRefused) as caught:
            self.d.click(self.ref("Continue"))
        self.assertIn("judged a commitment by kev kev-latest", str(caught.exception))
        self.assertIn("untouched", self.d.text()["text"], "nothing was charged")
        state = self.asked[-1]
        self.assertEqual(state["control"]["label"], "Continue")
        self.assertIn("Payment", state["page"]["heading"])

    def test_an_abstention_leaves_the_press_to_the_words(self):
        self.d.go(self.site + "/pages/tricky.html")
        self.d.click(self.ref("Next chapter"))
        self.assertIn("NEXT CHAPTER", self.d.text()["text"])

    def test_the_model_cannot_take_away_a_card_the_words_raise(self):
        from yantrik_browser import judgement
        no = lambda state: {"result": {"result": {"answers": {"commit": {"type": "noul", "yes": 0.0}}, "by": {}}}}
        d = drv.Driver(Bench.d.browser, judge=judgement.Judgement(ask=no))
        d.go(self.site + "/pages/shop.html")
        place = next(e["ref"] for e in d.read(all=True)["elements"] if e.get("name") == "Place order")
        with self.assertRaises(drv.PageRefused):
            d.click(place)

    def test_a_link_that_navigates_is_not_put_to_the_model(self):
        self.asked.clear()
        self.d.go(self.site + "/pages/shop.html")
        self.d.click(next(e["ref"] for e in self.d.read(all=True)["elements"] if e.get("name") == "Other page"))
        self.assertEqual(self.asked, [])

    def test_no_shell_to_ask_is_the_words_alone(self):
        from yantrik_browser import judgement

        def down(state):
            raise ConnectionError("no shell")
        d = drv.Driver(Bench.d.browser, judge=judgement.Judgement(ask=down))
        d.go(self.site + "/pages/tricky.html")
        d.click(next(e["ref"] for e in d.read(all=True)["elements"] if e.get("name") == "Next chapter"))
        self.assertIn("NEXT CHAPTER", d.text()["text"])


class OverTheSocket(unittest.TestCase):
    """The same browser, reached as every app is: app.describe / app.act on its socket, under
    the gate. What the driver refuses is refused here in the same words, and what the gate adds —
    the grade, the mode, the grant — is on top."""

    @classmethod
    def setUpClass(cls):
        from yantrik_browser import service
        from yantrik_surface import wire
        cls.wire = wire
        cls.dir = tempfile.mkdtemp(prefix="yb-sock-")
        cls.settings = os.path.join(cls.dir, "settings.yaml")
        cls.mode = os.path.join(cls.dir, "mind-mode.json")
        with open(cls.settings, "w") as f:
            f.write("tool_permission: dangerous\n")
        cls.surface, _ = service.build(Bench.d, socket_path=os.path.join(cls.dir, "app-browser.sock"),
                                       settings_path=cls.settings, mode_path=cls.mode)
        cls.surface.serve_in_thread()
        cls.sock = cls.surface.socket_path()

    @classmethod
    def tearDownClass(cls):
        cls.surface.stop()
        shutil.rmtree(cls.dir, ignore_errors=True)

    def call(self, method, params=None, timeout=40):
        return self.wire.call_once(self.sock, method, params or {}, timeout=timeout)

    def act(self, action, **args):
        return self.call("app.act", {"action": action, "args": args})

    def set_mode(self, mode):
        with open(self.mode, "w") as f:
            f.write('{"mode": "%s", "session_rules": []}' % mode)

    def test_describe_names_the_page_and_every_action_with_its_grade(self):
        self.set_mode("ask")
        self.act("go", url=Bench.site + "/pages/shop.html")
        d = self.call("app.describe")["result"]
        self.assertIn("Test Shop", d["summary"])
        grades = {a["name"]: a["permission"] for a in d["actions"]}
        self.assertEqual(grades["read"], "safe")
        self.assertEqual(grades["console"], "safe")
        self.assertEqual(grades["screenshot"], "safe")
        self.assertEqual(grades["click"], "standard")
        self.assertEqual(grades["commit"], "sensitive")

    def test_a_click_and_its_consequence_arrive_as_the_result(self):
        self.set_mode("ask")
        self.act("go", url=Bench.site + "/pages/shop.html")
        page = self.act("read", all=True)["result"]["result"]
        ref = next(e["ref"] for e in page["elements"] if e["name"] == "Save for later")
        got = self.act("click", ref=ref)["result"]
        self.assertIn("Added to wishlist", got["result"].get("appeared", []), got)

    def test_commit_asks_even_in_auto(self):
        self.set_mode("auto")
        self.act("go", url=Bench.site + "/pages/shop.html")
        page = self.act("read", all=True)["result"]["result"]
        ref = next(e["ref"] for e in page["elements"] if e["name"] == "Place order")
        refused = self.act("commit", ref=ref, label="Place order", site="127.0.0.1")
        self.assertIn("error", refused)
        self.assertIn("GRANT:", refused["error"]["message"])
        self.assertEqual(self.act("read")["result"]["result"]["title"], "Test Shop", "nothing was ordered")

    def test_a_refusal_arrives_in_the_drivers_words(self):
        self.set_mode("ask")
        self.act("go", url=Bench.site + "/pages/shop.html")
        got = self.act("click", ref="e99999")
        self.assertIn("no longer on the page", got["error"]["message"])


class Words(unittest.TestCase):
    def test_a_link_that_goes_somewhere_is_not_a_commitment(self):
        self.assertIsNone(drv.commitment_of({"role": "link", "name": "submit", "href": "https://x/submit"}))
        self.assertEqual(drv.commitment_of({"role": "link", "name": "Delete"}), "delete")
        self.assertEqual(drv.commitment_of({"role": "button", "name": "Submit"}), "submit")

    def test_commitments(self):
        for label in ("Place order", "Buy now", "Send", "Delete", "Pay $12.00", "Confirm purchase"):
            self.assertTrue(commit.reads_as_commitment(label), label)
        for label in ("Order history", "Sign in", "Accept all", "Search", "Sender name", "Deals"):
            self.assertIsNone(commit.reads_as_commitment(label), label)


class FakeBrowser:
    """A cdp.Browser that becomes ready on schedule: connect() is refused `fails` times then
    works; pages() is empty `empty_pages` times before the tab shows up."""

    def __init__(self, fails=0, empty_pages=0):
        self.fails = fails
        self.empty_pages = empty_pages
        self.connects = 0
        self.closed = 0

    def connect(self):
        self.connects += 1
        if self.connects <= self.fails:
            raise cdp.BrowserClosed()

    def pages(self):
        if self.empty_pages > 0:
            self.empty_pages -= 1
            return []
        return [object()]

    def close(self):
        self.closed += 1


class FakeProc:
    def __init__(self, exit_code=None):
        self.exit_code = exit_code
        self.returncode = exit_code  # a real Popen fills this as poll() sees the exit

    def poll(self):
        return self.exit_code


class ReadyWait(unittest.TestCase):
    """wait_ready against fakes: no Chrome, no waiting longer than the poll interval."""

    def log(self, text):
        f = tempfile.NamedTemporaryFile("w", suffix=".log", delete=False)
        f.write(text)
        f.close()
        self.addCleanup(os.unlink, f.name)
        return f.name

    def test_returns_once_connect_and_a_page_both_work(self):
        b = FakeBrowser(fails=2, empty_pages=1)
        wait_ready(b, FakeProc(), self.log("ready\n"), timeout=5, interval=0.01)
        self.assertEqual(b.connects, 4)  # refused twice, then a tab-less pass, then ready
        self.assertEqual(b.closed, 0)

    def test_early_exit_names_the_code_and_quotes_the_log(self):
        b = FakeBrowser()
        with self.assertRaises(RuntimeError) as got:
            wait_ready(b, FakeProc(exit_code=1), self.log("DevToolsActivePort file doesn't exist\n"),
                       timeout=5, interval=0.01)
        self.assertIn("exited with code 1", str(got.exception))
        self.assertIn("DevToolsActivePort", str(got.exception))
        self.assertEqual(b.closed, 1)

    def test_timeout_quotes_the_log(self):
        b = FakeBrowser(fails=10 ** 6)
        started = time.monotonic()
        with self.assertRaises(RuntimeError) as got:
            wait_ready(b, FakeProc(), self.log("still trying\n"), timeout=0.5, interval=0.01)
        self.assertIn("did not start in", str(got.exception))
        self.assertIn("still trying", str(got.exception))
        self.assertEqual(b.closed, 1)
        self.assertLess(time.monotonic() - started, 5)


if __name__ == "__main__":
    if not chrome():
        print("no google-chrome or chromium on PATH: the live browser tests are skipped")
        unittest.main(argv=[sys.argv[0], "Words", "ReadyWait"])  # neither needs Chrome
    unittest.main()
