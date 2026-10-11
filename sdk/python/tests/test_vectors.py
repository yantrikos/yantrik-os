"""The policy vectors, replayed through this package's real dispatch.

`deploy/yantrik-os/surface-vectors.json` is generated from `gate::decide` (piece A of the surface
SDK design, design/surface-sdk-2026-09-23.md: "One decision, proven identical everywhere"). Its
`decide` section is every combination of grade, ceiling, mode, session rule, grant and the
action's own description, with the outcome (allow / CEILING / GRANT) and the exact refusal; its
other sections pin the tables, the "cannot be undone" phrases, revisions and envelopes. Every
implementation replays it, and a copy that drifts fails its own build.

A decision vector is replayed as a caller meets it: a surface publishing one action at that grade
with that description, the ceiling and the mode (and the session rule) written to the files the
dispatch reads, a stand-in shell holding the grant when one is attached, and `app.act` — so what
is checked is what the dispatch does, not a second reading of the table.

Every section is replayed, and a missing file is a failure inside this repository. The harness
also runs on vectors transcribed from `gate.rs`'s own tests, in the same shape, so it is checked
even where the package is tested on its own. `YANTRIK_SURFACE_VECTORS=<path>` replays another
copy of the file (a sibling branch's, say).
"""

import json
import os
import unittest

import support
from yantrik_surface import (LADDER, MODES, PROTOCOL, SOCKET_FLOOR, Action, GrantRefused,
                             Surface, gate, revision, wire)

VECTORS = os.environ.get("YANTRIK_SURFACE_VECTORS") or os.path.join(
    support.REPO, "deploy", "yantrik-os", "surface-vectors.json")
GRANT = "vector-grant"

# The spellings each input and output may come in. A vector without one of a required field's
# spellings is an error, not a guess.
FIELDS = {
    "grade": ("grade", "graded", "permission"),
    "ceiling": ("ceiling", "tool_permission"),
    "mode": ("mode",),
    "granted": ("grant", "granted"),
    "unrecoverable": ("unrecoverable",),
    "purpose": ("purpose", "description"),
    "expect": ("outcome", "expect", "verdict"),
    "sentence": ("refusal", "sentence", "message"),
    "app": ("app",),
    "action": ("action",),
}
REQUIRED = ("grade", "ceiling", "mode", "expect")
ALLOW = {"allow", "allowed", "run", "ok"}


def field(vector, name, default=None):
    for key in FIELDS[name]:
        if key in vector:
            return vector[key]
    if name in REQUIRED:
        raise KeyError("vector %s has no `%s` (looked for %s)"
                       % (vector.get("id", "?"), name, ", ".join(FIELDS[name])))
    return default


def outcome_of(vector):
    """allow, CEILING or GRANT."""
    expect = str(field(vector, "expect")).strip()
    if expect.lower() in ALLOW:
        return "allow"
    word = expect.rstrip(":").upper()
    if word in ("CEILING", "GRANT"):
        return word
    raise ValueError("vector %s expects %r, which is none of allow / CEILING / GRANT"
                     % (vector.get("id", "?"), expect))


def rules_of(vector, app, action):
    """`session_rule: true` is a rule for this very app.action; a list names its own."""
    if "session_rule" in vector:
        return [(app, action)] if vector["session_rule"] else []
    rules = []
    for rule in vector.get("session_rules") or vector.get("rules") or []:
        rules.append((rule.get("app"), rule.get("action")) if isinstance(rule, dict)
                     else tuple(rule))
    return rules


