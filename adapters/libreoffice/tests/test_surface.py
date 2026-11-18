"""LibreOffice's surface over a fake UNO: every action, every refusal, the grades, and what
describe shows — through the real `Office` and the SDK's real dispatch."""

import os
import sys
import unittest

import support
from yantrik_surface import gate


class TestWhatDescribeShows(support.Case):
    def test_a_libreoffice_that_is_not_running_is_said_to_be_so(self):
        self.soffice.stop()
        summary, state, revision = self.view()
        self.assertEqual(summary, "LibreOffice — not running")
        self.assertFalse(state["connected"])
        self.assertIn("not listening on the pipe `yantrik-libreoffice`", state["problem"])
        self.assertEqual(self.view()[2], revision, "an unchanged view keeps its revision")
        self.assertIn("LibreOffice is not running", self.refused("read_text"))

    def test_a_python_without_uno_is_said_to_be_so(self):
        saved = sys.modules.get("uno")
        sys.modules["uno"] = None  # `import uno` now raises ImportError, as on a machine without it
        self.addCleanup(lambda: sys.modules.pop("uno") if saved is None else
                        sys.modules.__setitem__("uno", saved))
        from yantrik_libreoffice import Office, build
        surface = build(Office(), settings_path=self.machine.settings,
                        mode_path=self.machine.mode_file)
        described = surface.describe_json()
        self.assertEqual(described["summary"], "LibreOffice — no python3-uno to reach it with")
        self.assertIn("python3-uno", described["state"]["problem"])

    def test_a_uno_runtime_that_will_not_start_is_said_to_be_so(self):
        class Broken(support.fake_uno.FakeUno):
            def getComponentContext(self):
                raise support.fake_uno.UnoException("no office installation found")
        from yantrik_libreoffice import Office, build
        surface = build(Office(uno_module=Broken(self.soffice)),
                        settings_path=self.machine.settings, mode_path=self.machine.mode_file)
        described = surface.describe_json()
        self.assertEqual(described["summary"], "LibreOffice — no UNO runtime to reach it with")
        self.assertIn("no office installation found", described["state"]["problem"])

    def test_running_with_nothing_open(self):
        summary, state, _ = self.view()
        self.assertEqual(summary, "LibreOffice — running, no document open")
        self.assertEqual(state, {"connected": True, "pipe": "yantrik-libreoffice", "front": None,
                                 "documents": [], "agent_documents": []},
                         "the Start Center is not a document")

    def test_every_open_document_with_what_a_mind_needs_to_name_it(self):
        report, budget = self.writer(), self.calc()
        self.act("open", path=report)
        self.act("open", path=budget, read_only=True)
        summary, state, _ = self.view()
        self.assertEqual(summary, "LibreOffice — report.odt (Writer); budget.ods (Calc, read-only); "
                                  "budget.ods in front")
        writer, calc = state["documents"]
        self.assertEqual({k: writer[k] for k in ("name", "kind", "path", "modified", "read_only",
                                                 "characters")},
                         {"name": "report.odt", "kind": "writer", "path": report,
                          "modified": False, "read_only": False, "characters": 34})
        self.assertEqual(calc["sheets"], [{"name": "Costs", "used": "A1:B4"},
                                          {"name": "Notes", "used": "A1"}])
        self.assertEqual(state["front"], "budget.ods")

    def test_many_documents_are_counted_not_listed(self):
        for i in range(4):
            self.act("open", path=self.writer("note-%d.odt" % i))
        self.assertEqual(self.view()[0], "LibreOffice — 4 documents open; note-3.odt in front")

    def test_the_revision_moves_when_the_content_does_not_only_when_the_flags_do(self):
        self.act("open", path=self.calc())
        self.act("write_cells", cells={"C1": "first edit"})
        _, _, before = self.view()
        # Still modified, same used area on the second sheet: only a cell differs.
        self.act("write_cells", cells={"C1": "second edit"})
        self.assertNotEqual(self.view()[2], before, "a changed cell must move the revision")


