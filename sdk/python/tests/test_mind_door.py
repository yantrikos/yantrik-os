"""The mind door, an agent's reach, and a mind's standing, as a Python surface holds them.

The Rust surfaces have had all three since #411, #189 and #182; a Python surface had none. A mind
could open Blender and never reach it (no door), a role's agent acting on Blender over the person's
socket was held to no reach, and a grant it spent was not bound to it. The rules are the
transport's (`reach.rs`, `mind_door.rs`, `yantrik-surface`'s `call.rs`), quoted from the Rust
source where their sentences are.
"""

import os
import socket
import stat
import tempfile
import threading
import unittest
from unittest import mock

import support
from yantrik_surface import Surface, caller, gate, mind_door, reach, wire

REACH_RS = "crates/yantrik-ipc-transport/src/reach.rs"
DOOR_RS = "crates/yantrik-ipc-transport/src/mind_door.rs"
CALL_RS = "crates/yantrik-surface/src/call.rs"


def reviewer():
    return {"agent": "deepseek:c-1a2b3c", "role": "reviewer", "name": "Reviewer",
            "surfaces": ["editor", "documents", "notes"], "ceiling": "safe"}


def coder():
    return {"agent": "pi:c-9f8e7d", "role": "coder", "name": "Coder",
            "surfaces": ["shell.agent_*", "editor"], "ceiling": "sensitive"}


def planner():
    return {"agent": "deepseek:c-7a1f02", "role": "planner", "name": "Planner",
            "surfaces": ["calendar", "notes"], "ceiling": "safe"}


def shell_says(reach_value=None, known=True):
    """A stand-in for the shell's `reach_of`, as the socket carries its answer."""
    def ask(token, what):
        return {"jsonrpc": "2.0", "id": 1, "result": {
            "app": "shell", "action_id": "app-shell#1", "accepted": True, "settled": True,
            "result": {"reach": reach_value, "known": known}}}
    return ask


def no_shell(token, what):
    raise reach.Unanswered("the shell did not say %s (no socket)" % what)


def notes_surface(ask_shell, ran, **kwargs):
    s = Surface("notes", ask_shell=ask_shell, **kwargs)

    @s.action("list_notes", grade="safe")
    def list_notes() -> dict:
        """List the notes."""
        ran.append("list_notes")
        return {"notes": []}

    @s.action("new_note", grade="standard")
    def new_note(title: str) -> dict:
        """Make a note."""
        ran.append(("new_note", title))
        peer = caller()
        return {"uid": peer.uid if peer else None}

    return s


