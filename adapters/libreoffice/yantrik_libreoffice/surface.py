"""LibreOffice's surface: what a mind reads, what it may ask for, and what each action costs.

Built only on the public Python SDK (`yantrik_surface`): the dispatch, the argument checks, the
revision guard, the ceiling, the mode and the grant are all the SDK's, and this file is only
LibreOffice's vocabulary. The work itself is `office.Office`, over UNO.

The grades, and why (README.md has the long form):

  read_text, read_cells        safe       reading changes nothing
  open, close                  standard   a window opens or shuts; no file changes; `close`
                                          refuses a document with unsaved changes
  write_text, write_cells      standard   the document in memory changes, and nothing on disk
                                          does until `save`; LibreOffice's Undo takes it back
  save_as, export_pdf          standard   they only ever create a new file: a path that exists
                                          is refused, which is what keeps them `standard`
  save                         sensitive  it replaces the file the document came from; with
                                          writes `standard`, this is the one step at which a
                                          person decides whether a mind's edits become the file

None of the descriptions says the action cannot be undone, and that is a decision too: the
words would make `auto` mode ask about `save` as well, and a person who chose `auto` has said
that `sensitive` work may run unasked.

Whose LibreOffice a call reaches: the person's calls reach the person's LibreOffice, on the pipe.
An agent's calls (and the Mind's) never do. They reach the agents' own LibreOffice, in the sandbox
of agent_office.py, working on copies of the person's files (workspace.py); where that sandbox
cannot be built, every one is refused with `CLOSED_TO_AGENTS` and the reason (#942, #953).
"""

from typing import Annotated, Literal, Optional

from yantrik_surface import Later, Refusal, Surface, files

from . import formulas
from .agent_office import CLOSED_TO_AGENTS, AgentOffice  # noqa: F401
from .office import NotReachable, Office, OfficeError

APP_ID = "libreoffice"

_KIND = {"writer": "Writer", "calc": "Calc", "impress": "Impress", "draw": "Draw"}

DOCUMENT = ("Which open document: its name as describe lists it, or its path. Left out: the only "
            "one open, or the one in front")


class LibreOfficeSurface(Surface):
    """The SDK's `Surface`, reading LibreOffice once per view.

    `snapshot` is overridden so the summary and the state come from one read of LibreOffice —
    two reads could straddle a change and publish a revision of a view that never existed. The
    default serialization (a lock around the connection's thread) is kept: UNO calls may come
    from any thread, and LibreOffice serializes them itself.
    """

    def __init__(self, office, app_id=APP_ID, agents=None, **kwargs):
        super().__init__(app_id, **kwargs)
        self.office = office
        self.agents = agents

    def snapshot(self):
        try:
            documents, front = self.office.overview()
        except NotReachable as e:
            return ("LibreOffice — %s" % e.short,
                    {"connected": False, "pipe": self.office.pipe, "problem": str(e),
                     "documents": [], "agent_documents": self._agent_documents()})
        except OfficeError as e:
            return ("LibreOffice — did not answer",
                    {"connected": False, "pipe": self.office.pipe, "problem": str(e),
                     "documents": [], "agent_documents": self._agent_documents()})
        return summary_of(documents, front), {
            "connected": True, "pipe": self.office.pipe, "front": front, "documents": documents,
            "agent_documents": self._agent_documents()}

    def _agent_documents(self):
        """The documents open in the agents' own LibreOffice (agent_office.py), when it runs."""
        return self.agents.documents() if self.agents is not None else []


def summary_of(documents, front):
    """One line a person could read."""
    if not documents:
        return "LibreOffice — running, no document open"
    if len(documents) > 3:
        return "LibreOffice — %d documents open%s" % (
            len(documents), "; %s in front" % front if front else "")
    parts = []
    for d in documents:
        notes = [_KIND[d["kind"]]]
        if d["modified"]:
            notes.append("unsaved changes")
        if d["read_only"]:
            notes.append("read-only")
        parts.append("%s (%s)" % (d["name"], ", ".join(notes)))
    in_front = "; %s in front" % front if front and len(documents) > 1 else ""
    return "LibreOffice — %s%s" % ("; ".join(parts), in_front)


