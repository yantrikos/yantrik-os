"""LibreOffice is open to agents only through its sandbox (#942, #953): every action an agent (or
the Mind) can call is served by the agents' own sandboxed LibreOffice, never the person's, and
where that sandbox cannot be built every one is refused with one sentence and nothing is started.
The person's own use through the desktop is unaffected."""

import types
import unittest
from unittest import mock

import support
from test_agent_files import AgentCase, no_reach
from yantrik_libreoffice import agent_office, surface

ACTIONS = {
    "open": {"path": "{doc}"},
    "close": {},
    "read_text": {},
    "read_cells": {},
    "write_text": {"text": "x"},
    "write_cells": {"cells": {"A1": 1}},
    "save": {},
    "save_as": {"path": "{home}/Documents/copy.odt"},
    "export_pdf": {"path": "{home}/Documents/copy.pdf"},
}


class TestAgentsThroughTheSandbox(AgentCase):
    def setUp(self):
        super().setUp()
        self.doc_path = self.doc(self.home, "Documents", "plan.odt", text="plan")

    def args(self, action):
        return {k: (v.format(doc=self.doc_path, home=self.home) if isinstance(v, str) else v)
                for k, v in ACTIONS[action].items()}

    def test_the_table_covers_every_action_the_surface_offers(self):
        offered = {a["name"] for a in self.surface.describe_json()["actions"]}
        self.assertEqual(offered, set(ACTIONS))

    def test_without_a_sandbox_every_action_refuses_and_touches_no_libreoffice(self):
        self.agents.no_sandbox = "bubblewrap (bwrap) is not installed"
        for action in ACTIONS:
            said = self.agent_refused(action, **self.args(action))
            self.assertTrue(said.startswith(surface.CLOSED_TO_AGENTS), (action, said))
            self.assertIn("#953", said)
            self.assertIn("bwrap", said)
        self.assertEqual(self.agent_soffice.connections, 0, "the agents' UNO was never reached")
        self.assertEqual(self.agents.starts, 0, "no LibreOffice was started for an agent")
        self.assertEqual(self.soffice.connections, 0, "the person's UNO was never reached")

    def test_without_a_sandbox_the_minds_account_is_refused_with_no_token_at_all(self):
        """A peer on the Mind's uid is an agent whether or not it carries a token (#942 L3)."""
        self.agents.no_sandbox = "user namespaces are off"
        peer = types.SimpleNamespace(uid=4242, pid=77)
        with mock.patch("yantrik_surface.files.caller", return_value=peer), \
                mock.patch("yantrik_surface.mind_door.mind_uid", return_value=4242):
            for action in ACTIONS:
                with self.assertRaises(support.RpcError) as caught:
                    self.surface.act({"action": action, "args": self.args(action)})
                self.assertTrue(caught.exception.message.startswith(surface.CLOSED_TO_AGENTS),
                                action)
        self.assertEqual(self.agents.starts + self.agent_soffice.connections
                         + self.soffice.connections, 0)

    def test_with_the_sandbox_every_action_works_and_none_reaches_the_persons_libreoffice(self):
        for action in ("open", "read_text", "write_text", "save", "save_as", "export_pdf",
                       "close"):
            self.agent(action, **self.args(action))
        sheet = self.doc(self.home, "Documents", "s.ods")
        support.fake_uno.write_doc(sheet, "calc", sheets=[{"name": "S", "cells": {"A1": 2}}])
        self.agent("open", path=sheet)
        self.agent("write_cells", cells={"B1": "=A1*2"})
        self.assertEqual(self.agent("read_cells", range="A1")["result"]["rows"], [[2]])
        self.assertEqual((self.soffice.documents, self.soffice.stores), ([], []),
                         "the person's LibreOffice was only asked what it has open")
        self.assertEqual(self.agents.starts, 1)

    def test_the_mind_without_a_token_is_served_by_the_sandbox_too(self):
        peer = types.SimpleNamespace(uid=4242, pid=77)
        with mock.patch("yantrik_surface.files.caller", return_value=peer), \
                mock.patch("yantrik_surface.mind_door.mind_uid", return_value=4242):
            self.surface.act({"action": "open", "args": {"path": self.doc_path}})
        self.assertEqual(self.agents.starts, 1)
        self.assertEqual(self.soffice.documents, [])

    def test_the_real_agent_office_refuses_where_bwrap_is_missing(self):
        real = agent_office.AgentOffice(person=self.office, home=self.home,
                                        bwrap=("/nonexistent/bwrap",))
        surface_ = surface.build(self.office, settings_path=self.machine.settings,
                                 mode_path=self.machine.mode_file, stat=self.stat, agents=real,
                                 ask_shell=no_reach)
        with mock.patch("subprocess.Popen") as popen:
            for action in ACTIONS:
                with self.assertRaises(support.RpcError) as caught:
                    surface_.act({"action": action, "args": self.args(action),
                                  "agent_token": "tok-lo"})
                self.assertTrue(caught.exception.message.startswith(surface.CLOSED_TO_AGENTS),
                                (action, caught.exception.message))
            popen.assert_not_called()

    def test_the_person_still_works_in_their_own_libreoffice(self):
        opened = self.result("open", path=self.doc_path)
        self.assertEqual(opened["document"], "plan.odt")
        self.assertEqual(self.result("read_text")["text"], "plan")
        self.assertEqual(self.agent_soffice.connections, 0)
        self.assertEqual(self.agents.starts, 0)


if __name__ == "__main__":
    unittest.main()
