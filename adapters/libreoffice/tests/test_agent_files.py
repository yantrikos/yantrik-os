"""An agent names only files in the person's home, outside its protected places, and acts on no
document the person opened from anywhere else (#443; #942's audit). `read_text` is graded `safe`,
so before this an agent's `open ~/.ssh/id_ed25519` and `read_text` was any file the person can
read, asked about nothing. The person is not asked.

The shell's `files_stat` is stood in for by `stat` below, which answers as `home_paths::stat`
does for the cases here; the rule itself is tested where it lives, in yantrik-ipc-contracts."""

import os
import unittest
from unittest import mock

import support
from yantrik_libreoffice import build

SENTINEL = "SENTINEL-libreoffice-agent-91d2"


def no_reach(token, what):
    """The shell's answer to `reach_of` for a live token with no role."""
    return {"jsonrpc": "2.0", "id": 1, "result": {"accepted": True, "settled": True,
                                                  "result": {"reach": None, "known": True}}}


class AgentCase(support.Case):
    """A home with protected places, a stand-in for the shell's rule, and calls made as an agent."""

    def setUp(self):
        super().setUp()
        home = self.machine.home
        saved = os.environ.get("HOME")
        os.environ["HOME"] = home
        self.addCleanup(lambda: os.environ.pop("HOME") if saved is None else
                        os.environ.__setitem__("HOME", saved))
        self.home = home
        os.makedirs(os.path.join(home, "Documents"))
        os.makedirs(os.path.join(home, ".local", "share", "yantrik", "snippets"))
        os.makedirs(os.path.join(home, ".ssh"))
        self.asked = []
        self.surface = build(self.office, settings_path=self.machine.settings,
                             mode_path=self.machine.mode_file, ask_shell=no_reach, stat=self.stat,
                             agents=self.agents)

    def stat(self, asked):
        """`home_paths::stat` for these cases: outside, protected, or what is there."""
        self.asked.append(asked)
        path = os.path.join(self.home, asked[2:]) if asked.startswith("~/") else asked
        if not path.startswith(self.home + os.sep):
            return {"exists": "unknown", "reason": "outside"}
        rest = os.path.relpath(path, self.home)
        if rest.startswith(".ssh") or rest.startswith(os.path.join(".local", "share", "yantrik")):
            return {"exists": "unknown", "reason": "protected"}
        if os.path.isfile(path):
            return {"exists": True, "kind": "file"}
        if os.path.isdir(path):
            return {"exists": True, "kind": "directory"}
        return {"exists": False, "reason": "not_found"}

    def agent(self, action, **args):
        return self.surface.act({"action": action, "args": args, "agent_token": "tok-lo"})

    def agent_refused(self, action, **args):
        with self.assertRaises(support.RpcError) as caught:
            self.agent(action, **args)
        return caught.exception.message

    def doc(self, *parts, text=SENTINEL):
        path = os.path.join(*parts)
        support.fake_uno.write_doc(path, "writer", text=text)
        return path