class TestWithin(unittest.TestCase):
    """`reach::within`, case for case as its own tests have it."""

    def test_a_phones_hold_survives_the_shells_answer_and_asks_on_a_python_surface(self):
        # The regression the security review of 29 Sep found: `asks_above` was dropped here, and a
        # phone's turn acted unasked on every Python surface.
        reply = {"jsonrpc": "2.0", "id": 1, "result": {
            "app": "shell", "action_id": "app-shell#1", "accepted": True, "settled": True,
            "result": {"reach": {"agent": "pi:main", "role": "remote", "name": "turn asked from a phone",
                                 "surfaces": ["*"], "ceiling": "sensitive", "asks_above": "safe"},
                       "known": True}}}
        held = reach.reach_in_reply(reply)
        self.assertEqual(held.get("asks_above"), "safe")
        authority = gate.Authority("dangerous", gate.Mode("bypass", frozenset()))
        authority.held_by(held)
        self.assertIsNone(gate.decide(authority, "browser", "read", "safe", "Read the page"))
        self.assertTrue(gate.decide(authority, "browser", "go", "standard", "Go to a page").startswith("GRANT:"))

    def test_a_reach_of_every_app_still_holds_its_ceiling(self):
        phone = {"agent": "pi:main", "role": "remote", "name": "turn asked from a phone",
                 "surfaces": ["*"], "ceiling": "standard"}
        self.assertIsNone(reach.within(phone, "files", "move", "standard", {}))
        err = reach.within(phone, "calendar", "delete_event", "sensitive", {})
        self.assertIn("which may touch every app, at most `standard`", err)
        self.assertFalse(reach.covers(["*x"], "files", "move"))

    def test_an_act_on_its_surfaces_and_under_its_ceiling_runs(self):
        self.assertIsNone(reach.within(reviewer(), "notes", "list_notes", "safe", {}))
        self.assertIsNone(reach.within(coder(), "shell", "agent_run", "sensitive", {}))
        self.assertIsNone(reach.within(coder(), "shell", "agent_job", "standard", {}))
        self.assertIsNone(reach.within(coder(), "editor", "save", "standard", {}))

    def test_an_act_off_its_surfaces_is_refused_naming_the_role_and_its_reach(self):
        err = reach.within(reviewer(), "files", "move", "safe", {})
        self.assertTrue(err.startswith("REACH: files.move is outside the Reviewer's reach"), err)
        self.assertIn("`deepseek:c-1a2b3c` is the Reviewer, which may touch editor, documents and "
                      "notes, at most `safe`", err)
        self.assertIn("outside the Coder's reach",
                      reach.within(coder(), "shell", "new_agent", "sensitive", {}))
        self.assertIsNotNone(reach.within(coder(), "shell", "files_delete", "dangerous", {}))
        self.assertIsNotNone(reach.within(coder(), "editorial", "x", "safe", {}))
        support.quoted(self, REACH_RS, '"REACH: {what} is outside the {name}\'s reach, so it was '
                       'not run. {who}. {NEXT}"')

    def test_an_act_above_its_ceiling_is_refused_even_on_its_surfaces(self):
        err = reach.within(reviewer(), "notes", "new_note", "standard", {})
        self.assertTrue(err.startswith("REACH: notes.new_note is graded `standard`, above the "
                                       "Reviewer's `safe` ceiling"), err)
        self.assertIn("nobody was asked", err)
        self.assertIn("above the Coder's `sensitive` ceiling",
                      reach.within(coder(), "shell", "agent_run", "dangerous", {}))

    def test_a_role_opens_the_apps_its_reach_names_and_no_others(self):
        def open_(name):
            return reach.within(planner(), "shell", "open_app", "standard", {"name": name})
        self.assertIsNone(open_("notes"))
        self.assertIsNone(open_("calendar"))
        self.assertIsNone(open_("  Notes "))
        err = open_("terminal")
        self.assertTrue(err.startswith("REACH: shell.open_app opens `terminal`, an app the "
                                       "Planner's reach does not name"), err)
        self.assertIn("Opening an app its reach does name is within it, at any ceiling", err)
        self.assertIsNotNone(reach.within(planner(), "shell", "start_service", "standard",
                                          {"name": "notes"}))
        self.assertIsNotNone(reach.within(planner(), "shell", "open_app", "standard", {}))
        self.assertIsNotNone(reach.within(planner(), "shell", "open_app", "standard",
                                          {"name": ["notes"]}))

    def test_asking_the_person_and_reading_itself_are_within_every_reach(self):
        nothing = dict(reviewer(), surfaces=[])
        for always in reach.ALWAYS:
            app, action = always.split(".", 1)
            self.assertIsNone(reach.within(nothing, app, action, "safe", {}), always)
        self.assertIn("nothing on this desktop beyond asking the person",
                      reach.within(nothing, "shell", "show_agent", "safe", {}))
        support.quoted(self, REACH_RS, '"shell.record_unasked_action",')

    def test_names_fold_and_trim_as_rust_does_them(self):
        # Only A–Z fold: the Kelvin sign lowers to "k" in Python and must not name `kiosk`.
        self.assertFalse(reach.same_ascii_case("Kiosk", "kiosk"))
        self.assertTrue(reach.same_ascii_case("Notes", "notes"))
        self.assertIsNotNone(reach.within(dict(planner(), surfaces=["Kiosk"]), "kiosk",
                                          "read", "safe", {}))
        # `str::trim` takes Unicode whitespace and not the separators U+001C–U+001F.
        self.assertEqual(reach.trim(" 　notes\t"), "notes")
        self.assertEqual(reach.trim("\x1fnotes"), "\x1fnotes")
        self.assertIsNotNone(reach.within(planner(), "shell", "open_app", "standard",
                                          {"name": "\x1fterminal"}))

    def test_a_grade_or_a_ceiling_off_the_ladder_never_widens_a_reach(self):
        self.assertIsNotNone(reach.within(coder(), "shell", "agent_run", "catastrophic", {}))
        typo = dict(coder(), ceiling="sensitve")
        self.assertIsNotNone(reach.within(typo, "shell", "agent_job", "safe", {}))


