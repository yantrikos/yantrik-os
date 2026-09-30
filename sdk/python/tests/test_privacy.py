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
from yantrik_surface import mind_door, privacy

PRIVACY_RS = "crates/yantrik-ipc-transport/src/privacy.rs"


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
