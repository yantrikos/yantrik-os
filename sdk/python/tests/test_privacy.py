"""Private mode, as a Python surface holds it: while the person has it on, the mind door refuses
every request, describe included, and no call carrying an agent token acts. The person's own calls
are untouched. The rule is the transport's (`privacy.rs`), quoted from the Rust source.
"""

import json
import os
import unittest
from unittest import mock

import support
import test_mind_door
from test_mind_door import notes_surface, shell_says
from yantrik_surface import Surface, mind_door, privacy, wire

PRIVACY_RS = "crates/yantrik-ipc-transport/src/privacy.rs"
SURFACE_PRIVATE_RS = "crates/yantrik-surface/src/private.rs"
OWNER_RS = "crates/yantrik-ipc-transport/src/private_owner.rs"


class TestTheFile(support.MachineCase):
    def write(self, text):
        with open(privacy.privacy_path(), "w", encoding="utf-8") as f:
            f.write(text)

    def test_absent_is_off_and_only_a_clear_false_is_off(self):
        self.assertFalse(privacy.is_private(), "a desktop nobody made private")
        self.write(json.dumps({"private": False, "since": 1}))
        self.assertFalse(privacy.is_private())
        for unclear in ("", "{", "[]", "{}", '{"private": "no"}', '{"private": 0}',
                        json.dumps({"private": True})):
            self.write(unclear)
            self.assertTrue(privacy.is_private(), unclear)

    def test_the_private_view_is_the_crates(self):
        support.quoted(self, SURFACE_PRIVATE_RS,
                       'pub const PRIVATE_SUMMARY: &str = "%s";' % privacy.PRIVATE_SUMMARY)
        support.quoted(self, SURFACE_PRIVATE_RS, "pub const RESERVED_KEYS: &[&str] = &[%s];"
                       % ", ".join('"%s"' % k for k in privacy.RESERVED_KEYS))

    def test_the_file_and_the_refusal_are_the_transports(self):
        support.quoted(self, PRIVACY_RS, 'pub const PRIVACY_FILE: &str = "privacy.json";')
        self.assertEqual(os.path.basename(privacy.privacy_path()), privacy.PRIVACY_FILE)
        rust = support.rust(PRIVACY_RS)
        if rust is None:
            self.skipTest("the Rust source is not beside this SDK")
        said = rust.split('pub const REFUSAL: &str = "', 1)[1].split('";', 1)[0]
        self.assertEqual(said, privacy.REFUSAL)