class TestTheShellsAnswer(unittest.TestCase):
    """#189: the reach is the shell's word, and anything else is an error — never "no reach"."""

    def test_a_reach_a_role_and_liveness_read_from_the_shells_answer(self):
        self.assertEqual(reach.reach_of("t", shell_says(reviewer())), reviewer())
        self.assertIsNone(reach.reach_of("t", shell_says(None)))
        self.assertTrue(reach.standing_of("t", shell_says(None, known=True)))
        self.assertFalse(reach.standing_of("t", shell_says(None, known=False)))

    def test_an_answer_of_any_other_shape_is_an_error(self):
        for bad in ({}, {"result": {}}, {"result": {"result": {}}},
                    {"result": {"result": {"reach": {"agent": 3}}}},
                    {"result": {"reach": None}},  # the act's answer without its envelope
                    {"error": {"code": -32602, "message": "no"}}):
            with self.assertRaises(reach.Unanswered, msg=repr(bad)):
                reach.reach_in_reply(bad)
            with self.assertRaises(reach.Unanswered, msg=repr(bad)):
                reach.known_in_reply(bad)

    def test_the_shell_is_asked_by_the_tokens_digest_never_the_token(self):
        asked = []
        reach.reach_of(" tok-1 ", lambda token, what: asked.append(token) or
                       shell_says(None)(token, what))
        self.assertEqual(reach.token_digest(" tok-1 "), reach.token_digest("tok-1"))
        self.assertEqual(len(reach.token_digest("tok-1")), 64)

    def test_with_no_shell_to_ask_a_token_is_refused_not_let_through(self):
        with self.assertRaises(wire.RpcError) as caught:
            reach.read_reach("some-agent-token", no_shell)
        self.assertTrue(caught.exception.message.startswith("REACH:"))
        self.assertIn("Nothing was run", caught.exception.message)
        self.assertIsNone(reach.read_reach(None, no_shell), "no token: the person, as before")


class TestAnActIsHeldToItsAgentsReach(support.MachineCase):

    def test_a_role_acts_only_within_its_reach_and_the_handler_never_runs_outside_it(self):
        ran = []
        s = notes_surface(shell_says(reviewer()), ran)
        message = self.refusal(lambda: s.act({"action": "new_note", "args": {"title": "x"},
                                              "agent_token": "tok"}))
        self.assertTrue(message.startswith("REACH: notes.new_note is graded `standard`, above "
                                           "the Reviewer's `safe` ceiling"), message)
        self.assertEqual(ran, [])
        s.act({"action": "list_notes", "agent_token": "tok"})
        self.assertEqual(ran, ["list_notes"])

    def test_no_token_is_the_persons_call_and_meets_no_reach(self):
        ran = []
        s = notes_surface(no_shell, ran)
        s.act({"action": "new_note", "args": {"title": "mine"}})
        self.assertEqual(ran, [("new_note", "mine")])

    def test_a_token_the_shell_cannot_place_is_refused(self):
        ran = []
        s = notes_surface(no_shell, ran)
        message = self.refusal(lambda: s.act({"action": "list_notes", "agent_token": "tok"}))
        self.assertTrue(message.startswith("REACH: the shell did not say"), message)
        self.assertEqual(ran, [])

    def test_a_grant_is_not_spent_on_an_act_outside_the_reach(self):
        ran, spent = [], []
        s = notes_surface(shell_says(reviewer()), ran,
                          spend_grant=lambda *a: spent.append(a))
        message = self.refusal(lambda: s.act({"action": "new_note", "args": {"title": "x"},
                                              "agent_token": "tok", "grant": "appr-1"}))
        self.assertTrue(message.startswith("REACH:"), message)
        self.assertEqual(spent, [], "the person's Allow is not used up on an act that cannot run")

    def test_a_grant_is_spent_as_no_agent_until_the_shell_believes_a_python_forwarder(self):
        # #466: the shell believes a forwarded caller only from its own binaries, so an agent
        # forwarded from a Python app would be "not believed" and every grant would fail.
        ran, spent = [], []
        s = notes_surface(shell_says(None), ran, spend_grant=lambda *a: spent.append(a))
        peer = wire.PeerCred(4321, os.getuid(), os.getgid())
        s.act({"action": "new_note", "args": {"title": "x"}, "agent_token": " tok ",
               "grant": "appr-2"}, peer)
        self.assertEqual(spent, [("appr-2", "notes", "new_note", {"title": "x"})])
        self.assertEqual(ran, [("new_note", "x")])
        # What the spend will carry once it can (#182, `spend_params` in the Rust gate).
        params = gate.spend_params("appr-2", "notes", "new_note", {"title": "x"},
                                   gate.CallingAgent("tok", 4321))
        self.assertEqual(params["agent_token"], "tok")
        self.assertEqual(params["args"]["caller_pid"], 4321)
        self.assertNotIn("agent_token", params["args"])
        self.assertNotIn("agent_token", gate.spend_params("appr-3", "notes", "x", {}))