def _answer(work, *args, **kwargs):
    """Run an `Office` call; its sentence, if it refuses, is the caller's refusal."""
    try:
        return work(*args, **kwargs)
    except OfficeError as e:
        raise Refusal(str(e)) from None


# Why an agent's call on a document the person opened from a place an agent may not read is
# refused.
HIDDEN = ("the document is a file an agent is not shown, and it is left alone; `open` one in the "
          "person's home")


def agents_files(stat=None, always=False):
    """The rule `Office.admit` is held to (#443): an agent names only files in the person's
    home, outside its protected places, and acts on no document the person opened from
    anywhere else — `read_text` was any file the person can read, graded `safe` (#942's audit).
    `stat` stands in for the shell's `files_stat` in a test. With `always`, the rule's checks of
    a document bind every call, as they do in the agents' own LibreOffice, whose every call is an
    agent's, even work done after the dispatch has answered (`export_pdf`)."""
    def agent():
        return always or files.agent_is_calling()

    def admit(what, path):
        if what == "read":
            files.may_read_file(path, stat)
        elif what == "write":
            files.may_write_file(path, stat)
        elif what == "formula":
            said = formulas.refusal(path) if agent() else None
            if said:
                raise Refusal(said)
        elif what == "outside":
            if agent():
                raise Refusal(path)
        elif what == "links":
            if agent():
                raise Refusal("the document is linked to another file or source (%s), which an "
                              "agent does not read through; ask the person" % ", ".join(path[:3]))
        elif path and agent():
            try:
                files.may_read_file(path, stat)
            except Refusal:
                raise Refusal(HIDDEN) from None
    return admit