class TestTheGrades(support.Case):
    GRADES = {"open": "standard", "read_text": "safe", "read_cells": "safe",
              "write_text": "standard", "write_cells": "standard", "save": "sensitive",
              "save_as": "standard", "export_pdf": "standard", "close": "standard"}

    def actions(self):
        return {a["name"]: a for a in self.surface.describe_json()["actions"]}

    def test_each_action_carries_the_grade_the_readme_argues_for(self):
        self.assertEqual({n: a["permission"] for n, a in self.actions().items()}, self.GRADES)

    def test_no_description_says_it_cannot_be_undone(self):
        # A decision, not an accident: those words would make auto mode ask about `save` too.
        for name, action in self.actions().items():
            self.assertFalse(gate.unrecoverable(action["description"]), name)

    def test_every_argument_is_typed_and_described_and_slow_actions_say_so(self):
        for name, action in self.actions().items():
            for param, spec in action["parameters"]["properties"].items():
                self.assertTrue(spec["description"], "%s(%s) is not described" % (name, param))
        self.assertEqual({n: a.get("expected_seconds") for n, a in self.actions().items()
                          if a.get("expected_seconds")},
                         {"open": 15, "save": 10, "save_as": 10, "export_pdf": 20})
        self.assertEqual(self.actions()["write_cells"]["parameters"]["properties"]["cells"]["type"],
                         "object")
        self.assertEqual(self.actions()["write_text"]["parameters"]["properties"]["at"]["enum"],
                         ["end", "start", "replace_all"])

    def test_ask_mode_asks_before_a_save_and_nothing_else(self):
        self.machine.set_mode("ask")
        report = self.writer()
        self.act("open", path=report)
        self.act("write_text", text="\nThird.")
        self.assertTrue(self.refused("save").startswith(
            "GRANT: libreoffice.save is graded `sensitive` and this machine is in ask mode"))
        self.assertNotIn("Third.", self.on_disk(report)["text"], "nothing reached the disk")
        self.act("export_pdf", path=self.path("report.pdf"))
        self.machine.set_mode("auto")
        self.act("save")
        self.assertIn("Third.", self.on_disk(report)["text"])

    def test_a_standard_ceiling_refuses_a_save_whatever_the_mode(self):
        self.machine.set_ceiling("standard")
        self.act("open", path=self.writer())
        self.assertTrue(self.refused("save").startswith("CEILING: libreoffice.save is graded "
                                                        "`sensitive`, above this machine's "
                                                        "`standard` ceiling"))


class TestOpenAndName(support.Case):
    def test_open_answers_with_the_name_the_other_actions_take(self):
        report = self.writer()
        self.assertEqual(self.result("open", path=report),
                         {"document": "report.odt", "kind": "writer", "path": report,
                          "already_open": False})
        self.assertTrue(self.result("open", path=report)["already_open"])
        self.assertEqual(len(self.soffice.documents), 1, "a file open already is not opened twice")

    def test_paths_are_absolute_and_real(self):
        self.assertEqual(self.refused("open", path="report.odt"),
                         "`path` must be an absolute path (or start with ~), and `report.odt` is not")
        self.assertEqual(self.refused("open", path=self.path("nothing.odt")),
                         "no file at %s" % self.path("nothing.odt"))
        home_file = os.path.join(self.machine.home, "letter.odt")
        support.fake_uno.write_doc(home_file, "writer", text="Dear")
        saved = os.environ.get("HOME")
        os.environ["HOME"] = self.machine.home
        self.addCleanup(lambda: os.environ.pop("HOME") if saved is None else
                        os.environ.__setitem__("HOME", saved))
        self.assertEqual(self.result("open", path="~/letter.odt")["path"], home_file)

    def test_a_file_libreoffice_cannot_make_a_document_of_is_refused(self):
        odd = self.path("drawing.odg")
        support.fake_uno.write_doc(odd, "chart")
        self.assertIn("did not recognise the file", self.refused("open", path=odd))

    def test_which_document_when_none_is_named(self):
        self.assertEqual(self.refused("read_text"),
                         "no document is open in LibreOffice; `open` one first")
        self.act("open", path=self.writer("a.odt"))
        self.act("open", path=self.writer("b.odt", text="bee"))
        self.assertEqual(self.result("read_text")["document"], "b.odt", "the one in front")
        self.soffice.front = None
        self.assertEqual(self.refused("read_text"),
                         "2 documents are open (a.odt, b.odt); say which with `document`")
        self.assertEqual(self.result("read_text", document="A.ODT")["document"], "a.odt")
        self.assertEqual(self.result("read_text", document=self.path("b.odt"))["text"], "bee")
        self.assertEqual(self.refused("read_text", document="c.odt"),
                         "no open document is `c.odt`; open: a.odt, b.odt")

    def test_a_document_never_saved_is_named_by_its_title(self):
        self.soffice.new_document("writer")
        self.assertEqual(self.view()[1]["documents"][0]["name"], "Untitled 1")
        self.assertEqual(self.result("write_text", text="Draft", document="untitled 1")["document"],
                         "Untitled 1")


