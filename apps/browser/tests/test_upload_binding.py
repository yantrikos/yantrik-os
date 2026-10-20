"""The card binds `upload` (#874): what the person was shown is what goes, or nothing does.

The card names the file (absolute path, size, sha256 prefix), the site and the control's label,
and those are `upload`'s own arguments, so the person's grant is bound to them. These tests drive
the driver against a stand-in browser that records every DevTools command, so "nothing was
uploaded" is checked as "no DOM.setFileInputFiles was sent", and the service's real dispatch for
"refused before any card".
"""

import json
import os
import tempfile
import unittest

import fakes
from yantrik_browser import driver as drv
from yantrik_browser import service, uploads
from yantrik_surface import wire


class Bench(unittest.TestCase):
    url = "https://github.com/settings/profile"
    control = None

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="yb-bind-")
        self.addCleanup(self.tmp.cleanup)
        self.home = os.path.realpath(self.tmp.name)
        self.runtime = os.path.join(self.home, "run")
        os.mkdir(self.runtime, 0o700)
        os.makedirs(os.path.join(self.home, "Pictures"))
        self.photo = os.path.join(self.home, "Pictures", "emblem.png")
        with open(self.photo, "wb") as f:
            f.write(b"\x89PNG the emblem")
        self.browser = fakes.Browser(self.url, self.control)
        self.d = drv.Driver(self.browser, judge=False, stat=fakes.files_stat(self.home),
                            runtime=self.runtime)

    def card(self, path=None):
        """What upload_check answers: the card's arguments."""
        got = self.d.upload_check("e5", path or self.photo)
        return {k: got[k] for k in ("path", "size", "sha256", "site", "label")}

    def upload(self, card, ref="e5"):
        return self.d.upload(ref, card["path"], card["size"], card["sha256"], card["site"], card["label"])

    def refused(self, card, ref="e5"):
        with self.assertRaises(drv.PageRefused) as caught:
            self.upload(card, ref)
        self.assertEqual(self.browser.set_files, [], "a file was put into the page")
        return str(caught.exception)


class WhatTheCardNames(Bench):
    def test_upload_check_names_file_size_hash_site_and_label(self):
        got = self.d.upload_check("e5", self.photo)
        self.assertEqual(got["path"], self.photo)
        self.assertEqual(got["size"], len(b"\x89PNG the emblem"))
        self.assertEqual(len(got["sha256"]), uploads.SHA_SHOWN)
        self.assertEqual(got["site"], "github.com")
        self.assertEqual(got["label"], "Upload a photo…")
        self.assertEqual(got["accepts"], "image/*")
        self.assertEqual(self.browser.calls, [], "upload_check put nothing into the page")

    def test_upload_check_refuses_a_protected_path_and_touches_nothing(self):
        os.makedirs(os.path.join(self.home, ".ssh"))
        with open(os.path.join(self.home, ".ssh", "id_ed25519"), "w") as f:
            f.write("key")
        with self.assertRaises(drv.PageRefused) as caught:
            self.d.upload_check("e5", os.path.join(self.home, ".ssh", "id_ed25519"))
        self.assertIn("protected place", str(caught.exception))
        self.assertEqual(self.browser.calls, [])