class TestAMindActsOnlyAsAnAttachedAgent(support.MachineCase):
    MIND = 4242

    def setUp(self):
        super().setUp()
        patcher = mock.patch.object(mind_door, "mind_uid", lambda: self.MIND)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.mind = wire.PeerCred(99, self.MIND, self.MIND)

    def test_a_mind_with_no_token_is_refused(self):
        ran = []
        s = notes_surface(shell_says(None), ran)
        message = self.refusal(lambda: s.act({"action": "list_notes"}, self.mind))
        self.assertTrue(message.startswith("MIND: a mind acts on the desktop only as an attached "
                                           "agent"), message)
        self.assertEqual(ran, [])
        support.quoted(self, CALL_RS, "this call carried none. Nothing was run.")

    def test_a_token_that_is_not_a_live_agents_is_refused(self):
        ran = []
        s = notes_surface(shell_says(None, known=False), ran)
        message = self.refusal(lambda: s.act({"action": "list_notes", "agent_token": "made-up"},
                                             self.mind))
        self.assertEqual(message, "MIND: this agent token is not one the shell has given a live "
                                  "agent. Nothing was run.")
        support.quoted(self, CALL_RS, "MIND: this agent token is not one the shell has given a "
                                      "live agent. Nothing was run.")

    def test_with_no_shell_to_ask_a_mind_is_refused(self):
        s = notes_surface(no_shell, [])
        message = self.refusal(lambda: s.act({"action": "list_notes", "agent_token": "t"},
                                             self.mind))
        self.assertTrue(message.startswith("MIND: the shell did not say"), message)

    def test_a_live_agent_acts_and_its_reach_still_holds_it(self):
        ran = []
        s = notes_surface(shell_says(None, known=True), ran)
        out = s.act({"action": "new_note", "args": {"title": "from a mind"}, "agent_token": "t"},
                    self.mind)
        self.assertEqual(out["result"]["uid"], self.MIND, "the handler is told the kernel's uid")
        held = notes_surface(shell_says(reviewer(), known=True), [])
        self.assertTrue(self.refusal(lambda: held.act(
            {"action": "new_note", "args": {"title": "x"}, "agent_token": "t"}, self.mind))
            .startswith("REACH:"))

    def test_the_person_and_reading_need_no_standing(self):
        ran = []
        s = notes_surface(no_shell, ran)
        person = wire.PeerCred(1, os.getuid(), os.getgid())
        s.act({"action": "list_notes"}, person)
        self.assertEqual(ran, ["list_notes"])
        self.assertEqual(s.describe_json(self.mind)["app"], "notes", "describe is free")

    def test_the_mind_account_is_never_root_nor_this_process(self):
        with mock.patch.object(mind_door, "mind_uid", lambda: 0):
            self.assertFalse(mind_door.is_mind(0))
        with mock.patch.object(mind_door, "mind_uid", lambda: os.getuid()):
            self.assertFalse(mind_door.is_mind(os.getuid()))
        self.assertTrue(mind_door.is_mind(self.MIND))
        self.assertFalse(mind_door.is_mind(None))

    def test_the_one_act_that_needs_no_standing_is_the_shells_own(self):
        self.assertFalse(reach.needs_standing("shell", "memory_validate"))
        self.assertTrue(reach.needs_standing("notes", "memory_validate"))
        support.quoted(self, CALL_RS, 'STANDING_NOT_NEEDED: &[(&str, &str)] = &[("shell", '
                                      '"memory_validate")]')