class TestPrivateSurface(test_mind_door.TestTheDoor):
    """A real door, real bytes, the person private or not."""

    def private(self, on=True):
        with open(privacy.privacy_path(), "w", encoding="utf-8") as f:
            json.dump({"private": on, "since": 1}, f)

    def test_the_door_refuses_everything_while_private_and_serves_again_after(self):
        with mock.patch.object(mind_door, "is_mind", lambda uid: uid == os.getuid()):
            _, server = self.serve(shell_says(None, known=True))
            self.private()
            for method, params in (("app.describe", {}),
                                   ("app.act", {"action": "new_note", "args": {"title": "t"},
                                                "agent_token": "live"})):
                refused = self.call(server.door, method, params)
                self.assertEqual(refused["error"]["message"], privacy.REFUSAL, method)
            self.private(False)
            self.assertEqual(self.call(server.door, "app.describe")["result"]["app"], "notes")

    def test_a_mind_already_connected_is_refused_from_the_next_request(self):
        with mock.patch.object(mind_door, "is_mind", lambda uid: True):
            _, server = self.serve()
            held = self.connect(server.door)
            held.sendall(b'{"jsonrpc":"2.0","id":1,"method":"app.describe","params":{}}\n')
            self.assertIn(b'"notes"', held.recv(65536))
            self.private()
            held.sendall(b'{"jsonrpc":"2.0","id":2,"method":"app.describe","params":{}}\n')
            self.assertIn(b"PRIVATE:", held.recv(65536))

    def test_the_persons_socket_describes_the_private_view_and_acts_reply_with_it(self):
        sentinel = "Call Priya SENTINEL-924"
        s = notes_surface(shell_says(None, known=True), [])

        @s.view
        def state():
            return {"title": sentinel, "units": "lines"}

        s.show_in_private("units")
        server = s.serve_in_thread()
        self.addCleanup(s.stop)
        before = self.call(server.path, "app.describe")["result"]
        self.assertEqual(before["state"]["title"], sentinel)
        self.private()
        hidden = self.call(server.path, "app.describe")["result"]
        self.assertIs(hidden["private"], True)
        self.assertEqual(hidden["summary"], privacy.PRIVATE_SUMMARY)
        self.assertEqual(hidden["state"], {"private": True, "units": "lines"})
        self.assertNotIn("SENTINEL-924", json.dumps(hidden))
        # The Rust view, byte for byte: the same state hashes to the same revision.
        self.assertEqual(hidden["revision"], wire.revision(privacy.PRIVATE_SUMMARY, hidden["state"]))
        done = self.call(server.path, "app.act", {"action": "new_note", "args": {"title": "mine"}})
        self.assertIs(done["result"]["private"], True)
        self.assertNotIn("SENTINEL-924", json.dumps(done))
        stale = self.call(server.path, "app.act", {"action": "list_notes", "args": {},
                                                   "expect_revision": before["revision"]})
        self.assertTrue(stale["error"]["message"].startswith("STALE:"), stale)
        self.assertNotIn("SENTINEL-924", json.dumps(stale))
        self.private(False)
        self.assertEqual(self.call(server.path, "app.describe")["result"]["state"]["title"], sentinel)

    def test_private_mode_turned_on_during_a_call_holds_for_that_calls_answer(self):
        # #926 review, L1: the person turns it on while the call runs on the app's thread.
        sentinel = "Call Priya SENTINEL-924"
        flip = []
        s = Surface("notes")

        @s.view
        def state():
            if flip:
                flip.pop()
                self.private()
            return {"title": sentinel}

        @s.action("go_private", grade="safe")
        def go_private() -> dict:
            "Turn Private mode on mid-call."
            self.private()
            return {"ok": True}

        server = s.serve_in_thread()
        self.addCleanup(s.stop)
        acted = self.call(server.path, "app.act", {"action": "go_private", "args": {}})["result"]
        self.assertIs(acted["private"], True, acted)
        self.assertNotIn("SENTINEL-924", json.dumps(acted))
        self.private(False)
        flip.append(True)
        described = self.call(server.path, "app.describe")["result"]
        self.assertIs(described["private"], True, described)
        self.assertEqual(described["summary"], privacy.PRIVATE_SUMMARY)
        self.assertNotIn("SENTINEL-924", json.dumps(described))

    def test_an_unvouched_caller_gets_neither_the_result_nor_the_refusal_text(self):
        # #926 review, M1: this test process is not an installed desktop program.
        from yantrik_surface import Refusal
        sentinel = "Call Priya SENTINEL-924"
        s = Surface("notes")

        @s.action("read_note", grade="safe")
        def read_note() -> dict:
            "Answer with the open note."
            return {"text": sentinel}

        @s.action("open_note", grade="safe")
        def open_note(title: str) -> dict:
            "Open a note by title."
            raise Refusal("no note %r; the open one is %s" % (title, sentinel))

        server = s.serve_in_thread()
        self.addCleanup(s.stop)
        self.assertEqual(self.call(server.path, "app.act", {"action": "read_note", "args": {}})
                         ["result"]["result"], {"text": sentinel})
        self.private()
        read = self.call(server.path, "app.act", {"action": "read_note", "args": {}})["result"]
        self.assertIs(read["accepted"], True)
        self.assertIsNone(read["result"])
        self.assertEqual(read["withheld"], privacy.WITHHELD)
        self.assertNotIn("SENTINEL-924", json.dumps(read))
        refused = self.call(server.path, "app.act", {"action": "open_note", "args": {"title": "x"}})
        self.assertEqual(refused["error"]["message"], privacy.WITHHELD_REFUSAL)

    def test_an_unhandled_fault_reaches_an_unvouched_caller_as_the_neutral_sentence(self):
        # #926 review, M1: KeyError, ValueError, the deferred path and a failing view.
        from yantrik_surface import Later
        sentinel = "Salary review SENTINEL-924"
        broken_view = []
        s = Surface("sheet")

        @s.view
        def state():
            if broken_view:
                raise KeyError(sentinel)
            return {"rows": 3}

        # Kept in Private mode, so the private describe reads the view, and meets its fault.
        s.show_in_private("rows")

        @s.action("cell", grade="safe")
        def cell() -> dict:
            "Read a cell."
            raise KeyError(sentinel)

        @s.action("parse", grade="safe")
        def parse() -> dict:
            "Parse a cell."
            raise ValueError("cannot parse %s" % sentinel)

        @s.action("export", grade="safe")
        def export():
            "Export, off the app's thread."
            def work():
                raise KeyError(sentinel)
            return Later(work)

        server = s.serve_in_thread()
        self.addCleanup(s.stop)
        off = self.call(server.path, "app.act", {"action": "cell", "args": {}})
        self.assertIn("SENTINEL-924", json.dumps(off), "off, the fault is the app's own words")
        self.private()
        for action in ("cell", "parse", "export"):
            refused = self.call(server.path, "app.act", {"action": action, "args": {}})
            self.assertEqual(refused["error"]["message"], privacy.WITHHELD_REFUSAL, (action, refused))
            self.assertNotIn("SENTINEL-924", json.dumps(refused), action)
        broken_view.append(True)
        described = self.call(server.path, "app.describe")
        self.assertNotIn("SENTINEL-924", json.dumps(described), described)

    def test_the_readers_while_private_are_the_crates(self):
        d = privacy.INSTALL_DIR
        own = d + "/yantrik-notes"
        for yes in (d + "/yantrik-ui", own, own + " (deleted)"):
            self.assertTrue(privacy.is_private_reader_in(yes, own, d, "notes"), yes)
        for no in ("/tmp/yantrik-term", d + "/yantrik-email", "/usr/bin/python3", "yantrik-ui", ""):
            self.assertFalse(privacy.is_private_reader_in(no, own, d, "notes"), no)
        # yantrik-term toward the shell only; another namespace or another file is nobody.
        term = d + "/yantrik-term"
        self.assertTrue(privacy.is_vouched(term, True, True, None, d, "shell"))
        self.assertFalse(privacy.is_vouched(term, True, True, None, d, "notes"))
        self.assertFalse(privacy.is_vouched(term, False, True, None, d, "shell"))
        self.assertFalse(privacy.is_vouched(term, True, False, None, d, "shell"))
        self.assertFalse(privacy.reads_while_private(os.getpid(), "shell"))
        support.quoted(self, OWNER_RS, 'pub const PRIVATE_READERS: &[(&str, &str)] = &[')
        support.quoted(self, OWNER_RS, '    ("yantrik-term", "shell"),\n];')
        support.quoted(self, SURFACE_PRIVATE_RS, 'pub const WITHHELD: &str = "%s";' % privacy.WITHHELD)
        support.quoted(self, SURFACE_PRIVATE_RS, '"%s";' % privacy.WITHHELD_REFUSAL)

    def test_an_app_cannot_show_the_private_views_own_keys(self):
        s = Surface("notes")
        for key in privacy.RESERVED_KEYS + ("", "  "):
            with self.assertRaises(ValueError):
                s.show_in_private(key)

    def test_an_agent_token_on_the_persons_socket_is_refused_and_the_person_is_not(self):
        ran = []
        s = notes_surface(shell_says(None, known=True), ran)
        server = s.serve_in_thread()
        self.addCleanup(s.stop)
        self.private()
        refused = self.call(server.path, "app.act", {"action": "new_note", "args": {"title": "t"},
                                                     "agent_token": "live"})
        self.assertEqual(refused["error"]["message"], privacy.REFUSAL)
        done = self.call(server.path, "app.act", {"action": "new_note", "args": {"title": "mine"}})
        self.assertIn("result", done, done)
        self.assertEqual(ran, [("new_note", "mine")], "only the person's own call ran")


# TestTheDoor lends its door, socket and client; its own cases run in test_mind_door, not here.
for _name in dir(test_mind_door.TestTheDoor):
    if _name.startswith("test_") and _name not in TestPrivateSurface.__dict__:
        setattr(TestPrivateSurface, _name, None)

if __name__ == "__main__":
    unittest.main()