def replay(case, vector):
    """Run one vector through the dispatch; return what happened as (outcome, sentence)."""
    app = field(vector, "app") or "files"
    action = field(vector, "action") or "move"
    purpose = field(vector, "purpose")
    if purpose is None:
        purpose = "Move a file." + (" It is not recoverable." if field(vector, "unrecoverable")
                                    else "")

    machine = case.machine
    machine.set_ceiling(field(vector, "ceiling"))
    machine.set_mode({"mode": field(vector, "mode"), "session_rules": [
        {"app": a, "action": x} for a, x in rules_of(vector, app, action)]})

    spent = []

    def shell(grant, app_id, name, args):
        if grant != GRANT or (app_id, name, args) != (app, action, {}):
            raise GrantRefused("no approval request `%s`." % grant)
        spent.append(grant)

    s = Surface(app, spend_grant=shell)
    spec = s.add_action(Action(action, purpose), lambda args: {"done": True})
    spec.permission = field(vector, "grade")  # as published, even off the ladder
    params = {"action": action, "args": {}}
    if field(vector, "granted"):
        params["grant"] = GRANT
    try:
        out = s.act(params)
    except wire.RpcError as e:
        case.assertEqual(e.code, wire.RPC_INVALID_PARAMS, e.message)
        word = e.message.split(":", 1)[0]
        if word == "CEILING":
            case.assertEqual(spent, [], "a grant was spent on an act the ceiling refused (#154)")
        return word, e.message
    case.assertTrue(out["accepted"])
    return "allow", None


def mismatch(case, vector):
    """None when the dispatch does what the vector says; otherwise what it did instead."""
    expected = outcome_of(vector)
    got, sentence = replay(case, vector)
    if got != expected:
        return "%s, not %s: %s" % (got, expected, sentence)
    want = field(vector, "sentence")
    if expected != "allow" and want and sentence != want:
        return "the sentence differs: %r, not %r" % (sentence, want)
    return None


# `gate.rs`'s own tests, as vectors in the generated file's shape: they keep the harness honest
# until the file lands, and say the same things the Rust tests say.
KILL = "End a running process by pid"
FROM_GATE_RS = [
    {"id": "ceiling-before-mode", "app": "system-monitor", "action": "kill_process",
     "purpose": KILL, "grade": "dangerous", "ceiling": "sensitive", "mode": "bypass",
     "outcome": "CEILING"},
    {"id": "mode-after-ceiling", "app": "system-monitor", "action": "kill_process",
     "purpose": KILL, "grade": "dangerous", "ceiling": "dangerous", "mode": "ask",
     "outcome": "GRANT"},
    {"id": "bypass-runs-it", "app": "system-monitor", "action": "kill_process", "purpose": KILL,
     "grade": "dangerous", "ceiling": "dangerous", "mode": "bypass", "outcome": "allow"},
    {"id": "full-bypass-runs-it", "app": "system-monitor", "action": "kill_process",
     "purpose": KILL, "grade": "dangerous", "ceiling": "dangerous", "mode": "bypass_all",
     "outcome": "allow"},
    {"id": "a-grant-runs-it", "app": "system-monitor", "action": "kill_process", "purpose": KILL,
     "grade": "dangerous", "ceiling": "dangerous", "mode": "ask", "grant": True,
     "outcome": "allow"},
    {"id": "grant-not-spent-past-ceiling", "app": "system-monitor", "action": "kill_process",
     "purpose": KILL, "grade": "dangerous", "ceiling": "sensitive", "mode": "ask", "grant": True,
     "outcome": "CEILING"},
    {"id": "off-ladder", "app": "weather", "action": "set_location", "purpose": "",
     "grade": "catastrophic", "ceiling": "dangerous", "mode": "bypass", "outcome": "CEILING",
     "refusal": "CEILING: weather.set_location is graded `catastrophic`, which is not a level "
                "this OS defines (safe < standard < sensitive < dangerous), so it was not run."},
] + [
    {"id": "standard-floor-%s" % mode, "app": "notifications", "action": "notify",
     "purpose": "Post a notification", "grade": "standard", "ceiling": "sensitive", "mode": mode,
     "outcome": "allow"}
    for mode in ("plan", "ask", "auto", "bypass", "bypass_all")
] + [
    {"id": "standard-under-safe-ceiling", "app": "notifications", "action": "notify",
     "purpose": "Post a notification", "grade": "standard", "ceiling": "safe", "mode": "bypass",
     "outcome": "CEILING"},
    {"id": "session-rule", "grade": "sensitive", "ceiling": "sensitive", "mode": "ask",
     "session_rule": True, "outcome": "allow"},
    {"id": "session-rule-for-another-action", "grade": "sensitive", "ceiling": "sensitive",
     "mode": "ask", "session_rules": [["files", "rename"]], "outcome": "GRANT"},
]


