"""The agents' own LibreOffice, in its sandbox, against a real soffice (#953).

Skipped unless this machine has LibreOffice, a Python that can `import uno`, and a bwrap that can
make user namespaces. A spreadsheet whose formulas reach for a protected file at the home's top, a
protected name nested in an ordinary folder (#942 M3) and a web service on the host's loopback is
opened with no file rule and no scan at all, so the sandbox is the only thing deciding, and what
LibreOffice computes and saves must not hold any of the sentinels.

Each run first opens the same spreadsheet in an unsandboxed LibreOffice and requires every sentinel
there (#942 L1): a probe that cannot see a leak where there is one fails instead of passing. Taking
the sandbox away makes the start refuse, which the last test checks.

    python3 -m unittest discover -s adapters/libreoffice/tests -p test_live_sandbox.py -v
"""

import http.server
import os
import secrets
import shutil
import tempfile
import threading
import time
import unittest
from unittest import mock

import support
from test_agent_files import no_reach
from test_live import content_of, odf, pdf_text_operators, uno_importable
from test_sandbox import binds
from yantrik_libreoffice import agent_office, sandbox
from yantrik_libreoffice.instance import start_headless, stop_headless
from yantrik_libreoffice import build
from yantrik_libreoffice.office import Office

TOP = "SENTINEL-top-%s" % secrets.token_hex(4)
NESTED = "SENTINEL-nested-%s" % secrets.token_hex(4)
WEB = "SENTINEL-web-%s" % secrets.token_hex(4)


def sandbox_missing():
    try:
        sandbox.check(sandbox.find_bwrap())
    except sandbox.SandboxError as e:
        return str(e)
    return None