def build(office=None, app_id=APP_ID, stat=None, agents=None, **kwargs):
    """LibreOffice's surface over `office` (a fresh `Office` on the default pipe when None), with
    agents' calls served by `agents` (a fresh sandboxed `AgentOffice` when None). `kwargs` go to
    the SDK's `Surface` — tests pin the ceiling and mode files with them, and `stat` stands in
    for the shell's `files_stat`."""
    office = office if office is not None else Office()
    office.admit = agents_files(stat)
    agents = agents if agents is not None else AgentOffice(person=office)
    agents.admit = agents_files(stat, always=True)
    surface = LibreOfficeSurface(office, app_id, agents=agents, **kwargs)

    def serving():
        """What this call is served by: for an agent's, the agents' sandboxed LibreOffice, which
        refuses when no sandbox can be built and is never the person's."""
        return agents.workspace() if files.agent_is_calling() else office

    @surface.action("open", expected_seconds=15, timeout=120)
    def open_document(
            path: Annotated[str, "Absolute path of the file (~ is the home folder): .odt, .docx, "
                                 ".ods, .xlsx, .csv, .odp, .pptx, or anything else LibreOffice "
                                 "opens"],
            read_only: Annotated[bool, "Open it so nothing can change it, `save` included"] = False,
    ) -> dict:
        """Open a file in LibreOffice, in a window of its own; documents already open stay as
        they are, and a file already open is not opened twice. Answers with the name the other
        actions take as `document`."""
        if files.agent_is_calling():
            return _answer(agents.open, path, read_only)
        return _answer(office.open, path, read_only)

    @surface.action("read_text", grade="safe", timeout=60)
    def read_text(
            document: Annotated[Optional[str], DOCUMENT] = None,
            start: Annotated[int, "The first character to read, from 0"] = 0,
            max_chars: Annotated[int, "How many characters at most; `more` says whether there "
                                      "is more after them"] = 20000,
    ) -> dict:
        """Read the text of a Writer document: its body as plain text, one paragraph per line.
        A long document comes in parts: `start` and `max_chars` page through it."""
        return _answer(lambda: serving().read_text(document, start, max_chars))

    @surface.action("read_cells", grade="safe", timeout=60)
    def read_cells(
            range: Annotated[Optional[str], "Like `A1:D20`, or one cell like `B3`. Left out: "  # noqa: A002
                                            "every cell the sheet uses"] = None,
            sheet: Annotated[Optional[str], "The sheet's name, as describe lists it. Left out: "
                                            "the sheet in front"] = None,
            document: Annotated[Optional[str], DOCUMENT] = None,
    ) -> dict:
        """Read cells of a Calc spreadsheet: their values row by row (numbers as numbers, text as
        text, empty cells as null), and in `formulas` the formula behind every computed cell. At
        most 10000 cells at a time."""
        return _answer(lambda: serving().read_cells(range, sheet, document))

    @surface.action("write_text", timeout=60)
    def write_text(
            text: Annotated[str, "What to write; a line break starts a new paragraph"],
            at: Annotated[Literal["end", "start", "replace_all"],
                          "Where: after the last paragraph, before the first, or in place of "
                          "all the text"] = "end",
            document: Annotated[Optional[str], DOCUMENT] = None,
    ) -> dict:
        """Write text into a Writer document. Nothing on disk changes until `save`, and
        LibreOffice's Undo takes it back."""
        return _answer(lambda: serving().write_text(text, at, document))

    @surface.action("write_cells", timeout=60)
    def write_cells(
            cells: Annotated[dict, "Each cell and what goes in it, like {\"A1\": \"Total\", "
                                   "\"B1\": 42, \"C1\": \"=B1*2\"}: a number, text, a formula "
                                   "starting with =, or null to empty it. Text that should start "
                                   "with = starts with ' instead"],
            sheet: Annotated[Optional[str], "The sheet's name, as describe lists it. Left out: "
                                            "the sheet in front"] = None,
            document: Annotated[Optional[str], DOCUMENT] = None,
    ) -> dict:
        """Write cells of a Calc spreadsheet, every one or none: a cell that cannot be written
        leaves the sheet as it was. Nothing on disk changes until `save`, and LibreOffice's Undo
        takes it back."""
        return _answer(lambda: serving().write_cells(cells, sheet, document))

    @surface.action("save", grade="sensitive", expected_seconds=10, timeout=120)
    def save(document: Annotated[Optional[str], DOCUMENT] = None) -> dict:
        """Save a document over the file it was opened from, in that file's format: what the
        file held before is replaced by what the document holds now, and LibreOffice keeps no
        copy of the old one."""
        return _answer(lambda: serving().save(document))

    @surface.action("save_as", expected_seconds=10, timeout=120)
    def save_as(
            path: Annotated[str, "Absolute path of the new file; its extension picks the format "
                                 "(.odt, .docx, .ods, .xlsx, .csv, .odp, .pptx, …)"],
            document: Annotated[Optional[str], DOCUMENT] = None,
    ) -> dict:
        """Save a document to a new file, and from then on the document is that file. Never
        replaces a file: a path where something already is is refused."""
        return _answer(lambda: serving().save_as(path, document))

    @surface.action("export_pdf", expected_seconds=20, timeout=300)
    def export_pdf(
            path: Annotated[str, "Absolute path of the new PDF, ending in .pdf"],
            document: Annotated[Optional[str], DOCUMENT] = None,
    ) -> dict:
        """Write a PDF of a document to a new file, leaving the document as it is. Never replaces
        a file: a path where something already is is refused."""
        # Checked in this turn — the document, the path — and written after it, so a long export
        # holds up nobody reading the surface in the meantime. The answer waits for the file.
        _, work = _answer(lambda: serving().export_pdf_plan(path, document))
        return Later(lambda: _answer(work))

    @surface.action("close", timeout=60)
    def close(document: Annotated[Optional[str], DOCUMENT] = None) -> dict:
        """Close a document's window. Refused while it has unsaved changes: `save` or `save_as`
        it first, because this adapter never throws edits away."""
        return _answer(lambda: serving().close(document))

    return surface
