"""The gate as `yantrik_ipc_transport::gate` has it: reading the ceiling and the mode, the order
ceiling → grant → mode, a grant spent only past the ceiling (#154), and what rides beside `args`
(`grant`, `agent_token`) — including the token being stripped before a grant is spent."""

import contextlib
import io
import json
import unittest

import support
from yantrik_surface import (Authority, GrantRefused, Mode, Surface, agent_token, decide,
                             gate, grant_refusal, mode_from, wire)

G = support.RUST_GATE


class TestReadingTheFiles(unittest.TestCase):
    def test_the_ceiling_is_read_the_way_ceiling_from_reads_it(self):
        support.quoted(self, G, 'if key.trim() != "tool_permission" {')
        cases = {
            "tool_permission: dangerous": "dangerous",
            "theme: dark\ntool_permission: standard\n": "standard",
            'tool_permission: "dangerous"': "dangerous",
            "tool_permission: 'safe'": "safe",
            "  tool_permission  :  sensitive  ": "sensitive",
            "tool_permission: godmode": "sensitive",
            "tool_permission: dangerous # comment": "sensitive",
            "tool_permission: standard\ntool_permission: dangerous": "standard",
            "": "sensitive",
            "no colon here": "sensitive",
            "tool_permission: standard\r\n": "standard",
        }
        for text, expected in cases.items():
            self.assertEqual(gate.ceiling_from(text), expected, text)

    def test_a_missing_or_unreadable_settings_file_is_the_default(self):
        with support.Machine() as machine:
            self.assertEqual(gate.configured_ceiling(), "sensitive")
            with open(machine.settings, "wb") as f:
                f.write(b"tool_permission: \xff\xfe dangerous")
            self.assertEqual(gate.configured_ceiling(), "sensitive")
            machine.set_ceiling("dangerous")
            self.assertEqual(gate.configured_ceiling(), "dangerous")

    def test_the_mode_is_read_the_way_mode_from_reads_it(self):
        now = 1_800_000_000
        self.assertEqual(mode_from('{"mode":"auto"}', now), ("auto", set()))
        for text in ("", "{}", "mode: auto", '{"mode":"yolo"}', '{"mode":"BYPASS"}', "[]",
                     '{"mode": 3}'):
            self.assertEqual(mode_from(text, now).name, "ask", text)
        bypass = {"mode": "bypass", "previous": "auto", "bypass_expires_unix": now + 60}
        self.assertEqual(mode_from(json.dumps(bypass), now).name, "bypass")
        self.assertEqual(mode_from(json.dumps(bypass), now + 60).name, "auto")
        self.assertEqual(mode_from(json.dumps(dict(bypass, previous="bypass")), now + 60).name,
                         "ask")
        self.assertEqual(mode_from(json.dumps(dict(bypass, previous=None)), now + 60).name, "ask")
        # `as_u64`: a deadline that is not a whole non-negative number is no deadline at all.
        for until in (None, -5, 1.5e9, "1800000000", True):
            self.assertEqual(
                mode_from(json.dumps(dict(bypass, bypass_expires_unix=until)), now).name,
                "bypass", until)
        rules = mode_from(json.dumps({"mode": "ask", "session_rules": [
            {"app": "notes", "action": "delete"}, {"app": "x"}, "junk", {"app": 1, "action": "y"},
        ]}), now)
        self.assertEqual(rules, Mode("ask", frozenset({("notes", "delete")})))
        self.assertTrue(rules.covers("notes", "delete"))
        self.assertFalse(rules.covers("notes", "open"))

    def test_the_mode_file_sits_beside_the_settings_file(self):
        with support.Machine():
            self.assertEqual(gate.mode_path().rsplit("/", 1)[0],
                             gate.settings_path().rsplit("/", 1)[0])
            self.assertTrue(gate.mode_path().endswith("mind-mode.json"))

    def test_the_tables_are_the_rust_tables(self):
        support.quoted(self, G, 'pub const LADDER: [&str; 4] = ["safe", "standard", "sensitive", '
                                '"dangerous"];')
        support.quoted(self, G, '[("plan", "safe"), ("ask", "standard"), ("auto", "sensitive"), '
                                '("bypass", "dangerous")];')
        support.quoted(self, G, 'pub const SOCKET_FLOOR: &str = "standard";')
        support.quoted(self, G, 'pub const DEFAULT_MODE: &str = "ask";')
        support.quoted(self, G, 'pub const DEFAULT_CEILING: &str = "sensitive";')
        support.quoted(self, G, 'pub const MODE_FILE: &str = "mind-mode.json";')
        self.assertEqual(gate.LADDER, ("safe", "standard", "sensitive", "dangerous"))
        self.assertEqual(list(gate.MODES.items()), [("plan", "safe"), ("ask", "standard"),
                                                    ("auto", "sensitive"), ("bypass", "dangerous")])
        self.assertEqual((gate.SOCKET_FLOOR, gate.DEFAULT_MODE, gate.DEFAULT_CEILING,
                          gate.MODE_FILE), ("standard", "ask", "sensitive", "mind-mode.json"))


