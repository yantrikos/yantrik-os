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


class Bench:
    """One browser and one site for the whole run."""

    @classmethod
    def start(cls):
        cls.tmp = tempfile.mkdtemp(prefix="yb-test-")
        handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=HERE)
        handler.log_message = lambda *a, **k: None
        cls.http = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=cls.http.serve_forever, daemon=True).start()
        cls.site = "http://127.0.0.1:%d" % cls.http.server_address[1]
        port = free_port()
        cls.cdp = "http://127.0.0.1:%d" % port
        cls.proc = subprocess.Popen([
            chrome(), "--headless=new", "--no-first-run", "--no-default-browser-check",
            "--remote-debugging-address=127.0.0.1", "--remote-debugging-port=%d" % port,
            "--remote-allow-origins=%s" % cls.cdp, "--user-data-dir=%s" % cls.tmp,
            "--window-size=1280,800", "about:blank"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.time() + 20
        while time.time() < deadline:
            try:
                cdp.Browser(cls.cdp)._version()
                break
            except cdp.BrowserClosed:
                time.sleep(0.2)
        cls.d = drv.Driver(cdp.Browser(cls.cdp), judge=False)

    @classmethod
    def stop(cls):
        cls.d.browser.close()
        cls.proc.kill()
        cls.proc.wait()
        cls.http.shutdown()
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


class Tricks(unittest.TestCase):
    """What the security review of #477 tried, each one: a way to press a commitment without the
    person's card, or to confuse the reader. None of them may work."""

    @classmethod
    def setUpClass(cls):
        cls.d, cls.site = Bench.d, Bench.site

    def setUp(self):
        self.d.go(self.site + "/pages/tricky.html")

    def ref(self, name, role=None):
        for e in self.d.read(all=True)["elements"]:
            if e.get("name") == name and (role is None or e["role"] == role):
                return e["ref"]
        self.fail("no %r in %s" % (name, [e.get("name") for e in self.d.read(all=True)["elements"]]))

    def title(self):
        """What the page's log says a press did: "untouched" when nothing was pressed. (The page's
        own title cannot be set: its <img name="title"> shadows document.title, on purpose.)"""
        text = self.d.text()["text"]
        return "Tricky Page" if "untouched" in text else text

    def refused(self, fn, *args, **kwargs):
        with self.assertRaises(drv.PageRefused) as caught:
            fn(*args, **kwargs)
        return str(caught.exception)

    def test_a_bot_check_sees_a_person(self):
        # The page checks itself once on load and again when its button is pressed — by a
        # driver that has attached, read and clicked.
        self.d.go(self.site + "/pages/detect.html")
        self.d.click(self.ref("Check"))
        time.sleep(0.5)
        self.assertEqual(self.d.read()["title"], "HUMAN", self.d.text()["text"])

    def test_the_page_cannot_fake_the_reader_or_its_title(self):
        page = self.d.read()
        self.assertEqual(page["title"], "Tricky Page")
        self.assertIn("Tricky Page", self.d.status()["tabs"][0]["title"])

    def test_a_link_that_deletes_is_a_commitment(self):
        self.assertIn("commit", self.refused(self.d.click, self.ref("Delete")))
        self.assertEqual(self.title(), "Tricky Page")

    def test_a_link_to_a_form_is_not_but_a_link_that_orders_is(self):
        by = {e["name"]: e for e in self.d.read(all=True)["elements"]}
        self.assertNotIn("commitment", by["submit"])
        self.assertIn("commitment", by["Place order"])

    def test_a_press_that_lands_on_a_commitment_names_it(self):
        card = next(e["ref"] for e in self.d.read(all=True)["elements"] if e["role"] == "clickable")
        message = self.refused(self.d.click, card)
        self.assertIn("Buy now", message)
        self.assertEqual(self.title(), "Tricky Page")

    def test_enter_in_a_composer_with_a_send_button_is_refused_after_typing(self):
        message = self.refused(self.d.type, self.ref("Message"), "hi", enter=True)
        self.assertIn("Send", message)
        self.assertEqual(self.title(), "Tricky Page")

    def test_enter_reaches_a_button_tied_to_the_form_from_outside(self):
        message = self.refused(self.d.type, self.ref("Amount"), "5", enter=True)
        self.assertIn("Pay now", message)
        self.assertEqual(self.title(), "Tricky Page")

    def test_space_on_a_focused_commitment_is_refused(self):
        buy = self.ref("Buy now")
        self.assertIn("Space", self.refused(self.d.press, "space", buy))
        self.assertEqual(self.title(), "Tricky Page")

    def test_a_list_that_acts_on_change_does_not_get_a_commitment_chosen(self):
        self.refused(self.d.select, self.ref("Bulk action"), "Delete selected")
        self.assertEqual(self.title(), "Tricky Page")
        self.d.select(self.ref("Bulk action"), "Archive selected")

    def test_a_prompt_that_reads_as_a_commitment_goes_to_commit(self):
        self.d.click(self.ref("Wipe data"))
        self.refused(self.d.dialog, True, "DELETE")
        self.d.dialog(False)
        self.assertEqual(self.title(), "Tricky Page")

    def test_commit_on_a_dialog_needs_its_whole_message(self):
        self.d.click(self.ref("Wipe data"))
        for label in ("", "Type"):
            self.refused(self.d.commit, "dialog", label, "127.0.0.1")
        self.d.dialog(False)

    def test_words_are_words(self):
        by = {e["name"]: e for e in self.d.read(all=True)["elements"]}
        self.assertEqual(by["Post review"].get("commitment"), "post", "view inside review is not a word")
        zw = next(e for e in self.d.read(all=True)["elements"] if e.get("ref") and "uy" in e.get("name", "")
                  and e["name"] != "Buy now")
        self.assertIn("commitment", zw, "a zero-width space does not hide Buy")
        self.assertIn("Submit", by, "an input type=submit with no value is named as the browser names it")


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


if __name__ == "__main__":
    if not chrome():
        print("no google-chrome or chromium on PATH: the live browser tests are skipped")
        unittest.main(argv=[sys.argv[0], "Words"])
    unittest.main()
