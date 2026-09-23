"""An agent's reach, as this package holds a call to it (`yantrik_surface.reach`).

`deploy/yantrik-os/reach-vectors.json` is generated from `yantrik_ipc_transport::reach`
(`crates/yantrik-ipc-transport/src/reach/vectors.rs`):

* `within` — a reach and one call to it: the exact refusal, or null when the reach lets it
  through; `opens` stands in for the shell's own answer to which app a name opens.
* `standing` — what the shell answered about a token, or why it did not answer, and what a door
  decides from it.

Both are replayed here, and then the rule is met as a caller meets it: through `Surface.act`,
against a shell's answer handed in, against no shell at all, against a process that is not the
shell, and against a stand-in shell whose program is `yantrik-ui` — the door fails closed unless
the shell says what the token is. A call with no token asks nothing. `YANTRIK_REACH_VECTORS=<path>`
replays another copy.
"""

import json
import os
import unittest

import support
from yantrik_surface import Surface, agent_token, reach, wire

VECTORS = os.environ.get("YANTRIK_REACH_VECTORS") or os.path.join(
    support.REPO, "deploy", "yantrik-os", "reach-vectors.json")
RUST_REACH = "crates/yantrik-ipc-transport/src/reach.rs"


def load(case):
    if not os.path.isfile(VECTORS):
        if os.path.isfile(os.path.join(support.REPO, RUST_REACH)):
            case.fail("%s is missing; the crate that generates it is in this tree" % VECTORS)
        case.skipTest("no reach vectors outside the repository")
    with open(VECTORS, encoding="utf-8") as f:
        return json.load(f)


class TestTheVectors(unittest.TestCase):
    def test_every_call_is_held_as_the_rust_rule_holds_it(self):
        doc = load(self)
        table = doc["opens"]
        rows = doc["within"]
        self.assertGreaterEqual(len(rows), 150, "the table shrank")
        for row in rows:
            held = reach.reach_from_json(row["reach"])
            got = reach.within_call(held, row["app"], row["action"], row["grade"], row["args"],
                                    opens=table.get)
            self.assertEqual(got, row["refusal"], row)

    def test_every_answer_of_the_shell_is_decided_as_the_rust_rule_decides_it(self):
        rows = load(self)["standing"]
        self.assertGreaterEqual(len(rows), 9)
        for row in rows:
            if "unanswered" in row:
                held, refusal = reach.decided(None, row["unanswered"])
            else:
                try:
                    held, refusal = reach.decided(reach.standing_from_json(row["answer"]))
                except ValueError as e:
                    held, refusal = reach.decided(None, str(e))
            self.assertEqual(held._asdict() if held else None, row["held_to"], row)
            self.assertEqual(refusal, row["refusal"], row)


READER = reach.Reach("pi:c-read", "reader", "Reader", ["hello.look"], "safe")


def hello(**kwargs):
    s = Surface("hello", summary="hello", **kwargs)

    @s.action("look", grade="safe")
    def look() -> dict:
        """Say who is looking."""
        return {"agent_token": agent_token()}

    @s.action("add")
    def add(text: str) -> dict:
        """Add an item."""
        return {"added": text}

    return s


def answering(table):
    """A shell's answer, handed in: `table` from token to standing; `Unanswered` for `None`."""
    def standing(token):
        if table is None:
            raise reach.Unanswered("Connection failed (app-shell.sock): the shell is restarting")
        return table.get(token, (reach.UNKNOWN, None))
    return standing


class TestTheDispatch(support.MachineCase):
    """Through `Surface.act`, in the crate's order: the token's standing as the call is read,
    then — after the unknown action — the reach, before the arguments, the ceiling and a grant."""

    mode = "ask"

    def act(self, s, action, token=None, args=None, grant=None):
        params = {"action": action, "args": args or {}}
        if token is not None:
            params["agent_token"] = token
        if grant is not None:
            params["grant"] = grant
        return s.act(params)

    def test_a_role_is_held_to_its_reach_and_a_plain_agent_and_the_person_are_not(self):
        spent = []
        s = hello(reach_of=answering({"tok-reader": (reach.HELD, READER),
                                      "tok-plain": (reach.PLAIN, None)}),
                  spend_grant=lambda *call: spent.append(call))
        self.assertEqual(self.act(s, "look", "tok-reader")["result"]["agent_token"], "tok-reader")
        message = self.refusal(lambda: self.act(s, "add", "tok-reader", {"text": "x"}))
        self.assertTrue(message.startswith("REACH: hello.add is outside the Reader's reach, so it "
                                           "was not run. `pi:c-read` is the Reader, which may "
                                           "touch hello.look, at most `safe`, and may open hello "
                                           "(`shell.open_app name=<app>`)."), message)
        # Refused by the reach before its arguments, and before a grant is looked at.
        message = self.refusal(lambda: self.act(s, "add", "tok-reader", {"wrong": 1}, grant="g1"))
        self.assertTrue(message.startswith("REACH: hello.add is outside"), message)
        self.assertEqual(spent, [], "the reach's refusal spent nothing")
        # An unknown action is answered as unknown first.
        self.assertTrue(self.refusal(lambda: self.act(s, "nope", "tok-reader"))
                        .startswith("unknown action `nope`"))
        self.assertTrue(self.act(s, "add", "tok-plain", {"text": "x"})["accepted"])
        self.assertTrue(self.act(s, "add", None, {"text": "x"})["accepted"])

    def test_a_token_no_live_agent_carries_is_refused_whatever_it_asks(self):
        s = hello(reach_of=answering({}))
        for action in ("look", "add", "nope"):
            self.assertEqual(self.refusal(lambda a=action: self.act(s, a, "tok-stopped")),
                             reach.NO_LIVE_AGENT)
        support.quoted(self, RUST_REACH, reach.NO_LIVE_AGENT.replace("REACH: ", ""), skip=False)

    def test_a_shell_that_does_not_answer_refuses_every_token_and_not_the_persons_call(self):
        s = hello(reach_of=answering(None))
        message = self.refusal(lambda: self.act(s, "look", "tok-reader"))
        self.assertEqual(message, "REACH: the shell, which keeps every agent's reach, did not "
                                  "answer (Connection failed (app-shell.sock): the shell is "
                                  "restarting), so no act carrying an agent token runs until it "
                                  "does. Nothing was run.")
        self.assertTrue(self.act(s, "look")["accepted"], "no token asks nothing")