def at(ceiling, mode, granted=False):
    return Authority(ceiling, Mode(mode), granted)


class TestDecide(unittest.TestCase):
    """`gate.rs`'s own tests, ported one for one."""

    def test_the_order_is_ceiling_then_mode_and_each_says_which_it_was(self):
        err = decide(at("sensitive", "bypass"), "system-monitor", "kill_process", "dangerous")
        self.assertTrue(err.startswith("CEILING:") and "above this machine's `sensitive`" in err)
        err = decide(at("dangerous", "ask"), "system-monitor", "kill_process", "dangerous")
        self.assertTrue(err.startswith("GRANT:") and "ask mode" in err)
        self.assertIsNone(decide(at("dangerous", "bypass"), "system-monitor", "kill_process",
                                 "dangerous"))
        self.assertIsNone(decide(at("dangerous", "ask", granted=True), "system-monitor",
                                 "kill_process", "dangerous"))

    def test_standard_is_the_floor_in_every_mode_and_the_ceiling_still_binds_it(self):
        for mode in ("plan", "ask", "auto", "bypass"):
            self.assertIsNone(decide(at("sensitive", mode), "notifications", "notify",
                                     "standard"), mode)
        self.assertTrue(decide(at("safe", "bypass"), "notifications", "notify",
                               "standard").startswith("CEILING:"))

    def test_a_grade_off_the_ladder_is_refused_whatever_the_ceiling(self):
        err = decide(at("dangerous", "bypass"), "weather", "set_location", "catastrophic")
        self.assertTrue(err.startswith("CEILING:") and "not a level this OS defines" in err)

    def test_an_unknown_ceiling_is_the_default_not_an_opening(self):
        err = decide(at("everything", "bypass"), "a", "b", "dangerous")
        self.assertTrue(err.startswith("CEILING:"), err)

    def test_a_session_rule_covers_its_own_action_and_no_other(self):
        rules = Authority("dangerous", Mode("ask", frozenset({("notes", "delete")})))
        self.assertIsNone(decide(rules, "notes", "delete", "sensitive"))
        self.assertTrue(decide(rules, "notes", "purge", "sensitive").startswith("GRANT:"))

    def test_plan_says_no_card_is_coming(self):
        fragment = ('"GRANT: {app}.{action} is graded `{graded}` and this machine is in plan '
                    'mode, which raises no card for anything above `{SOCKET_FLOOR}` — so it was '
                    'not run. {PLAN}"')
        support.quoted(self, G, fragment)
        support.quoted(self, G, 'const PLAN: &str = "%s";' % support.GATE_PLAN)
        self.assertEqual(grant_refusal("notes", "delete", "sensitive", "plan"), support.render(
            fragment.strip('"'), app="notes", action="delete", graded="sensitive",
            SOCKET_FLOOR="standard", PLAN=support.GATE_PLAN))
        self.assertEqual(decide(at("dangerous", "plan"), "notes", "delete", "sensitive"),
                         grant_refusal("notes", "delete", "sensitive", Mode("plan")))

    def test_auto_names_what_it_runs_unasked(self):
        self.assertIn("in auto mode, which runs nothing above `sensitive`",
                      decide(at("dangerous", "auto"), "a", "b", "dangerous"))