class TestWriter(support.Case):
    def setUp(self):
        super().setUp()
        self.report = self.writer(text="One.\nTwo.")
        self.act("open", path=self.report)

    def test_read_text_pages_through_a_long_document(self):
        self.assertEqual(self.result("read_text"),
                         {"document": "report.odt", "characters": 9, "start": 0, "text": "One.\nTwo.",
                          "more": False})
        part = self.result("read_text", start=5, max_chars=2)
        self.assertEqual((part["text"], part["more"]), ("Tw", True))
        self.assertEqual(self.refused("read_text", max_chars=0),
                         "`start` must be 0 or more and `max_chars` 1 or more")

    def test_write_text_at_the_end_the_start_or_in_place_of_it_all(self):
        self.act("write_text", text="\nThree.")
        self.act("write_text", text="Zero.\n", at="start")
        self.assertEqual(self.result("read_text")["text"], "Zero.\nOne.\nTwo.\nThree.")
        answer = self.result("write_text", text="Only this.", at="replace_all")
        self.assertEqual((answer["characters"], answer["modified"]), (10, True))
        self.assertEqual(self.on_disk(self.report)["text"], "One.\nTwo.", "not saved yet")
        self.assertEqual(self.refused("write_text", text="x", at="middle"),
                         "`write_text` argument `at` must be one of `end`, `start`, "
                         "`replace_all`, and another string arrived")

    def test_the_kinds_are_not_mixed_up(self):
        self.act("open", path=self.calc())
        self.assertEqual(self.refused("read_text", document="budget.ods"),
                         "`budget.ods` is a Calc spreadsheet, not a Writer document; "
                         "`read_cells` reads a spreadsheet")
        self.assertEqual(self.refused("read_cells", document="report.odt"),
                         "`report.odt` is a Writer document, not a Calc spreadsheet; "
                         "`read_text` reads a text document")

    def test_a_read_only_document_is_not_written(self):
        self.act("close")
        self.act("open", path=self.report, read_only=True)
        self.assertEqual(self.refused("write_text", text="x"),
                         "`report.odt` is open read-only; nothing can change it. Close it and "
                         "open it again without `read_only`")
        self.assertIn("read-only", self.refused("save"))


class TestCalc(support.Case):
    def setUp(self):
        super().setUp()
        self.budget = self.calc()
        self.act("open", path=self.budget)

    def test_read_cells_reads_the_used_area_values_and_formulas(self):
        self.assertEqual(self.result("read_cells"), {
            "document": "budget.ods", "sheet": "Costs", "range": "A1:B4",
            "rows": [["Item", "Amount"], ["Rent", 1200], ["Food", 350.5], [None, 0]],
            "formulas": {"B4": "=SUM(B2:B3)"}})

    def test_a_range_a_cell_and_another_sheet(self):
        self.assertEqual(self.result("read_cells", range="B2:B3")["rows"], [[1200], [350.5]])
        self.assertEqual(self.result("read_cells", range="$a$2")["rows"], [["Rent"]])
        self.assertEqual(self.result("read_cells", sheet="Notes")["rows"], [["checked in March"]])
        self.assertEqual(self.refused("read_cells", sheet="Income"),
                         "`budget.ods` has no sheet `Income`; its sheets: Costs, Notes")
        self.assertEqual(self.refused("read_cells", range="A1-B2"),
                         "`A1-B2` is not a cell range; write one like `A1:D20`, or one cell like `B3`")
        self.assertEqual(self.refused("read_cells", range="A1:Z1000"),
                         "A1:Z1000 is 26000 cells; read at most 10000 at a time")

    def test_write_cells_numbers_text_formulas_and_emptying(self):
        answer = self.result("write_cells", cells={"C1": "Note", "C2": 3, "c3": "=B3*2",
                                                   "A3": None, "D1": "'=not a formula"})
        self.assertEqual(answer["written"], ["A3", "C1", "C2", "C3", "D1"])
        read = self.result("read_cells", range="A3:D3")
        self.assertEqual(read["rows"], [[None, 350.5, 0, None]])
        self.assertEqual(read["formulas"], {"C3": "=B3*2"})
        self.assertEqual(self.result("read_cells", range="C1:D2")["rows"],
                         [["Note", "=not a formula"], [3, None]])

    def test_a_write_with_one_bad_cell_writes_none(self):
        self.assertEqual(self.refused("write_cells", cells={"A1": "fine", "B1:C1": 2}),
                         "`B1:C1` is not a cell; name one cell, like `B3`")
        self.assertEqual(self.refused("write_cells", cells={"A1": "fine", "B1": True}),
                         "B1: a cell takes a number, text, a formula starting with `=`, or null "
                         "to empty it")
        self.assertEqual(self.refused("write_cells", cells={"A1": "fine", "ZZZ1": 1}),
                         "ZZZ1 is outside the sheet `Costs`")
        self.assertEqual(self.refused("write_cells", cells={}),
                         "`cells` is empty; map each cell to what goes in it, like "
                         "{\"A1\": \"Total\", \"B1\": 42}")
        self.assertEqual(self.result("read_cells", range="A1")["rows"], [["Item"]])
        self.assertEqual(self.refused("write_cells", cells="A1=5"),
                         "`write_cells` argument `cells` must be an object, and a string arrived")

    def test_save_writes_the_cells_to_the_file(self):
        self.act("write_cells", cells={"B2": 1250})
        self.assertEqual(self.result("save"), {"document": "budget.ods", "path": self.budget,
                                               "modified": False})
        self.assertEqual(self.on_disk(self.budget)["sheets"][0]["cells"]["B2"], 1250.0)