class TestTheDoor(support.MachineCase):
    """A real door directory, a real socket, real bytes. The mind account is this test's own uid
    only where a test says so — `is_mind` refuses the process's own uid by design."""

    def setUp(self):
        super().setUp()
        self.door = tempfile.mkdtemp(prefix="yantrik-minds-")
        self.addCleanup(lambda: os.path.isdir(self.door) and os.rmdir(self.door)
                        if not os.listdir(self.door) else None)
        os.chmod(self.door, 0o2750)
        self.minds = os.stat(self.door).st_gid
        for name, value in (("mind_uid", lambda: 4242), ("mind_gid", lambda: self.minds)):
            patcher = mock.patch.object(mind_door, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        saved = os.environ.get("YANTRIK_MIND_RUN")
        os.environ["YANTRIK_MIND_RUN"] = self.door
        self.addCleanup(lambda: os.environ.__setitem__("YANTRIK_MIND_RUN", saved)
                        if saved is not None else os.environ.pop("YANTRIK_MIND_RUN", None))

    def serve(self, ask_shell=None):
        s = notes_surface(ask_shell or shell_says(None), [])
        server = s.serve_in_thread()
        self.addCleanup(s.stop)
        return s, server

    def call(self, path, method, params=None):
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.settimeout(3)
        try:
            client.connect(path)
            client.sendall(('{"jsonrpc":"2.0","id":1,"method":"%s","params":%s}\n'
                            % (method, wire.json.dumps(params or {}))).encode())
            buf = b""
            while not buf.endswith(b"\n"):
                chunk = client.recv(65536)
                if not chunk:
                    break
                buf += chunk
            return wire.json.loads(buf) if buf else None
        except (ConnectionResetError, BrokenPipeError):
            # Closed with the request unread: the kernel resets the connection. Nothing answered.
            return None
        finally:
            client.close()

    def test_a_door_is_served_only_on_a_directory_exactly_as_the_tmpfiles_entry_makes_it(self):
        me, minds = 1000, 990
        self.assertTrue(mind_door.acceptable(me, minds, 0o42750, me, minds))
        self.assertFalse(mind_door.acceptable(0, minds, 0o42750, me, minds))
        self.assertFalse(mind_door.acceptable(me, 100, 0o42750, me, minds))
        self.assertFalse(mind_door.acceptable(me, minds, 0o42770, me, minds))
        self.assertFalse(mind_door.acceptable(me, minds, 0o40750, me, minds))
        self.assertFalse(mind_door.acceptable(me, minds, 0o42755, me, minds))
        support.quoted(self, DOOR_RS, "owner == me && group == minds && mode & 0o7777 == 0o2750")
        support.quoted(self, DOOR_RS, 'pub const DEFAULT_DIR: &str = "/run/yantrik-minds";')

    def test_only_a_surface_socket_of_this_session_gets_a_door(self):
        run = "/run/user/1000/yantrik"
        self.assertEqual(mind_door.door_for(run + "/app-blender.sock", run, "/run/yantrik-minds"),
                         "/run/yantrik-minds/app-blender.sock")
        self.assertIsNone(mind_door.door_for("/tmp/x/app-blender.sock", run, "/run/yantrik-minds"))
        for no in ("companion.sock", "a11y.sock", "perception.sock", "vault.sock", "harness"):
            self.assertFalse(mind_door.opens_a_door(no), no)

    def test_the_surface_listens_at_the_door_with_the_minds_mode(self):
        _, server = self.serve()
        door = os.path.join(self.door, "app-notes.sock")
        self.assertEqual(server.door, door)
        self.assertEqual(stat.S_IMODE(os.lstat(door).st_mode), 0o660)
        self.assertEqual(stat.S_IMODE(os.lstat(server.path).st_mode), 0o600, "the person's stays")

    def test_anyone_but_the_mind_account_is_closed_unread(self):
        _, server = self.serve()
        self.assertIsNone(self.call(server.door, "app.describe"),
                          "this test's own uid is the person, not a mind")
        self.assertEqual(self.call(server.path, "app.describe")["result"]["app"], "notes",
                         "and the person's own socket answers as before")

    def test_a_mind_at_the_door_is_held_as_a_mind(self):
        with mock.patch.object(mind_door, "is_mind", lambda uid: uid == os.getuid()):
            _, server = self.serve(shell_says(None, known=True))
            described = self.call(server.door, "app.describe")
            self.assertEqual(described["result"]["app"], "notes")
            refused = self.call(server.door, "app.act", {"action": "list_notes"})
            self.assertTrue(refused["error"]["message"].startswith("MIND:"), refused)
            done = self.call(server.door, "app.act", {"action": "new_note",
                                                      "args": {"title": "t"},
                                                      "agent_token": "live"})
            self.assertEqual(done["result"]["result"]["uid"], os.getuid())

    def ping(self, client):
        client.sendall(b'{"jsonrpc":"2.0","id":1,"method":"rpc.ping"}\n')
        return client.recv(4096)

    def connect(self, path, timeout=3):
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.settimeout(timeout)
        client.connect(path)
        self.addCleanup(client.close)
        return client

    def test_one_connection_past_the_bound_waits_for_a_slot(self):
        with mock.patch.object(mind_door, "is_mind", lambda uid: True), \
                mock.patch.object(mind_door, "DOOR_CONNECTIONS", 1):
            _, server = self.serve()
            held = self.connect(server.door)
            self.assertIn(b"pong", self.ping(held))
            waiting = self.connect(server.door, timeout=0.6)
            waiting.sendall(b'{"jsonrpc":"2.0","id":2,"method":"rpc.ping"}\n')
            with self.assertRaises(socket.timeout, msg="not served while the one slot is held"):
                waiting.recv(4096)
            held.close()
            waiting.settimeout(3)
            self.assertIn(b"pong", waiting.recv(4096), "and served once the slot is free")

    def test_a_line_past_the_limit_is_answered_and_not_read(self):
        with mock.patch.object(mind_door, "is_mind", lambda uid: True), \
                mock.patch.object(wire, "DOOR_MAX_LINE", 64):
            _, server = self.serve()
            client = self.connect(server.door)
            client.sendall(b'{"jsonrpc":"2.0","id":1,"method":"app.describe","params":{"x":"'
                           + b"a" * 200 + b'"}}\n')
            reply = wire.json.loads(client.recv(4096))
            self.assertEqual(reply["error"]["code"], wire.RPC_PARSE_ERROR)
            self.assertIn("request too large", reply["error"]["message"])
            self.assertIn("Nothing was run", reply["error"]["message"])
            self.assertEqual(client.recv(4096), b"", "and the connection is closed")
            # The person's own socket has no such limit: this is the door's rule.
            self.assertEqual(self.call(server.path, "app.describe", {"x": "a" * 200})["result"]
                             ["app"], "notes")

    def test_stopping_ends_the_mind_connections_still_open(self):
        with mock.patch.object(mind_door, "is_mind", lambda uid: True):
            s, server = self.serve()
            held = self.connect(server.door)
            self.assertIn(b"pong", self.ping(held))
            s.stop()
            self.assertEqual(held.recv(4096), b"", "the surface is gone, and so is the connection")

    def test_no_door_on_a_directory_that_is_not_exactly_as_made(self):
        os.chmod(self.door, 0o2770)
        _, server = self.serve()
        self.assertIsNone(server.door)
        self.assertEqual(os.listdir(self.door), [])

    def test_stopping_takes_the_door_away(self):
        s, server = self.serve()
        door = server.door
        s.stop()
        self.assertFalse(os.path.exists(door))


if __name__ == "__main__":
    unittest.main()