DELETE = "Take an event off the calendar. It is not recoverable"
UPDATE = "Change an event's title, time or notes"


class TestWhatCannotBeUndone(unittest.TestCase):
    """`gate.rs`'s tests of the action's own description, ported: the dispatch reads the
    published purpose, so `yos act` or a raw socket is asked about `calendar.delete_event` in auto
    exactly as the shell's card and the bridge ask about it."""

    def test_what_its_own_description_says_cannot_be_undone_is_asked_about_on_every_door(self):
        self.assertIsNone(decide(at("sensitive", "auto"), "calendar", "update_event", "sensitive",
                                 UPDATE))
        fragment = ('"GRANT: {app}.{action} is graded `{graded}` and {final_word}, and this machine '
                    'is in {mode} mode, which asks before anything that cannot be undone — so it '
                    'was not run. {HOW}"')
        support.quoted(self, G, fragment)
        self.assertEqual(
            decide(at("sensitive", "auto"), "calendar", "delete_event", "sensitive", DELETE),
            "GRANT: calendar.delete_event is graded `sensitive` and its own description says it "
            "cannot be undone, and this machine is in auto mode, which asks before anything that "
            "cannot be undone — so it was not run. Ask the shell for approval first "
            "(`request_approval` with this app, action and these exact arguments, poll "
            "`approval_status`, then send the granted request_id as `grant` on app.act — `yos act` "
            "does all of that for you), or have the person at the machine press Allow when the "
            "card appears.")
        # Below the floor's grade too: a `standard` action that says so is asked about in ask.
        err = decide(at("sensitive", "ask"), "blender", "delete_object", "standard",
                     "Delete an object. Past that undo it is not recoverable.")
        self.assertTrue(err.startswith("GRANT:") and "cannot be undone" in err, err)
        # Bypass asks nobody, a grant answers it, and a `safe` read is never turned into a card.
        self.assertIsNone(decide(at("sensitive", "bypass"), "calendar", "delete_event",
                                 "sensitive", DELETE))
        self.assertIsNone(decide(at("sensitive", "auto", granted=True), "calendar",
                                 "delete_event", "sensitive", DELETE))
        self.assertIsNone(decide(at("sensitive", "plan"), "files", "describe_trash", "safe",
                                 "Lists what was deleted permanently"))

    def test_a_session_rule_never_covers_what_cannot_be_undone_nor_anything_in_plan(self):
        def with_rule(mode, action):
            return Authority("dangerous", Mode(mode, frozenset({("calendar", action)})))

        self.assertIsNone(decide(with_rule("ask", "update_event"), "calendar", "update_event",
                                 "sensitive", "Move it"))
        err = decide(with_rule("ask", "delete_event"), "calendar", "delete_event", "sensitive",
                     DELETE)
        self.assertIn("cannot be undone", err)
        err = decide(with_rule("plan", "update_event"), "calendar", "update_event", "sensitive",
                     "Move it")
        self.assertTrue(err.startswith("GRANT:") and "plan mode" in err, err)

    def test_plan_says_so_when_the_reason_is_the_description(self):
        fragment = ('"GRANT: {app}.{action} is graded `{graded}` and {final_word}, and this machine '
                    'is in plan mode, which raises no card for that — so it was not run. {PLAN}"')
        support.quoted(self, G, fragment)
        self.assertEqual(
            decide(at("dangerous", "plan"), "calendar", "delete_event", "sensitive", DELETE),
            "GRANT: calendar.delete_event is graded `sensitive` and its own description says it "
            "cannot be undone, and this machine is in plan mode, which raises no card for that — "
            "so it was not run. Say what you would do and let the person decide; they switch the "
            "mode from the chip in the status bar.")

    def test_the_phrases_are_read_the_way_the_gate_reads_them(self):
        support.quoted(self, G, 'pub const UNRECOVERABLE_PHRASES: [&str; 7] = [')
        self.assertTrue(gate.unrecoverable(DELETE))
        self.assertTrue(gate.unrecoverable("THIS CANNOT BE UNDONE"))
        self.assertTrue(gate.unrecoverable("there is no undo to argue with"))
        self.assertFalse(gate.unrecoverable("Move a file or folder to recoverable Trash"))
        self.assertFalse(gate.unrecoverable(""))
        self.assertEqual(len(gate.UNRECOVERABLE_PHRASES), 7)

    def test_a_cap_is_compared_on_this_ladder(self):
        self.assertIs(gate.permits("standard", "safe"), True)
        self.assertIs(gate.permits("standard", "dangerous"), False)
        self.assertIs(gate.permits("nonsense", "sensitive"), True)
        self.assertIs(gate.permits("nonsense", "dangerous"), False)
        self.assertIsNone(gate.permits("dangerous", "spicy"))

    def test_the_dispatch_reads_the_published_description(self):
        with support.Machine("dangerous", "auto"):
            s = Surface("calendar")

            @s.action("delete_event", grade="sensitive", description=DELETE)
            def delete_event(id: str) -> dict:
                return {"deleted": id}

            @s.action("update_event", grade="sensitive", description=UPDATE)
            def update_event(id: str) -> dict:
                return {"updated": id}

            self.assertTrue(s.act({"action": "update_event", "args": {"id": "1"}})["accepted"])
            with self.assertRaises(wire.RpcError) as caught:
                s.act({"action": "delete_event", "args": {"id": "1"}})
            self.assertIn("its own description says it cannot be undone", caught.exception.message)