class TestAgentFiles(AgentCase):
    def test_an_agent_opens_only_files_in_the_home(self):
        for path in (self.doc(self.home, ".ssh", "id_ed25519"),
                     self.doc(self.home, ".local", "share", "yantrik", "snippets", "snippets.odt"),
                     self.doc(self.files, "outside.odt")):
            said = self.agent_refused("open", path=path)
            self.assertTrue(said.endswith(" is protected") or said.endswith(" is outside"), said)
        self.assertEqual(self.soffice.documents + self.agent_soffice.documents, [],
                         "nothing was opened")
        mine = self.doc(self.home, "Documents", "plan.odt", text="the agent's own")
        self.assertEqual(self.agent("open", path=mine)["result"]["path"], mine)
        self.assertEqual(self.agent("read_text")["result"]["text"], "the agent's own")
        # The person opens anything they can read, as before.
        self.assertEqual(self.result("open", path=self.doc(self.files, "theirs.odt"))["document"],
                         "theirs.odt")

    def test_no_agent_call_reaches_a_document_the_person_has_open(self):
        """An agent's calls are served by the agents' own LibreOffice: the person's documents,
        from anywhere, are not in it to be read, changed, saved or exported."""
        for theirs in (self.doc(self.files, "theirs.odt"),
                       self.doc(self.home, "Documents", "theirs-at-home.odt")):
            self.act("open", path=theirs)
        for name in ("theirs.odt", "theirs-at-home.odt"):
            for action, args in (("read_text", {}), ("write_text", {"text": "x"}), ("save", {}),
                                 ("save_as", {"path": os.path.join(self.home, "Documents", "c.odt")}),
                                 ("export_pdf", {"path": os.path.join(self.home, "Documents", "c.pdf")}),
                                 ("close", {})):
                said = self.agent_refused(action, document=name, **args)
                self.assertIn("no document is open", said, action)
                self.assertNotIn(SENTINEL, said)
        self.assertEqual(self.soffice.stores, [], "nothing of the person's was written")
        self.assertFalse(os.path.exists(os.path.join(self.home, "Documents", "copy.odt")))
        self.assertIn(SENTINEL, self.result("read_text", document="theirs.odt")["text"],
                      "the person reads their own document")

    def test_an_agent_writes_only_where_it_may_write(self):
        self.agent("open", path=self.doc(self.home, "Documents", "plan.odt", text="plan"))
        for path, why in ((os.path.join(self.home, ".config", "autostart", "x.odt"), "hidden_place"),
                          (os.path.join(self.home, ".local", "share", "yantrik", "x.odt"), "protected"),
                          (os.path.join(self.files, "out.odt"), "outside")):
            self.assertTrue(self.agent_refused("save_as", path=path).endswith(why), path)
        self.assertTrue(self.agent_refused(
            "export_pdf", path=os.path.join(self.files, "out.pdf")).endswith(" is outside"))
        made = os.path.join(self.home, "Documents", "plan-copy.odt")
        self.assertEqual(self.agent("save_as", path=made)["result"]["path"], made)
        self.assertTrue(os.path.exists(made))

    def sheet(self, by=None):
        """A sheet in the home, opened by an agent (in the agents' LibreOffice) or by `by`."""
        path = os.path.join(self.home, "Documents", "book.ods")
        support.fake_uno.write_doc(path, "calc", sheets=[{"name": "Sheet1"}])
        (by or self.agent)("open", path=path)
        return path

    def test_an_agent_writes_no_formula_that_reads_a_file(self):
        """A formula reads any file the person can read, and `read_cells` hands it back."""
        store = os.path.join(self.home, ".local", "share", "yantrik", "snippets", "snippets.json")
        with open(store, "w", encoding="utf-8") as f:
            f.write(SENTINEL)
        self.sheet()
        for formula in ("='file://%s'#$Sheet1.A1" % store, "=INDIRECT(\"'file://%s'#$S.A1\")" % store,
                        "=DDE(\"soffice\";\"file://%s\";\"A1\")" % store,
                        "=WEBSERVICE(\"file://%s\")" % store, "=hyperlink(\"file://x\")",
                        "=FILTERXML(A2;\"//a\")", "=SUM(1;INDIRECT(\"A2\"))",
                        "=SUM('x'#$S.A1)", "=[1]Sheet1!A1", "=com.sun.star.sheet.addin.X.F(1)"):
            self.agent_refused("write_cells", cells={"A1": formula})
        self.assertNotIn(SENTINEL, str(self.agent("read_cells", range="A1:A3")))
        self.assertEqual(self.agent("read_cells", range="A1")["result"]["rows"], [[None]])

    def test_an_agent_writes_formulas_of_its_own_cells(self):
        self.sheet()
        self.agent("write_cells", cells={"A1": 2, "A2": 3, "B1": "=SUM(A1:A2)*2",
                                         "B2": '=IF(A1>1;"a(b";"c")', "B3": "=$Sheet1.A1+1"})
        self.assertIn("B1", self.agent("read_cells", range="A1:B3")["result"]["formulas"])

    def test_a_person_may_write_any_formula(self):
        self.sheet(by=self.act)
        self.act("write_cells", cells={"A1": "=WEBSERVICE(\"https://example.org\")"})

    def test_a_linked_document_is_not_read_or_written_by_an_agent(self):
        self.sheet()
        self.agent("write_cells", cells={"A1": "old"})
        self.agent_soffice.documents[-1].external_links.append("file:///x")
        self.assertIn("linked", self.agent_refused("read_cells", range="A1"))
        said = self.agent_refused("write_cells", cells={"A1": "new"})
        self.assertIn("linked", said)
        self.assertEqual(self.agent_soffice.documents[-1].sheets[0].cells["A1"], {"text": "old"},
                         "the refused write was put back")
        self.assertEqual(self.agent_soffice.documents[-1].sheets[0].cells["A1"], {"text": "old"})

    def seed(self, cells):
        """A sheet the person left with `cells` already in it, as a crafted file would."""
        self.sheet()
        sheet = self.agent_soffice.documents[-1].sheets[0]
        sheet.cells.update(cells)
        return sheet

    def test_a_named_expression_that_reads_a_file_is_not_read_through(self):
        """#942 round 3: `=sneak` passes the allowlist (a name), and `sneak` is an INDIRECT of a
        protected file. Calc registers the link only as it computes, after any check made before
        the read, so the read is refused before anything is computed."""
        sheet = self.seed({"B1": {"formula": "=sneak", "value": SENTINEL}})
        computed = []
        names = type("N", (), {
            "getElementNames": lambda me: ("sneak",),
            "getByName": lambda me, n: type("C", (), {
                "getContent": lambda m: "INDIRECT('file:///h/.local/share/yantrik/s.csv'#$s.A1)"})()})()
        sheet.doc.NamedRanges = names
        with mock.patch.object(support.fake_uno._Range, "getDataArray",
                               lambda me: computed.append(1) or (("%s" % SENTINEL,),)):
            said = self.agent_refused("read_cells", range="B1")
        self.assertIn("sneak", said)
        self.assertNotIn(SENTINEL, said)
        self.assertEqual(computed, [], "nothing was computed before the refusal")

    def test_a_link_that_appears_while_values_are_computed_withholds_the_rows(self):
        """The second wall: links are asked again after the values exist."""
        sheet = self.seed({"A1": {"formula": "=A2", "value": 1.0}})

        def compute(me):
            sheet.doc.external_links.append("file:///x")
            return ((SENTINEL,),)
        with mock.patch.object(support.fake_uno._Range, "getDataArray", compute):
            said = self.agent_refused("read_cells", range="A1")
        self.assertIn("linked", said)
        self.assertNotIn(SENTINEL, said)

    def test_a_formula_elsewhere_in_the_document_that_reads_a_file_refuses_the_read(self):
        """`=A5` is plain; A5, outside the range read, is the person's INDIRECT."""
        self.seed({"A1": {"formula": "=A5", "value": 0.0},
                   "A5": {"formula": "=INDIRECT(\"file:///s\")", "value": 0.0}})
        self.assertIn("outside", self.agent_refused("read_cells", range="A1"))


if __name__ == "__main__":
    unittest.main()