class TheBinding(Bench):
    def test_the_file_the_card_named_is_put_into_the_input(self):
        out = self.upload(self.card())
        (sent,) = self.browser.set_files
        self.assertEqual(sent["objectId"], "obj-1")
        (staged,) = sent["files"]
        self.assertTrue(staged.startswith(self.runtime), staged)
        self.assertEqual(os.path.basename(staged), "emblem.png")
        self.assertEqual(out["chosen"], ["emblem.png"])
        self.assertEqual(out["submitted"], False)
        self.assertIn("emblem.png", out["did"])
        # It never presses anything on the direct route.
        self.assertFalse([c for c in self.browser.calls if c[0] == "Input.dispatchMouseEvent"])

    def test_a_file_changed_after_the_card_uploads_nothing(self):
        card = self.card()
        with open(self.photo, "wb") as f:
            f.write(b"\x89PNG not the emblem")
        message = self.refused(card)
        self.assertIn("not the file the person was shown", message)
        self.assertIn("Nothing was uploaded".lower(), message.lower())

    def test_a_file_changed_to_the_same_size_uploads_nothing(self):
        card = self.card()
        with open(self.photo, "wb") as f:
            f.write(b"\x89PNG the emblen")
        self.assertIn("not the file the person was shown", self.refused(card))

    def test_another_site_uploads_nothing(self):
        card = self.card()
        self.browser.t.url = "https://evil.example/upload"
        self.assertIn("this tab is on evil.example, not github.com", self.refused(card))

    def test_a_changed_label_uploads_nothing(self):
        card = self.card()
        self.browser.control["name"] = "Upload your SSH key"
        self.assertIn("now reads", self.refused(card))

    def test_a_ref_that_is_gone_uploads_nothing(self):
        card = self.card()
        self.assertIn("no longer on the page", self.refused(card, ref="e9"))

    def test_a_path_swapped_for_a_link_after_the_card_uploads_nothing(self):
        card = self.card()
        os.makedirs(os.path.join(self.home, ".ssh"))
        key = os.path.join(self.home, ".ssh", "id_ed25519")
        with open(key, "wb") as f:
            f.write(b"\x89PNG the emblem")      # same bytes, so only the link can stop it
        os.unlink(self.photo)
        os.symlink(key, self.photo)
        self.assertIn("link", self.refused(card))

    def test_a_short_or_made_up_hash_uploads_nothing(self):
        card = self.card()
        for bad in ("", "abc", "zz" * 8, card["sha256"][:8]):
            self.assertIn("sha256 must be", self.refused(dict(card, sha256=bad)), bad)

    def test_a_path_not_as_the_card_showed_it_uploads_nothing(self):
        card = self.card()
        self.assertIn("absolute path", self.refused(dict(card, path=self.photo + "/")))


class ThroughTheChooser(Bench):
    control = {"ref": "e5", "role": "button", "name": "Attach files", "direct": False}

    def test_a_button_that_opens_a_chooser_is_answered_and_interception_is_turned_off(self):
        out = self.upload(self.card())
        methods = [c[0] for c in self.browser.calls]
        on = methods.index("Page.setInterceptFileChooserDialog")
        self.assertEqual(self.browser.calls[on][1], {"enabled": True})
        self.assertLess(on, methods.index("Input.dispatchMouseEvent"))
        (sent,) = self.browser.set_files
        self.assertEqual(sent["backendNodeId"], 42)
        last = [c for c in self.browser.calls if c[0] == "Page.setInterceptFileChooserDialog"][-1]
        self.assertEqual(last[1], {"enabled": False})
        self.assertEqual(out["chosen"], ["emblem.png"])

    def test_a_chooser_from_a_frame_of_another_site_is_left_unanswered(self):
        card = self.card()
        real = self.browser.call

        def foreign(method, params=None, **kw):
            got = real(method, params, **kw)
            if method == "Input.dispatchMouseEvent" and (params or {}).get("type") == "mouseReleased":
                self.browser.t.chooser = {"mode": "selectSingle", "node": 77, "frame": "F9"}
            if method == "Page.getFrameTree":
                return {"frameTree": {"frame": {"id": "F0", "securityOrigin": "https://github.com"},
                                      "childFrames": [{"frame": {"id": "F9", "securityOrigin": "https://ads.example"}}]}}
            return got
        self.browser.call = foreign
        self.assertIn("frame from another site (https://ads.example)", self.refused(card))
        last = [c for c in self.browser.calls if c[0] == "Page.setInterceptFileChooserDialog"][-1]
        self.assertEqual(last[1], {"enabled": False})

    def test_a_press_that_navigates_elsewhere_is_left_unanswered(self):
        card = self.card()
        real = self.browser.call

        def away(method, params=None, **kw):
            got = real(method, params, **kw)
            if method == "Input.dispatchMouseEvent" and (params or {}).get("type") == "mouseReleased":
                self.browser.t.url = "https://evil.example/"
            return got
        self.browser.call = away
        self.assertIn("took the tab to evil.example", self.refused(card))

    def test_no_chooser_uploads_nothing_and_interception_is_turned_off(self):
        card = self.card()
        real = self.browser.call

        def no_chooser(method, params=None, **kw):
            got = real(method, params, **kw)
            self.browser.t.chooser = None
            return got
        self.browser.call = no_chooser
        drv.CHOOSER_WAIT, wait = 0.2, drv.CHOOSER_WAIT
        try:
            self.assertIn("no file chooser opened", self.refused(card))
        finally:
            drv.CHOOSER_WAIT = wait
        last = [c for c in self.browser.calls if c[0] == "Page.setInterceptFileChooserDialog"][-1]
        self.assertEqual(last[1], {"enabled": False})