class Shell:
    """A stand-in for the shell's store: `ok-*` ids hold once, for exactly
    `system-monitor.kill_process {"pid": 42}` — the stand-in `gate.rs`'s tests use."""

    def __init__(self):
        self.spent = []
        self.calls = []

    def __call__(self, grant, app, action, args):
        self.calls.append((grant, app, action, args))
        if not grant.startswith("ok-"):
            raise GrantRefused("no approval request `%s`." % grant)
        if (app, action, args) != ("system-monitor", "kill_process", {"pid": 42}):
            raise GrantRefused("`%s` was approved for another call, and this call carries %s."
                               % (grant, json.dumps(args)))
        if grant in self.spent:
            raise GrantRefused("`%s` was already used." % grant)
        self.spent.append(grant)


def sysmon(shell):
    # The shell's answer about the one token these tests carry: a live agent with no role, so the
    # gate alone decides for it (`test_reach` holds the rest).
    s = Surface("system-monitor", spend_grant=shell, reach_of=lambda token: ("plain", None))
    seen = {}

    @s.action("kill_process", grade="dangerous")
    def kill_process(pid: int) -> dict:
        """End a process. It is not recoverable."""
        seen["token"] = agent_token()
        return {"killed": pid}

    return s, seen


class TestGrants(support.MachineCase):
    ceiling = "dangerous"
    mode = "ask"

    def test_a_grant_is_not_spent_on_an_act_the_ceiling_refuses(self):
        shell = Shell()
        self.machine.set_ceiling("sensitive")
        s, _ = sysmon(shell)
        call = {"action": "kill_process", "args": {"pid": 42}, "grant": "ok-154"}
        self.assertTrue(self.refusal(lambda: s.act(call)).startswith("CEILING:"))
        self.assertEqual(shell.calls, [], "the ceiling's refusal, before anything is spent")
        self.machine.set_ceiling("dangerous")
        self.assertTrue(s.act(call)["accepted"], "left unspent by the refusal, so it holds now")
        message = self.refusal(lambda: s.act(call))
        self.assertTrue(message.startswith("GRANT:") and "already used" in message, message)

    def test_a_grant_is_not_spent_on_a_call_its_own_arguments_refuse(self):
        shell = Shell()
        s, _ = sysmon(shell)
        for args, refusal in (
                ({"pid": "forty-two"},
                 "`kill_process` argument `pid` must be an integer, and a string arrived"),
                ({}, "`kill_process` needs argument `pid`"),
                ({"pid": 42, "signal": 9}, "`kill_process` has no argument `signal`; it takes: pid")):
            self.assertEqual(self.refusal(lambda a=args: s.act({"action": "kill_process",
                                                                "args": a, "grant": "ok-1"})),
                             refusal)
        self.assertEqual(shell.calls, [], "the arguments were answered before anything was spent")
        self.assertTrue(s.act({"action": "kill_process", "args": {"pid": 42},
                               "grant": "ok-1"})["accepted"], "the same grant, the call right")
        self.assertEqual(shell.spent, ["ok-1"])

    def test_a_grant_is_bound_to_the_arguments_as_sent(self):
        shell = Shell()
        s, _ = sysmon(shell)
        message = self.refusal(lambda: s.act({"action": "kill_process", "args": {"pid": "42"},
                                              "grant": "ok-2"}))
        self.assertIn("approved for another call", message, "\"42\" on the call is not 42 on the card")

    def test_a_grant_that_does_not_hold_ends_the_call_in_the_shells_words(self):
        fragment = ('"GRANT: `{id}` does not authorise {app_id}.{action} — {why} Nothing was run; '
                    'a grant covers one action, once, with the arguments the person was shown."')
        support.quoted(self, G, fragment)
        s, _ = sysmon(Shell())
        message = self.refusal(lambda: s.act({"action": "kill_process", "args": {"pid": 42},
                                              "grant": "made-up"}))
        self.assertEqual(message, support.render(
            fragment.strip('"'), id="made-up", app_id="system-monitor", action="kill_process",
            why="no approval request `made-up`."))

    def test_any_grant_attached_is_spent_even_where_the_mode_would_not_ask(self):
        self.machine.set_mode("bypass")
        shell = Shell()
        s, _ = sysmon(shell)
        message = self.refusal(lambda: s.act({"action": "kill_process", "args": {"pid": 7},
                                              "grant": "ok-1"}))
        self.assertIn("approved for another call", message)

    def test_a_grant_rides_as_text_and_an_empty_one_is_none(self):
        self.assertEqual(gate.grant_of({"grant": " appr-7 "}), "appr-7")
        self.assertIsNone(gate.grant_of({"grant": ""}))
        self.assertIsNone(gate.grant_of({"grant": 7}))
        self.assertIsNone(gate.grant_of({}))
        s, _ = sysmon(Shell())
        # An empty grant is no grant: the mode refuses as if none came.
        self.assertTrue(self.refusal(lambda: s.act({"action": "kill_process", "args": {"pid": 42},
                                                    "grant": "  "})).startswith("GRANT: system"))

    def test_an_unknown_action_is_answered_before_a_grant_is_looked_at(self):
        shell = Shell()
        s, _ = sysmon(shell)
        self.assertTrue(self.refusal(lambda: s.act({"action": "nope", "grant": "ok-2"}))
                        .startswith("unknown action `nope`"))
        self.assertEqual(shell.calls, [])

    def test_the_grant_is_spent_against_the_arguments_without_the_agent_token(self):
        shell = Shell()
        s, seen = sysmon(shell)
        with contextlib.redirect_stderr(io.StringIO()) as said:
            out = s.act({"action": "kill_process",
                         "args": {"pid": 42, "agent_token": "smuggled"},
                         "agent_token": " tok-1 ", "grant": "ok-token"})
        self.assertIn("an agent token arrived inside `args`; it was removed and not used",
                      said.getvalue())
        self.assertNotIn("smuggled", said.getvalue(), "the token itself is never logged")
        self.assertTrue(out["accepted"])
        self.assertEqual(shell.calls, [("ok-token", "system-monitor", "kill_process",
                                        {"pid": 42})])
        self.assertEqual(seen["token"], "tok-1", "the token beside args reaches the handler")
        self.assertIsNone(agent_token(), "and is gone once the dispatch is over")

    def test_a_token_inside_args_alone_is_removed_and_not_used(self):
        self.machine.set_mode("bypass")
        s, seen = sysmon(Shell())
        with contextlib.redirect_stderr(io.StringIO()):
            s.act({"action": "kill_process", "args": {"pid": 1, "agent_token": "smuggled"}})
        self.assertIsNone(seen["token"])
        support.quoted(self, G, 'pub const AGENT_TOKEN: &str = "agent_token";')

    def test_a_parameter_cannot_be_called_agent_token(self):
        with self.assertRaises(ValueError):
            gate_surface = Surface("x")

            @gate_surface.action("a")
            def a(agent_token: str) -> dict:
                """Try."""
                return {}


if __name__ == "__main__":
    unittest.main()