class TestFilesOnDisk(support.Case):
    def setUp(self):
        super().setUp()
        self.report = self.writer()
        self.act("open", path=self.report)

    def test_save_as_makes_a_new_file_and_never_replaces_one(self):
        copy = self.path("report-copy.docx")
        self.assertEqual(self.result("save_as", path=copy),
                         {"document": "report-copy.docx", "path": copy, "was": self.report})
        self.assertEqual(self.soffice.stores[-1], ("storeAs:MS Word 2007 XML", copy))
        self.assertEqual(self.refused("save_as", path=self.report),
                         "a file is already at %s, and this action never replaces one; choose "
                         "another path" % self.report)
        self.assertEqual(self.refused("save_as", path=self.path("report.xlsx")),
                         "`report-copy.docx` is a Writer document, which cannot be saved as .xlsx; "
                         "it can be saved as .doc, .docx, .odt, .rtf, .txt")
        self.assertIn("must end in", self.refused("save_as", path=self.path("report.pages")))
        self.assertEqual(self.refused("save_as", path=self.path("no/such/dir/x.odt")),
                         "there is no directory %s to write into" % self.path("no/such/dir"))

    def test_a_document_never_saved_has_nothing_to_save_over(self):
        self.soffice.new_document("writer")
        self.assertEqual(self.refused("save", document="Untitled 1"),
                         "`Untitled 1` has never been saved, so it has no file to save over; "
                         "`save_as` it to a path")

    def test_export_pdf_writes_a_new_pdf_and_leaves_the_document_alone(self):
        self.act("write_text", text=" Edited.")
        pdf = self.path("report.pdf")
        reply = self.act("export_pdf", path=pdf)
        self.assertEqual((reply["accepted"], reply["settled"]), (True, True))
        self.assertEqual(reply["result"]["path"], pdf)
        self.assertGreater(reply["result"]["bytes"], 0)
        with open(pdf, "rb") as f:
            self.assertTrue(f.read().startswith(b"%PDF-"))
        document = self.view()[1]["documents"][0]
        self.assertEqual((document["path"], document["modified"]), (self.report, True))
        self.assertEqual(self.refused("export_pdf", path=pdf),
                         "a file is already at %s, and this action never replaces one; choose "
                         "another path" % pdf)
        self.assertEqual(self.refused("export_pdf", path=self.path("report.png")),
                         "%s must end in .pdf" % self.path("report.png"))

    def test_close_never_throws_edits_away(self):
        self.act("write_text", text="unsaved")
        self.assertEqual(self.refused("close"),
                         "`report.odt` has changes that are not saved; `save` or `save_as` it "
                         "first — this adapter never throws edits away")
        self.act("save")
        self.assertEqual(self.result("close"), {"closed": "report.odt"})
        self.assertEqual(self.view()[0], "LibreOffice — running, no document open")


class TestLibreOfficeComesAndGoes(support.Case):
    def test_a_libreoffice_that_quits_and_comes_back_is_found_again(self):
        self.act("open", path=self.writer())
        self.assertEqual(self.soffice.connections, 1)
        self.soffice.stop()
        self.assertEqual(self.view()[0], "LibreOffice — not running")
        self.assertIn("LibreOffice is not running", self.refused("read_text"))
        self.soffice.start()
        self.assertEqual(self.view()[0], "LibreOffice — running, no document open")
        self.act("open", path=self.writer())
        self.assertEqual(self.soffice.connections, 2, "one new connection, not one per call")

    def test_a_document_closed_under_a_call_is_said_plainly(self):
        self.act("open", path=self.writer())
        doc = self.soffice.documents[0]
        _, work = self.office.export_pdf_plan(self.path("x.pdf"))
        doc.close(True)
        with self.assertRaises(Exception) as caught:
            work()
        self.assertIn("the document was closed while this ran", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
