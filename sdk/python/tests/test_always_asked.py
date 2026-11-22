"""The always-asked rule at the Python door: the table `always_asked.rs` holds, and a declared
`always_asks` purpose, are asked in every mode but full bypass (#164), which asks nothing (#1353, decision D), before any
mode or rule answers — the order `gate::decide` has."""

import re
import unittest

import support
from yantrik_surface import Authority, Mode, always_asked, gate

RUST = "crates/yantrik-ipc-transport/src/always_asked.rs"
SAID = "A sentence that says nothing about permanence."


def authority(mode, granted=False):
    return Authority("dangerous", Mode(mode, frozenset()), granted)


def rust_strings(source, header):
    """The strings of the Rust table whose header matches the regex `header` (its length ignored)."""
    found = re.search(header, source)
    assert found, "no Rust table matches %r" % header
    body = source[found.end():].split("];")[0]
    return re.findall(r'"([^"]+)"', body)


class TestAlwaysAsked(unittest.TestCase):
    def source(self):
        with open(support.REPO + "/" + RUST, encoding="utf-8") as f:
            return f.read()

    def test_the_tables_are_the_rust_ones_item_by_item(self):
        src = self.source()
        pairs = rust_strings(src, r"pub const ALWAYS_ASKED: \[\(&str, &str\); \d+\] = \[")
        pairs = pairs[:]  # the length digit sits before `]`; strings only
        self.assertEqual(list(always_asked.ALWAYS_ASKED), list(zip(pairs[0::2], pairs[1::2])))
        self.assertEqual(list(always_asked.BROWSER_APPS), rust_strings(src, r"const BROWSER_APPS: \[&str; \d+\] = \["))
        self.assertEqual(list(always_asked.BROWSER_SILENT), rust_strings(src, r"pub const BROWSER_SILENT: \[&str; \d+\] = \["))

    def test_an_unknown_browser_action_asks_outside_full_bypass_and_fails_closed(self):
        for name in ("browser", "chromium", "app-browser"):
            for mode in ("ask", "auto", "bypass"):
                refusal = gate.decide(authority(mode), name, "a_future_action", "standard", SAID)
                self.assertTrue(refusal and refusal.startswith("GRANT:"), (name, mode))
            self.assertIsNone(gate.decide(authority("bypass_all"), name, "a_future_action", "standard", SAID), name)

    def test_type_is_refused_with_the_always_asked_wording(self):
        for mode in ("auto", "bypass"):
            refusal = gate.decide(authority(mode), "browser", "type", "standard", SAID)
            self.assertIn("to allow on a card every time", refusal, mode)
            self.assertNotIn("cannot be undone", refusal, mode)

    def test_a_listed_action_that_says_it_cannot_be_undone_keeps_that_wording(self):
        said = "Type it. It cannot be undone."
        self.assertIn("cannot be undone", gate.decide(authority("bypass"), "browser", "type", "standard", said))
        self.assertIsNone(gate.decide(authority("bypass_all"), "browser", "type", "standard", said))

    def test_type_asks_in_every_mode_but_full_bypass(self):
        for mode in ("ask", "auto", "bypass"):
            refusal = gate.decide(authority(mode), "browser", "type", "standard", SAID)
            self.assertTrue(refusal and refusal.startswith("GRANT:"), mode)
        self.assertIsNone(gate.decide(authority("bypass_all"), "browser", "type", "standard", SAID))
        self.assertIsNone(gate.decide(authority("auto", granted=True), "browser", "type", "standard", SAID))

    def test_each_silent_action_runs_unasked(self):
        for action in always_asked.BROWSER_SILENT:
            self.assertIsNone(gate.decide(authority("bypass_all"), "browser", action, "standard", SAID), action)

    def test_send_mail_and_the_shell_questions_ask_outside_full_bypass_and_run_in_it(self):
        for app, action in always_asked.ALWAYS_ASKED:
            self.assertIsNone(gate.decide(authority("bypass_all"), app, action, "sensitive", SAID), action)
            for mode in ("ask", "auto", "bypass"):
                refusal = gate.decide(authority(mode), app, action, "sensitive", SAID)
                self.assertTrue(refusal and refusal.startswith("GRANT:"), (mode, action))
        self.assertIsNone(gate.decide(authority("bypass_all"), "shell", "set_theme", "sensitive", SAID))

    def test_an_always_asks_purpose_asks_on_a_python_surface(self):
        purpose = "Hand over a key. " + always_asked.ALWAYS_ASKS
        for mode in ("auto", "bypass"):
            refusal = gate.decide(authority(mode), "anything", "grant", "sensitive", purpose)
            self.assertTrue(refusal and refusal.startswith("GRANT:"), mode)
        self.assertIn("plan mode", gate.decide(authority("plan"), "anything", "grant", "sensitive", purpose))
        self.assertIsNone(gate.decide(authority("bypass_all"), "anything", "grant", "sensitive", purpose),
                          "full bypass asks nothing (decision D)")
        self.assertIsNone(gate.decide(authority("bypass_all", granted=True), "anything", "grant", "sensitive", purpose))
        self.assertIsNone(gate.decide(authority("bypass_all"), "anything", "look", "safe", purpose), "safe is exempt")


if __name__ == "__main__":
    unittest.main()