class ClickRefusesAFileControl(Bench):
    def test_click_on_a_file_control_points_to_upload(self):
        self.browser.run = lambda tab, fn, *a, **k: (
            {"ref": "e5", "role": "button", "name": "Upload a photo…", "x": 1, "y": 1, "accepts": "image/*"}
            if fn == "target" else {})
        with self.assertRaises(drv.PageRefused) as caught:
            self.d.click("e5")
        self.assertIn("upload ref=e5", str(caught.exception))
        self.assertFalse([c for c in self.browser.calls if c[0] == "Input.dispatchMouseEvent"])


class TheServiceAsksOnlyAboutWhatItWouldDo(Bench):
    """Through the surface's real dispatch, in `ask` mode: a refused path never reaches the
    gate (no card), a good one does, and a grant is never spent on a call that would upload
    nothing."""

    def setUp(self):
        super().setUp()
        self.spent = []
        settings = os.path.join(self.home, "settings.yaml")
        mode = os.path.join(self.home, "mind-mode.json")
        with open(settings, "w") as f:
            f.write("tool_permission: sensitive\n")
        with open(mode, "w") as f:
            json.dump({"mode": "bypass"}, f)
        self.s, _ = service.build(self.d, settings_path=settings, mode_path=mode,
                                  spend_grant=lambda *a: self.spent.append(a))

    def act(self, action, grant=None, **args):
        params = {"action": action, "args": args}
        if grant:
            params["grant"] = grant
        return self.s.act(params)

    def refusal(self, fn):
        with self.assertRaises(wire.RpcError) as caught:
            fn()
        return caught.exception.message

    def test_upload_is_sensitive_and_says_it_cannot_be_undone(self):
        (spec,) = [a for a in self.s.describe_json()["actions"] if a["name"] == "upload"]
        self.assertEqual(spec["permission"], "sensitive")
        self.assertIn("cannot be undone", spec["description"])
        self.assertEqual(sorted(spec["parameters"]["required"]),
                         ["label", "path", "ref", "sha256", "site", "size"])
        (check,) = [a for a in self.s.describe_json()["actions"] if a["name"] == "upload_check"]
        self.assertEqual(check["permission"], "safe")

    def test_a_protected_path_is_refused_before_any_card(self):
        os.makedirs(os.path.join(self.home, ".ssh"))
        key = os.path.join(self.home, ".ssh", "id_ed25519")
        with open(key, "w") as f:
            f.write("key")
        message = self.refusal(lambda: self.act("upload", ref="e5", path=key, size=3,
                                                sha256="0" * 16, site="github.com", label="Upload a photo…"))
        self.assertIn("protected place", message)
        self.assertNotIn("GRANT", message)
        message = self.refusal(lambda: self.act("upload_check", ref="e5", path=key))
        self.assertIn("protected place", message)

    def test_a_good_file_is_asked_about_even_in_bypass(self):
        card = self.card()
        message = self.refusal(lambda: self.act("upload", ref="e5", **card))
        self.assertTrue(message.startswith("GRANT: browser.upload is graded `sensitive` and its own "
                                           "description says it cannot be undone"), message)
        self.assertEqual(self.browser.set_files, [])

    def test_with_the_persons_grant_the_file_goes(self):
        card = self.card()
        out = self.act("upload", grant="req-1", ref="e5", **card)
        self.assertEqual(len(self.spent), 1)
        self.assertEqual(len(self.browser.set_files), 1)
        self.assertEqual(out["result"]["chosen"], ["emblem.png"])

    def test_a_file_changed_after_the_card_spends_no_grant_and_uploads_nothing(self):
        card = self.card()
        with open(self.photo, "wb") as f:
            f.write(b"changed")
        message = self.refusal(lambda: self.act("upload", grant="req-1", ref="e5", **card))
        self.assertIn("not the file the person was shown", message)
        self.assertEqual(self.spent, [])
        self.assertEqual(self.browser.set_files, [])


if __name__ == "__main__":
    unittest.main()