class TestTheHarness(support.MachineCase):
    """The replay itself, on vectors from gate.rs's tests."""

    def test_the_rust_tests_these_come_from_are_still_there(self):
        for name in ("the_order_is_ceiling_then_mode_and_each_says_which_it_was",
                     "standard_is_the_floor_in_every_mode_and_the_ceiling_still_binds_it",
                     "a_grade_off_the_ladder_is_refused_whatever_the_ceiling",
                     "a_grant_is_not_spent_on_an_act_the_ceiling_refuses"):
            support.quoted(self, support.RUST_GATE, "fn %s()" % name)

    def test_every_transcribed_vector(self):
        for vector in FROM_GATE_RS:
            with self.subTest(vector["id"]):
                self.assertIsNone(mismatch(self, vector))

    def test_a_vector_that_does_not_say_its_outcome_is_an_error(self):
        with self.assertRaises(KeyError):
            outcome_of({"grade": "safe", "ceiling": "safe", "mode": "ask"})
        with self.assertRaises(ValueError):
            outcome_of({"outcome": "maybe"})


class TestTheGeneratedVectors(support.MachineCase):
    def setUp(self):
        super().setUp()
        if not os.path.isfile(VECTORS):
            if os.path.isdir(os.path.join(support.REPO, "deploy", "yantrik-os")):
                self.fail("%s is missing: this repository generates it from gate::decide "
                          "(YANTRIK_WRITE_VECTORS=1 cargo test -p yantrik-ipc-transport --lib "
                          "surface_vectors_write), and every port replays it" % VECTORS)
            self.skipTest("the package is being tested outside the repository, so there are no "
                          "generated vectors to replay (the harness runs on gate.rs's cases)")
        with open(VECTORS, encoding="utf-8") as f:
            self.doc = json.load(f)

    def section(self, name):
        self.assertIn(name, self.doc, "surface-vectors.json has no `%s` section" % name)
        return self.doc[name]

    def test_the_decisions(self):
        vectors = self.section("decide")
        self.assertTrue(vectors, "surface-vectors.json holds no decisions")
        drifted = []
        for vector in vectors:
            why = mismatch(self, vector)
            if why:
                drifted.append("%s: %s" % (vector.get("id", "?"), why))
        self.assertEqual(drifted, [], "%d of %d decisions differ from gate::decide:\n%s" % (
            len(drifted), len(vectors), "\n".join(drifted[:40])))

    def test_the_tables(self):
        self.assertEqual(tuple(self.section("ladder")), LADDER)
        self.assertEqual([tuple(m) for m in self.section("modes")], list(MODES.items()))
        self.assertEqual(self.section("socket_floor"), SOCKET_FLOOR)
        self.assertEqual(self.section("protocol"), PROTOCOL)

    def test_the_phrases_and_how_a_description_is_read(self):
        self.assertEqual(tuple(self.section("phrases")), gate.UNRECOVERABLE_PHRASES)
        for case in self.section("purposes"):
            self.assertEqual(gate.unrecoverable(case["purpose"]), case["unrecoverable"],
                             case["purpose"])

    def test_the_revisions(self):
        for case in self.section("revision"):
            with self.subTest(case["summary"]):
                self.assertEqual(revision(case["summary"], case["state"]), case["revision"])

    def test_the_float_edges(self):
        for case in self.section("revision_float_edges"):
            with self.subTest(case.get("rendered")):
                if "rendered" in case:
                    self.assertEqual(wire.canonical_state(case["state"]), case["rendered"])
                self.assertEqual(revision(case["summary"], case["state"]), case["revision"])

    def test_the_envelopes_have_this_dispatchs_keys(self):
        envelopes = self.section("envelopes")
        s = Surface("weather")
        s.add_action(Action("refresh", "Fetch the weather again", "safe"), lambda args: {})
        mine = s.describe_json()
        theirs = envelopes["describe"]
        self.assertEqual(set(theirs) - {"protocol"}, set(mine) - {"protocol"})
        self.assertEqual(set(theirs["actions"][0]), set(mine["actions"][0]))
        act = s.act({"action": "refresh"})
        self.assertEqual(set(envelopes["act"]), set(act))


if __name__ == "__main__":
    unittest.main()