class TestTheShellIsAsked(support.MachineCase):
    """Over the socket, as a door meets the shell: by digest, and only a `yantrik-ui` process."""

    def test_no_shell_at_all_is_a_refusal_naming_why(self):
        s = hello()
        with self.assertRaises(wire.RpcError) as caught:
            s.act({"action": "look", "agent_token": "tok-reader"})
        message = caught.exception.message
        self.assertTrue(message.startswith("REACH: the shell, which keeps every agent's reach, did "
                                           "not answer (Connection failed ("), message)
        self.assertIn("app-shell.sock", message)
        self.assertTrue(s.act({"action": "look"})["accepted"], "the person's own call")

    def test_a_process_that_is_not_the_shell_is_not_asked(self):
        heard = []

        class Impostor(Surface):
            def handle_from(self, method, params, peer):
                heard.append(method)
                return {"standing": "plain"}

        impostor = Impostor("shell")
        impostor.serve_in_thread()
        self.addCleanup(impostor.stop)
        fragment = ('"the process answering as the shell is {exe} (pid {}), not the desktop\'s own '
                    '{SHELL_BINARY}, so it was not asked"')
        support.quoted(self, RUST_REACH, fragment, skip=False)
        message = self.refusal(lambda: hello().act({"action": "look", "agent_token": "tok-plain"}))
        self.assertEqual(message, (
            "REACH: the shell, which keeps every agent's reach, did not answer ("
            + support.render(fragment.strip('"'), os.getpid(), exe=os.readlink("/proc/self/exe"),
                             SHELL_BINARY="yantrik-ui")
            + "), so no act carrying an agent token runs until it does. Nothing was run."))
        self.assertEqual(heard, [], "nothing was written to a process that is not the shell")

    def test_the_desktops_shell_is_asked_by_digest_and_its_answer_holds(self):
        shell = support.ShellStandIn(self.machine).start(self)
        s = hello()
        self.assertEqual(s.act({"action": "look", "agent_token": "tok-reader"})["result"],
                         {"agent_token": "tok-reader"})
        message = self.refusal(lambda: s.act({"action": "add", "args": {"text": "x"},
                                              "agent_token": "tok-reader"}))
        self.assertTrue(message.startswith("REACH: hello.add is outside the Reader's reach"), message)
        self.assertTrue(s.act({"action": "add", "args": {"text": "x"},
                               "agent_token": "tok-plain"})["accepted"])
        self.assertEqual(self.refusal(lambda: s.act({"action": "look", "agent_token": "tok-gone"})),
                         reach.NO_LIVE_AGENT)
        # What went over the socket was the digest, never the token.
        answer = wire.call_once(shell.path, reach.ASK,
                                {"token_sha256": reach.token_digest("tok-reader")})
        self.assertEqual(reach.standing_from_json(answer["result"]), (reach.HELD, READER))
        # The shell goes: every token is refused again, and the person's call runs.
        shell.stop()
        message = self.refusal(lambda: s.act({"action": "look", "agent_token": "tok-plain"}))
        self.assertTrue(message.startswith("REACH: the shell, which keeps every agent's reach, did "
                                           "not answer ("), message)
        self.assertTrue(s.act({"action": "look"})["accepted"])


class TestOpening(unittest.TestCase):
    """Only the shell knows which app a name opens; this package serves no shell, so no opening
    act is let through by the app it names — the vectors replay the rule with their table."""

    def test_no_resolution_opens_nothing_on_a_reachs_say_so(self):
        planner = reach.Reach("deepseek:c-p1an", "planner", "Planner", ["calendar", "notes"], "safe")
        message = reach.within_call(planner, "shell", "open_app", "standard", {"name": "notes"})
        self.assertTrue(message.startswith("REACH: shell.open_app `notes` is outside the Planner's "
                                           "reach"), message)
        self.assertIsNone(reach.within_call(planner, "shell", "open_app", "standard",
                                            {"name": "notes"}, opens={"notes": "notes"}.get))


if __name__ == "__main__":
    unittest.main()