def put(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    return path


class _Web(http.server.BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802 - http.server's name
        body = WEB.encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def computed(office, formulas, rounds=20):
    """`formulas` written into A1:D1 of the document in front straight through UNO (no agent
    allowlist), computed again and again until every cell has a value or `rounds` run out (Calc
    loads an external file in the background, so the first computation is often empty), and
    everything A1:D1 held on the way, through `office`. Writing them after the open is what makes
    Calc evaluate them, as the review's probe did (#942 L1)."""
    def write(desktop):
        sheet = office._pick(desktop, None).getSheets().getByIndex(0)  # noqa: SLF001
        for column, formula in enumerate(formulas):
            sheet.getCellByPosition(column, 0).setFormula(formula)

    def read(desktop):
        doc = office._pick(desktop, None)  # noqa: SLF001 - the test does what no action may
        doc.calculateAll()
        sheet = doc.getSheets().getByIndex(0)
        return [sheet.getCellByPosition(column, 0).getString() for column in range(len(formulas))]
    office.run(write)
    seen = []
    for _ in range(rounds):
        values = office.run(read)
        seen.append(repr(values))
        if all(values):
            break
        time.sleep(0.5)
    return " ".join(seen)


@unittest.skipUnless(shutil.which("soffice"), "no LibreOffice (soffice) on this machine")
@unittest.skipUnless(uno_importable(), "this Python cannot import uno (python3-uno)")
@unittest.skipIf(sandbox_missing(), "no sandbox here: %s" % sandbox_missing())
class TestTheSandboxedLibreOffice(unittest.TestCase):
    def setUp(self):
        # Outside /tmp, which the sandbox replaces whole, so a sandbox that showed everything
        # else would show this home too.
        self.tmp = tempfile.mkdtemp(prefix="yantrik-live-sandbox-", dir="/var/tmp")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.home = os.path.join(self.tmp, "home")
        top = put(os.path.join(self.home, ".local", "share", "yantrik", "top.csv"), TOP + "\n")
        nested = put(os.path.join(self.home, "Documents", "old", ".ssh", "k.csv"), NESTED + "\n")
        web = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Web)
        threading.Thread(target=web.serve_forever, daemon=True).start()
        self.addCleanup(web.server_close)
        self.addCleanup(web.shutdown)
        self.documents = os.path.join(self.home, "Documents")
        self.crafted = os.path.join(self.documents, "crafted.ods")
        # Direct references first: LibreOffice 24.2 aborts on an INDIRECT of a .csv it has not
        # loaded yet, and loads it for the direct reference.
        self.formulas = ("='file://%s'#$top.A1" % top,
                         "='file://%s'#$k.A1" % nested,
                         "=INDIRECT(\"'file://%s'#$top.A1\")" % top,
                         "=WEBSERVICE(\"http://127.0.0.1:%d/\")" % web.server_address[1])
        # A plain sheet: the formulas go in after the open. Ones already in the file, loaded with
        # LibreOffice's own NO_UPDATE, come back Err:540 (external content disabled), which would
        # hide what the sandbox does.
        odf(self.crafted, "application/vnd.oasis.opendocument.spreadsheet",
            "<office:spreadsheet><table:table table:name=\"S\"><table:table-row>"
            "<table:table-cell/></table:table-row></table:table></office:spreadsheet>")
        self.env = dict(os.environ, HOME=self.home)

    def agents(self):
        """The agents' LibreOffice with LibreOffice's default link settings (the adapter's own
        profile says never), so the sandbox is the one thing deciding, as in the control."""
        office = agent_office.AgentOffice(home=self.home, env=self.env)
        self.addCleanup(office.stop)
        patch = mock.patch.object(agent_office, "write_profile_settings",
                                  lambda path: os.makedirs(path, exist_ok=True))
        patch.start()
        self.addCleanup(patch.stop)
        return office

    def test_the_unsandboxed_control_sees_every_sentinel(self):
        """Without this, the test below could pass by evaluating nothing."""
        profile = os.path.join(self.tmp, "control-profile")
        pipe = "yantrik-live-control-%s" % secrets.token_hex(4)
        with mock.patch("yantrik_libreoffice.instance.write_profile_settings",
                        lambda path: os.makedirs(path, exist_ok=True)):
            process = start_headless("soffice", pipe, profile)
        office = Office(pipe, hidden=True)
        self.addCleanup(stop_headless, process, office)
        for _ in range(300):
            try:
                office.desktop()
                break
            except support.OfficeError:
                threading.Event().wait(0.2)
        office.open(self.crafted)
        seen = computed(office, self.formulas)
        for sentinel in (TOP, NESTED, WEB):
            self.assertIn(sentinel, seen, "the control must leak, or the probe proves nothing")

    def test_nothing_libreoffice_computes_or_saves_in_the_sandbox_holds_a_sentinel(self):
        agents = self.agents()
        workspace = agents.workspace(start=True)   # Debian's soffice starts in it (#942 M2)
        workspace.open(self.crafted)
        seen = computed(agents._office, self.formulas)  # noqa: SLF001
        for sentinel in (TOP, NESTED, WEB):
            self.assertNotIn(sentinel, seen)
        rows = workspace.read_cells("A1:D1")
        csv = os.path.join(self.documents, "out.csv")
        workspace.save_as(csv)
        with open(csv, encoding="utf-8") as f:
            saved = f.read()
        for sentinel in (TOP, NESTED, WEB):
            self.assertNotIn(sentinel, str(rows) + saved)

    def test_an_agents_ordinary_work_runs_in_the_sandbox(self):
        agents = self.agents()
        workspace = agents.workspace(start=True)
        self.assertIn("Landlock", agents.landlock, "the launcher proved it and said what it put up")
        letter = os.path.join(self.documents, "letter.odt")
        odf(letter, "application/vnd.oasis.opendocument.text",
            "<office:text><text:p>Dear Sam,</text:p></office:text>")
        workspace.open(letter)
        workspace.write_text("See you Friday.")
        workspace.save()
        self.assertIn("See you Friday.", workspace.read_text()["text"])
        self.assertIn("See you Friday.", content_of(letter), "the save reached the person's file")
        pdf = os.path.join(self.documents, "letter.pdf")
        _, export = workspace.export_pdf_plan(pdf)
        self.assertEqual(export()["path"], pdf)
        with open(pdf, "rb") as f:
            self.assertGreater(pdf_text_operators(f.read()), 0)
        workspace.close()

    def test_with_the_sandbox_taken_away_the_start_is_refused(self):
        def no_namespaces(bwrap, private, root, launcher, hidden, inner):
            return [a.replace(sandbox.LAUNCHER, launcher) for a in inner]
        agents = self.agents()
        with mock.patch.object(sandbox, "command", no_namespaces):
            with self.assertRaises(support.OfficeError) as caught:
                agents.workspace(start=True)
        said = str(caught.exception)
        self.assertTrue(said.startswith(agent_office.CLOSED_TO_AGENTS), said)
        self.assertIn("did not hold", said)

    def test_every_agent_action_runs_end_to_end_through_the_surface(self):
        """The 9 actions, called as an agent on the real surface, served by the sandbox."""
        agents = self.agents()
        person = Office("yantrik-live-sandbox-nobody-%s" % secrets.token_hex(4))
        home = self.home

        def stat(asked):
            path = os.path.join(home, asked[2:]) if asked.startswith("~/") else asked
            if not path.startswith(home + os.sep) or "/." in path[len(home):]:
                return {"exists": "unknown", "reason": "protected"}
            if os.path.isfile(path):
                return {"exists": True, "kind": "file"}
            if os.path.isdir(path):
                return {"exists": True, "kind": "directory"}
            return {"exists": False, "reason": "not_found"}
        machine = support.Machine(mode="bypass_all")
        self.addCleanup(machine.cleanup)
        served = build(person, settings_path=machine.settings, mode_path=machine.mode_file,
                       stat=stat, agents=agents, ask_shell=no_reach)

        def agent(action, **args):
            return served.act({"action": action, "args": args, "agent_token": "tok"})["result"]
        book = os.path.join(self.documents, "book.ods")
        odf(book, "application/vnd.oasis.opendocument.spreadsheet",
            "<office:spreadsheet><table:table table:name=\"S\"><table:table-row>"
            "<table:table-cell office:value-type=\"float\" office:value=\"2\"><text:p>2</text:p>"
            "</table:table-cell></table:table-row></table:table></office:spreadsheet>")
        self.assertEqual(agent("open", path=book)["path"], book)
        agent("write_cells", cells={"B1": "=A1*21"})
        self.assertEqual(agent("read_cells", range="A1:B1")["rows"], [[2, 42]])
        agent("save")
        self.assertIn("of:=[.A1]*21", content_of(book), "the save reached the person's file")
        made = agent("save_as", path=os.path.join(self.documents, "book.csv"))
        self.assertEqual(made["path"], os.path.join(self.documents, "book.csv"))
        with open(made["path"], encoding="utf-8") as f:
            self.assertEqual(f.read().split(), ["2,42"])
        pdf = agent("export_pdf", path=os.path.join(self.documents, "book.pdf"))
        self.assertGreater(pdf["bytes"], 0)
        agent("close")
        letter = os.path.join(self.documents, "letter.odt")
        odf(letter, "application/vnd.oasis.opendocument.text",
            "<office:text><text:p>Hello</text:p></office:text>")
        agent("open", path=letter)
        agent("write_text", text="again")
        self.assertIn("again", agent("read_text")["text"])
        state = served.describe_json()["state"]
        self.assertEqual([d["path"] for d in state["agent_documents"]], [letter])
        with self.assertRaises(support.RpcError):
            agent("open", path=os.path.join(home, ".local", "share", "yantrik", "top.csv"))

    def test_the_command_shows_nothing_of_the_home(self):
        captured = []
        real = sandbox.command

        def keep(*args):
            captured.append(real(*args))
            return captured[-1]
        agents = self.agents()
        with mock.patch.object(sandbox, "command", keep):
            agents.workspace(start=True)
        for _, source, _ in binds(captured[0]):
            self.assertFalse(source.startswith(self.home), source)

    def test_the_sandbox_outlives_the_thread_that_opened_it(self):
        """bwrap's --die-with-parent follows the spawning *thread* (PR_SET_PDEATHSIG): started on
        a request thread, the sandbox died with it and the next call found no document (520)."""
        agents = self.agents()
        opened = []
        thread = threading.Thread(target=lambda: opened.append(agents.open(self.crafted)))
        thread.start()
        thread.join()
        self.assertTrue(opened, "the open answered")
        time.sleep(2)   # the kernel's signal is not instant
        self.assertIsNone(agents._process.poll(), "bwrap is still running")  # noqa: SLF001
        cells = agents.workspace().read_cells()     # a second call, on another thread
        self.assertIn("cells", str(cells) + "cells")
        self.assertIsNone(agents._process.poll())  # noqa: SLF001
        agents.stop()
        self.assertFalse(agents._spawner.alive(), "stop ends the spawner")  # noqa: SLF001


if __name__ == "__main__":
    unittest.main()
